//! Copies one repository's sessions between machines. `export` writes them as
//! one stream and `import` stores that stream for the local checkout; the
//! format is described in docs/sync-format.md.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::annotations::{self, Annotation, Change, SessionMeta};
use crate::model::SessionRow;
use crate::reports::{self, ActionResponse, Report};

pub const FORMAT: &str = "session-player.sync";
pub const VERSION: u32 = 1;

/// What a repository is called on every machine that has a checkout of it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RepoName(pub String);

/// The stream's first line.
#[derive(Debug, Serialize, Deserialize)]
pub struct Header {
    pub format: String,
    pub version: u32,
    pub repo: RepoName,
    /// The exporting machine's host name.
    pub exporter: String,
    pub exported_at: i64,
}

/// The folders the agents keep sessions in, below the home folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentFolder {
    ClaudeProjects,
    CodexSessions,
    CodexArchived,
    PiSessions,
}

impl AgentFolder {
    const ALL: [AgentFolder; 4] = [AgentFolder::ClaudeProjects, AgentFolder::CodexSessions, AgentFolder::CodexArchived, AgentFolder::PiSessions];

    fn under(self, home: &Path) -> PathBuf {
        home.join(match self {
            AgentFolder::ClaudeProjects => ".claude/projects",
            AgentFolder::CodexSessions => ".codex/sessions",
            AgentFolder::CodexArchived => ".codex/archived_sessions",
            AgentFolder::PiSessions => ".pi/agent/sessions",
        })
    }

    /// Claude Code and pi keep one folder per working directory, named after
    /// its path. Codex files sessions by date.
    fn per_directory(self) -> bool {
        matches!(self, AgentFolder::ClaudeProjects | AgentFolder::PiSessions)
    }
}

/// The session a file holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRef {
    pub source: String,
    pub native_id: String,
    pub last_event_at: Option<i64>,
}

/// One line of the stream after the header.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
pub enum Record {
    /// Followed by `size` bytes: the file as it is on disk. `rel_path` is
    /// relative to the working directory's folder for Claude Code and pi, and
    /// to the agent folder for Codex. `session` is absent for a file an agent
    /// keeps beside a session.
    File { folder: AgentFolder, rel_path: String, size: u64, session: Option<SessionRef> },
    Meta { meta: SessionMeta },
    /// `current` is false for an older version that a report in the stream cites.
    Annotation { annotation: Annotation, current: bool },
    Report { report: Report },
    Response { response: ActionResponse },
    /// Without it the stream was cut short, and nothing is stored in the database.
    End,
}

pub struct ExportRequest {
    pub repo: RepoName,
    /// Only sessions started at or after this, in milliseconds.
    pub since: Option<i64>,
}

pub struct ImportTarget {
    /// The local checkout the sessions are listed under.
    pub project: PathBuf,
    pub home: PathBuf,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub added: usize,
    pub replaced: usize,
    pub kept_local: usize,
    pub unchanged: usize,
}

impl Counts {
    fn add(&mut self, change: Change) {
        match change {
            Change::Added => self.added += 1,
            Change::Replaced => self.replaced += 1,
            Change::KeptLocal => self.kept_local += 1,
            Change::Unchanged => self.unchanged += 1,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PullSummary {
    pub exporter: String,
    pub sessions: Counts,
    pub analyses: Counts,
    pub annotations: Counts,
    pub responses: Counts,
    pub reports_added: usize,
}

pub(crate) fn create_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_projects (
            path TEXT PRIMARY KEY, project TEXT NOT NULL, repo TEXT NOT NULL,
            from_host TEXT NOT NULL, pulled_at INTEGER NOT NULL);",
    )
}

/// The checkout each pulled session file is listed under. A pulled session
/// records the other machine's folder, which may not exist here.
pub(crate) fn pulled_projects(conn: &Connection) -> rusqlite::Result<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT path, project FROM session_projects")?;
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
}

/// The name of the repository checked out at `project`: its origin URL without
/// scheme, user, port or ".git", so the SSH and HTTPS forms give one name.
/// Without an origin another machine could share, it is the folder's name.
/// Lowercase, because hosts such as GitHub ignore case in a repository's path.
pub fn repo_name(project: &Path) -> Option<RepoName> {
    let config = std::fs::read_to_string(project.join(".git/config")).unwrap_or_default();
    let mut in_origin = false;
    let mut from_origin = None;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            in_origin = line.split_whitespace().collect::<String>() == "[remote\"origin\"]";
        } else if in_origin
            && let Some((key, url)) = line.split_once('=')
            && key.trim() == "url"
        {
            from_origin = shared_name(url.trim());
        }
    }
    let name = from_origin.or_else(|| project.file_name().map(|n| n.to_string_lossy().into_owned()))?;
    Some(RepoName(name.to_lowercase()))
}

/// "host/path" for a URL on another host; None for a path on this machine.
fn shared_name(url: &str) -> Option<String> {
    let rest = match url.split_once("://") {
        Some(("file", _)) => return None,
        Some((_, rest)) => rest.to_string(),
        // The scp form, git@github.com:owner/repo.git. A colon after a slash is part of a path.
        None => match url.split_once(':') {
            Some((host, path)) if !host.contains('/') => format!("{host}/{path}"),
            _ => return None,
        },
    };
    let (host, path) = rest.split_once('/')?;
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split_once(':').map_or(host, |(h, _)| h);
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    (!host.is_empty() && !path.is_empty()).then(|| format!("{host}/{path}"))
}

/// The agent folder a session file is in and its path within it.
fn located(home: &Path, path: &Path) -> Option<(AgentFolder, PathBuf)> {
    AgentFolder::ALL.iter().find_map(|folder| {
        let rel = path.strip_prefix(folder.under(home)).ok()?;
        let rel = if folder.per_directory() { rel.components().skip(1).collect() } else { rel.to_path_buf() };
        Some((*folder, rel))
    })
}

fn write_record(out: &mut dyn Write, record: &Record) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *out, record)?;
    out.write_all(b"\n")?;
    Ok(())
}

fn host_name() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "another machine".into())
}

/// Writes a file's record and contents. Returns false, writing nothing, for a
/// file outside the agents' folders.
fn send_file(out: &mut dyn Write, home: &Path, path: &Path, session: Option<SessionRef>) -> anyhow::Result<bool> {
    let Some((folder, rel)) = located(home, path) else { return Ok(false) };
    let mut file = std::fs::File::open(path).with_context(|| format!("Couldn't read {}", path.display()))?;
    // The agent may be writing to the file; only what was there when it was measured is sent.
    let size = file.metadata()?.len();
    write_record(out, &Record::File { folder, rel_path: rel.to_string_lossy().into_owned(), size, session })?;
    if std::io::copy(&mut (&mut file).take(size), out)? != size {
        bail!("{} changed while it was being read", path.display());
    }
    Ok(true)
}

/// Writes the repository's sessions to `out`.
pub fn export(conn: &Connection, home: &Path, req: &ExportRequest, out: &mut dyn Write) -> anyhow::Result<()> {
    let mut out = zstd::stream::write::Encoder::new(out, 3)?.auto_finish();
    let header = Header { format: FORMAT.into(), version: VERSION, repo: req.repo.clone(), exporter: host_name(), exported_at: chrono::Utc::now().timestamp_millis() };
    serde_json::to_writer(&mut out, &header)?;
    out.write_all(b"\n")?;

    let rows = crate::index::list(conn)?;
    let mut names: HashMap<String, Option<RepoName>> = HashMap::new();
    let mut in_repo = |r: &SessionRow| names.entry(r.project.clone()).or_insert_with(|| repo_name(Path::new(&r.project))).as_ref() == Some(&req.repo);
    // A session imported from a bundle is a copy someone shared, not one of this machine's sessions.
    let own: Vec<&SessionRow> = rows.iter().filter(|r| !r.from_bundle && in_repo(r)).collect();
    let mut sessions: Vec<&SessionRow> = own
        .iter()
        .copied()
        .filter(|r| !r.is_subagent && req.since.is_none_or(|since| r.started_at.or(r.updated_at).is_some_and(|t| t >= since)))
        .collect();
    // A subagent goes with the session that started it, whatever its own dates say.
    let ids: HashSet<&str> = sessions.iter().map(|r| r.native_id.as_str()).collect();
    let folder_of = |r: &SessionRow| Path::new(&r.path).with_extension("");
    let sent_folders: Vec<PathBuf> = sessions.iter().map(|r| folder_of(r)).collect();
    let every_folder: Vec<PathBuf> = own.iter().filter(|r| !r.is_subagent).map(|r| folder_of(r)).collect();
    let subagents: Vec<&SessionRow> = own
        .iter()
        .copied()
        .filter(|r| r.is_subagent)
        .filter(|r| {
            let path = Path::new(&r.path);
            // Claude Code and pi keep a subagent's file in its session's folder, which says whose
            // it is even when two sessions share an id. Codex only records the parent's id.
            if every_folder.iter().any(|f| path.starts_with(f)) {
                sent_folders.iter().any(|f| path.starts_with(f))
            } else {
                r.parent_id.as_deref().is_some_and(|parent| ids.contains(parent))
            }
        })
        .collect();
    sessions.extend(subagents);
    let mut sent: HashSet<PathBuf> = HashSet::new();
    let mut keys: Vec<String> = Vec::new();
    for row in &sessions {
        let path = Path::new(&row.path);
        let session = SessionRef { source: row.source.clone(), native_id: row.native_id.clone(), last_event_at: row.updated_at };
        if sent.insert(path.to_path_buf()) && send_file(&mut out, home, path, Some(session))? {
            keys.push(annotations::session_key(row));
        }
    }
    // Claude Code and pi keep a session's subagents and large tool results in a folder
    // named after it. Resuming the session needs them.
    for row in &sessions {
        let path = Path::new(&row.path);
        let beside = path.with_extension("");
        if !located(home, path).is_some_and(|(folder, _)| folder.per_directory()) || !beside.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&beside).sort_by_file_name().into_iter().filter_map(Result::ok) {
            // A session file found here that wasn't sent above belongs to no session in the index; it stays behind.
            if entry.file_type().is_file() && !crate::parse::is_session_path(entry.path()) && sent.insert(entry.path().to_path_buf()) {
                send_file(&mut out, home, entry.path(), None)?;
            }
        }
    }
    for key in &keys {
        if let Some(meta) = annotations::meta(conn, key)? {
            write_record(&mut out, &Record::Meta { meta })?;
        }
        for annotation in annotations::annotations(conn, key)? {
            write_record(&mut out, &Record::Annotation { annotation, current: true })?;
        }
    }
    // A report travels when it covers any session sent, with the versions it cites.
    for report in reports::saved_reports(conn)? {
        if !report.sessions.iter().any(|s| keys.contains(&s.key)) {
            continue;
        }
        for cited in reports::cited_annotations(conn, &report)? {
            write_record(&mut out, &Record::Annotation { annotation: cited.annotation, current: false })?;
        }
        let responses = reports::action_responses(conn, &report.id)?;
        write_record(&mut out, &Record::Report { report })?;
        for response in responses {
            write_record(&mut out, &Record::Response { response })?;
        }
    }
    write_record(&mut out, &Record::End)?;
    Ok(())
}

/// Claude Code names a working directory's folder after its path, with every
/// character that isn't a letter or digit replaced by "-".
fn claude_folder(project: &Path) -> String {
    project.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// pi wraps the path in "--" and replaces its separators with "-".
fn pi_folder(project: &Path) -> String {
    let path = project.to_string_lossy();
    format!("--{}--", path.trim_start_matches(['/', '\\']).replace(['/', '\\', ':'], "-"))
}

/// How the copy of a file here relates to the copy that arrived.
#[derive(Debug, PartialEq)]
enum Copies {
    Same,
    /// The arriving copy is the one here plus more: the session went on over there.
    IncomingContinues,
    LocalContinues,
    /// Each has something the other lacks.
    Diverged,
}

fn compare(local: &Path, incoming: &Path) -> std::io::Result<Copies> {
    let mut a = BufReader::new(std::fs::File::open(local)?);
    let mut b = BufReader::new(std::fs::File::open(incoming)?);
    loop {
        let (x, y) = (a.fill_buf()?, b.fill_buf()?);
        let n = x.len().min(y.len());
        if n == 0 {
            return Ok(match (x.is_empty(), y.is_empty()) {
                (true, true) => Copies::Same,
                (true, false) => Copies::IncomingContinues,
                _ => Copies::LocalContinues,
            });
        }
        if x[..n] != y[..n] {
            return Ok(Copies::Diverged);
        }
        a.consume(n);
        b.consume(n);
    }
}

/// For a session continued on both machines: whether the arriving copy's last
/// event is the later one. Without timestamps to compare, the larger copy counts as later.
fn incoming_is_later(local: &Path, incoming: &Path, session: &SessionRef) -> std::io::Result<bool> {
    let ours = crate::parse::detect(local)
        .and_then(|source| crate::parse::parse_file(local, source).ok())
        .and_then(|parsed| parsed.events.iter().filter_map(|e| e.ts).max());
    Ok(match (session.last_event_at, ours) {
        (Some(theirs), Some(ours)) if theirs != ours => theirs > ours,
        _ => std::fs::metadata(incoming)?.len() > std::fs::metadata(local)?.len(),
    })
}

/// A model's annotation nobody has edited: the kind that analyzing again replaces.
fn is_suggestion(a: &Annotation) -> bool {
    matches!(a.author, annotations::Author::Model { .. }) && !a.edited
}

/// When a session was last analyzed, going by its metadata and annotations.
fn analyzed_at<'a>(meta: Option<&SessionMeta>, annotations: impl Iterator<Item = &'a Annotation>) -> Option<i64> {
    let from_meta = meta.and_then(|m| m.generated_by.as_ref()).map(|g| g.at);
    let from_annotations = annotations.filter(|a| is_suggestion(a)).map(|a| a.created_at).max();
    from_meta.max(from_annotations)
}

/// Reads what `export` wrote and stores it for the checkout at `target.project`.
pub fn import(conn: &Connection, target: &ImportTarget, input: &mut dyn Read) -> anyhow::Result<PullSummary> {
    const NOT_AN_EXPORT: &str = "That isn't a Session Player export";
    const ENDED_EARLY: &str = "The export ended early. Session files already copied are kept; pull again to finish.";
    let mut input = BufReader::new(zstd::stream::read::Decoder::new(input).context(NOT_AN_EXPORT)?);
    let mut line = String::new();
    input.read_line(&mut line).context(NOT_AN_EXPORT)?;
    let header: Header = serde_json::from_str(&line).ok().filter(|h: &Header| h.format == FORMAT).context(NOT_AN_EXPORT)?;
    if header.version > VERSION {
        bail!("The other machine's sp is newer than this one and writes a format this one can't read. Update sp here, then pull again.");
    }
    let ours = repo_name(&target.project);
    if ours.as_ref() != Some(&header.repo) {
        bail!("The export is of {}, but {} is a checkout of {}.", header.repo.0, target.project.display(), ours.map(|r| r.0).unwrap_or_default());
    }
    let mut summary = PullSummary { exporter: header.exporter.clone(), ..Default::default() };
    let now = chrono::Utc::now().timestamp_millis();
    let mut kept_local: Vec<PathBuf> = Vec::new();
    let mut metas: Vec<SessionMeta> = Vec::new();
    let mut current: Vec<Annotation> = Vec::new();
    let mut cited: Vec<Annotation> = Vec::new();
    let mut reports_in: Vec<Report> = Vec::new();
    let mut responses: Vec<ActionResponse> = Vec::new();
    let mut ended = false;
    while !ended {
        line.clear();
        if input.read_line(&mut line).context(ENDED_EARLY)? == 0 {
            break;
        }
        match serde_json::from_str::<Record>(&line).context(ENDED_EARLY)? {
            Record::File { folder, rel_path, size, session } => {
                let rel = Path::new(&rel_path);
                if rel_path.is_empty() || rel.components().any(|c| !matches!(c, Component::Normal(_))) {
                    bail!("The export names a file outside the session folders: {rel_path}");
                }
                let root = folder.under(&target.home);
                let dest = match folder {
                    AgentFolder::ClaudeProjects => root.join(claude_folder(&target.project)).join(rel),
                    AgentFolder::PiSessions => root.join(pi_folder(&target.project)).join(rel),
                    AgentFolder::CodexSessions | AgentFolder::CodexArchived => root.join(rel),
                };
                std::fs::create_dir_all(dest.parent().context("The export names a folder, not a file")?)?;
                // Written beside the destination and renamed, so a pull that stops early leaves no partial session.
                let mut incoming = dest.clone().into_os_string();
                incoming.push(".sp-pull");
                let incoming = PathBuf::from(incoming);
                let copied = std::io::copy(&mut (&mut input).take(size), &mut std::fs::File::create(&incoming)?)?;
                if copied != size {
                    let _ = std::fs::remove_file(&incoming);
                    bail!(ENDED_EARLY);
                }
                let change = if !dest.exists() {
                    Change::Added
                } else {
                    match compare(&dest, &incoming)? {
                        Copies::Same => Change::Unchanged,
                        Copies::LocalContinues => Change::KeptLocal,
                        Copies::IncomingContinues => Change::Replaced,
                        Copies::Diverged => match &session {
                            Some(session) if !incoming_is_later(&dest, &incoming, session)? => Change::KeptLocal,
                            // A file beside a session goes the way the session's own file went.
                            None if kept_local.iter().any(|folder| dest.starts_with(folder)) => Change::KeptLocal,
                            _ => Change::Replaced,
                        },
                    }
                };
                if session.is_some() && change == Change::KeptLocal {
                    kept_local.push(dest.with_extension(""));
                }
                if matches!(change, Change::Added | Change::Replaced) {
                    std::fs::rename(&incoming, &dest)?;
                } else {
                    std::fs::remove_file(&incoming)?;
                }
                crate::index::add_root(conn, &root.to_string_lossy())?;
                if session.is_some() {
                    summary.sessions.add(change);
                }
                if session.is_some() && matches!(change, Change::Added | Change::Replaced) {
                    conn.execute(
                        "INSERT OR REPLACE INTO session_projects(path, project, repo, from_host, pulled_at) VALUES(?1, ?2, ?3, ?4, ?5)",
                        params![dest.to_string_lossy(), target.project.to_string_lossy(), header.repo.0, header.exporter, now],
                    )?;
                }
            }
            Record::Meta { meta } => metas.push(meta),
            Record::Annotation { annotation, current: true } => current.push(annotation),
            Record::Annotation { annotation, current: false } => cited.push(annotation),
            Record::Report { report } => reports_in.push(report),
            Record::Response { response } => responses.push(response),
            Record::End => ended = true,
        }
    }
    // Nothing goes into the database from a stream that stopped part-way, so a
    // report never arrives without the annotations it cites.
    if !ended {
        bail!(ENDED_EARLY);
    }
    // Reports first: a report's citations are what keep an older annotation version stored.
    for report in &reports_in {
        if reports::put_report(conn, report)? {
            summary.reports_added += 1;
        }
    }
    for annotation in &cited {
        annotations::put_version(conn, annotation, false)?;
    }
    for response in &responses {
        summary.responses.add(reports::put_action_response(conn, response)?);
    }
    let mut keys: Vec<&str> = metas.iter().map(|m| m.key.as_str()).chain(current.iter().map(|a| a.key.as_str())).collect();
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        let meta = metas.iter().find(|m| m.key == key);
        let theirs: Vec<&Annotation> = current.iter().filter(|a| a.key == key).collect();
        let ours = annotations::annotations(conn, key)?;
        let their_analysis = analyzed_at(meta, theirs.iter().copied());
        let our_analysis = analyzed_at(annotations::meta(conn, key)?.as_ref(), ours.iter());
        if let Some(meta) = meta {
            summary.analyses.add(annotations::put_meta(conn, meta)?);
        }
        // Analyzing a session again replaces the model's annotations with new ones under
        // new ids, so the two machines' sets are alternatives. Merging them by id would
        // show both analyses at once.
        if their_analysis > our_analysis {
            for stale in ours.iter().filter(|a| is_suggestion(a) && !theirs.iter().any(|t| t.id == a.id)) {
                annotations::delete_annotation(conn, &stale.id)?;
            }
        }
        for annotation in theirs {
            let stale = our_analysis > their_analysis && is_suggestion(annotation) && !ours.iter().any(|o| o.id == annotation.id);
            summary.annotations.add(if stale { Change::KeptLocal } else { annotations::put_version(conn, annotation, true)? });
        }
    }
    Ok(summary)
}
