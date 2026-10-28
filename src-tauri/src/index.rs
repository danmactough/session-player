use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rayon::prelude::*;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use walkdir::WalkDir;

use crate::model::{EventKind, Parsed, SessionRow, Source};
use crate::parse;

/// Bump when parsing changes so existing indexes are rebuilt on launch.
const INDEX_VERSION: &str = "12";
const FTS_BODY_LIMIT: usize = 400_000;
const SKIP_DIRS: &[&str] = &["memory", "tool-results", "subagent-artifacts", "node_modules", ".git"];

pub fn open(db_path: &Path) -> anyhow::Result<Connection> {
    let conn = Connection::open(db_path)?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS roots (path TEXT PRIMARY KEY, added_at INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS files (
            path TEXT PRIMARY KEY, root TEXT NOT NULL, mtime INTEGER NOT NULL,
            size INTEGER NOT NULL, source TEXT, error TEXT);
         CREATE TABLE IF NOT EXISTS sessions (
            path TEXT PRIMARY KEY, root TEXT NOT NULL, source TEXT NOT NULL,
            native_id TEXT NOT NULL, cwd TEXT, project TEXT NOT NULL,
            title TEXT NOT NULL, title_source TEXT NOT NULL, first_prompt TEXT,
            started_at INTEGER, updated_at INTEGER,
            user_turns INTEGER NOT NULL, assistant_msgs INTEGER NOT NULL,
            tool_calls INTEGER NOT NULL, input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL,
            models TEXT NOT NULL, git_branch TEXT, is_subagent INTEGER NOT NULL,
            parent_id TEXT, size INTEGER NOT NULL, imported_from TEXT,
            branches TEXT NOT NULL DEFAULT '[]', fork_entry TEXT, fork_turn INTEGER,
            error_count INTEGER NOT NULL DEFAULT 0);
         CREATE INDEX IF NOT EXISTS sessions_updated ON sessions(updated_at);
         CREATE VIRTUAL TABLE IF NOT EXISTS session_fts USING fts5(
            path UNINDEXED, title, body, tokenize='porter unicode61');",
    )?;
    let version: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key='index_version'", [], |r| r.get(0))
        .optional()?;
    if version.as_deref() != Some(INDEX_VERSION) {
        // Dropping the table lets schema changes take effect: reopening
        // recreates it with the current columns.
        conn.execute_batch(
            "DELETE FROM files; DROP TABLE sessions; DELETE FROM session_fts;",
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO settings(key, value) VALUES('index_version', ?1)",
            [INDEX_VERSION],
        )?;
        drop(conn);
        return open(db_path);
    }
    Ok(conn)
}

pub fn default_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else { return vec![] };
    [
        ".claude/projects",
        ".codex/sessions",
        ".codex/archived_sessions",
        ".pi/agent/sessions",
    ]
    .iter()
    .map(|p| home.join(p))
    .filter(|p| p.is_dir())
    .collect()
}

/// Adds the default agent directories the first time the app runs.
pub fn seed_roots(conn: &Connection) -> anyhow::Result<()> {
    let seeded: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key='seeded'", [], |r| r.get(0))
        .optional()?;
    if seeded.is_some() {
        return Ok(());
    }
    for root in default_roots() {
        add_root(conn, &root.to_string_lossy())?;
    }
    conn.execute("INSERT INTO settings(key, value) VALUES('seeded', '1')", [])?;
    Ok(())
}

pub fn roots(conn: &Connection) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT path FROM roots ORDER BY added_at")?;
    let rows = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
    Ok(rows)
}

pub fn add_root(conn: &Connection, path: &str) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    conn.execute("INSERT OR IGNORE INTO roots(path, added_at) VALUES(?1, ?2)", params![path, now])?;
    Ok(())
}

pub fn remove_root(conn: &mut Connection, path: &str) -> anyhow::Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM roots WHERE path=?1", [path])?;
    tx.execute(
        "DELETE FROM session_fts WHERE path IN (SELECT path FROM sessions WHERE root=?1)",
        [path],
    )?;
    tx.execute("DELETE FROM sessions WHERE root=?1", [path])?;
    tx.execute("DELETE FROM files WHERE root=?1", [path])?;
    tx.commit()?;
    Ok(())
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct IndexStats {
    pub scanned: usize,
    pub updated: usize,
    pub removed: usize,
    pub failed: usize,
    pub elapsed_ms: u128,
}

struct FileInfo {
    path: String,
    root: String,
    mtime: i64,
    size: i64,
}

struct Indexed {
    file: FileInfo,
    source: Option<Source>,
    row: Option<(SessionRow, String)>,
    error: Option<String>,
}

pub fn run(
    conn: &mut Connection,
    full: bool,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> anyhow::Result<IndexStats> {
    let started = std::time::Instant::now();
    if full {
        conn.execute_batch("DELETE FROM files; DELETE FROM sessions; DELETE FROM session_fts;")?;
    }
    let roots = roots(conn)?;
    let mut files: Vec<FileInfo> = Vec::new();
    for root in &roots {
        let walker = WalkDir::new(root).follow_links(true).into_iter().filter_entry(|e| {
            !(e.file_type().is_dir()
                && e.file_name().to_str().is_some_and(|n| SKIP_DIRS.contains(&n)))
        });
        for entry in walker.filter_map(Result::ok) {
            if !entry.file_type().is_file()
                || entry.path().extension().and_then(|e| e.to_str()) != Some("jsonl")
            {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            files.push(FileInfo {
                path: entry.path().to_string_lossy().into_owned(),
                root: root.clone(),
                mtime,
                size: meta.len() as i64,
            });
        }
    }

    let known: HashMap<String, (i64, i64)> = {
        let mut stmt = conn.prepare("SELECT path, mtime, size FROM files")?;
        stmt.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
            .collect::<Result<_, _>>()?
    };
    let seen: HashSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let removed: Vec<String> = known.keys().filter(|p| !seen.contains(p.as_str())).cloned().collect();
    let scanned = files.len();
    let changed: Vec<FileInfo> = files
        .into_iter()
        .filter(|f| known.get(&f.path) != Some(&(f.mtime, f.size)))
        .collect();

    let total = changed.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let sidecars = CodexSidecars::default();
    progress(0, total);
    let results: Vec<Indexed> = changed
        .into_par_iter()
        .map(|file| {
            let out = index_file(file, &sidecars);
            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if n % 10 == 0 || n == total {
                progress(n, total);
            }
            out
        })
        .collect();

    let mut stats = IndexStats { scanned, removed: removed.len(), ..Default::default() };
    let tx = conn.transaction()?;
    for path in &removed {
        tx.execute("DELETE FROM files WHERE path=?1", [path])?;
        tx.execute("DELETE FROM sessions WHERE path=?1", [path])?;
        tx.execute("DELETE FROM session_fts WHERE path=?1", [path])?;
    }
    for r in results {
        tx.execute(
            "INSERT OR REPLACE INTO files(path, root, mtime, size, source, error) VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                r.file.path,
                r.file.root,
                r.file.mtime,
                r.file.size,
                r.source.map(|s| s.as_str()),
                r.error
            ],
        )?;
        tx.execute("DELETE FROM session_fts WHERE path=?1", [&r.file.path])?;
        tx.execute("DELETE FROM sessions WHERE path=?1", [&r.file.path])?;
        if r.error.is_some() {
            stats.failed += 1;
        }
        let Some((row, body)) = r.row else { continue };
        stats.updated += 1;
        tx.execute(
            "INSERT INTO sessions(path, root, source, native_id, cwd, project, title, title_source,
                first_prompt, started_at, updated_at, user_turns, assistant_msgs, tool_calls,
                input_tokens, output_tokens, cache_read_tokens, models, git_branch, is_subagent,
                parent_id, size, imported_from, branches, fork_entry, fork_turn, error_count)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27)",
            params![
                row.path,
                r.file.root,
                row.source,
                row.native_id,
                row.cwd,
                row.project,
                row.title,
                row.title_source,
                row.first_prompt,
                row.started_at,
                row.updated_at,
                row.user_turns,
                row.assistant_msgs,
                row.tool_calls,
                row.input_tokens,
                row.output_tokens,
                row.cache_read_tokens,
                serde_json::to_string(&row.models)?,
                row.git_branch,
                row.is_subagent,
                row.parent_id,
                row.size,
                row.imported_from,
                serde_json::to_string(&row.branches)?,
                row.fork_entry,
                row.fork_turn,
                row.error_count,
            ],
        )?;
        tx.execute(
            "INSERT INTO session_fts(path, title, body) VALUES(?1, ?2, ?3)",
            params![row.path, row.title, body],
        )?;
    }
    tx.commit()?;

    // Codex keeps thread names in a separate file that changes without touching
    // the rollout, so re-apply them on every pass.
    apply_codex_titles(conn)?;

    stats.elapsed_ms = started.elapsed().as_millis();
    Ok(stats)
}

#[doc(hidden)]
pub fn index_parsed(path: &str, source: Source, mut parsed: Parsed) -> Option<(SessionRow, String)> {
    enrich(Path::new(path), source, &mut parsed, &CodexSidecars::default());
    let file = FileInfo { path: path.to_string(), root: String::new(), mtime: 0, size: 0 };
    build_row(&file, parsed)
}

/// Applies what lives outside the session file: Codex thread names and import
/// records, then title cleanup.
fn enrich(path: &Path, source: Source, parsed: &mut Parsed, sidecars: &CodexSidecars) {
    if source == Source::Codex {
        if let Some(t) = sidecars.title(path, &parsed.native_id) {
            parsed.title = Some((t, "thread"));
        }
        parsed.imported_from = sidecars.imported_from(path, &parsed.native_id);
    }
    parsed.title = parsed
        .title
        .take()
        .and_then(|(t, s)| clean_title(&t, parsed.git_branch.as_deref()).map(|t| (t, s)));
}

fn index_file(file: FileInfo, sidecars: &CodexSidecars) -> Indexed {
    let path = PathBuf::from(&file.path);
    let Some(source) = parse::detect(&path) else {
        return Indexed { file, source: None, row: None, error: None };
    };
    match parse::parse_file(&path, source) {
        Ok(mut parsed) => {
            enrich(&path, source, &mut parsed, sidecars);
            let row = build_row(&file, parsed);
            Indexed { file, source: Some(source), row, error: None }
        }
        Err(e) => Indexed { file, source: Some(source), row: None, error: Some(e.to_string()) },
    }
}

/// Collapses a parsed session into its index row plus the text used for search.
/// Sessions with no conversation at all (e.g. a file that only holds a title
/// record) are dropped.
fn build_row(file: &FileInfo, p: Parsed) -> Option<(SessionRow, String)> {
    let mut user_turns = 0;
    let mut assistant_msgs = 0;
    let mut tool_calls = 0;
    let mut error_count = 0;
    let mut first_prompt: Option<String> = None;
    let mut label_prompt: Option<String> = None;
    let mut prompts_seen = 0;
    let mut first_command: Option<String> = None;
    let mut last_ts: Option<i64> = None;
    let mut first_ts: Option<i64> = None;
    let mut body = String::new();
    for e in &p.events {
        if let Some(ts) = e.ts {
            last_ts = Some(last_ts.map_or(ts, |l: i64| l.max(ts)));
            first_ts = Some(first_ts.map_or(ts, |f: i64| f.min(ts)));
        }
        if e.is_error {
            error_count += 1;
        }
        match e.kind {
            EventKind::User if !e.sidechain => {
                user_turns += 1;
                if !e.text.is_empty() {
                    prompts_seen += 1;
                    if first_prompt.is_none() {
                        first_prompt = Some(e.text.chars().take(2000).collect());
                    }
                    // A one- or two-word opener ("add", "hooks") makes a poor label;
                    // look a couple of prompts further for something descriptive.
                    if label_prompt.is_none()
                        && prompts_seen <= 3
                        && e.text.split_whitespace().count() >= 3
                    {
                        label_prompt = Some(e.text.chars().take(2000).collect());
                    }
                }
            }
            EventKind::Command if first_command.is_none() => first_command = Some(e.text.clone()),
            EventKind::Assistant if !e.sidechain => assistant_msgs += 1,
            EventKind::ToolCall => tool_calls += 1,
            _ => {}
        }
        if body.len() < FTS_BODY_LIMIT {
            match e.kind {
                EventKind::User | EventKind::Assistant | EventKind::Command => {
                    body.push_str(&e.text);
                    body.push('\n');
                }
                EventKind::ToolCall => {
                    body.push_str(e.tool_name.as_deref().unwrap_or(""));
                    body.push(' ');
                    body.extend(e.text.chars().take(300));
                    body.push('\n');
                }
                _ => {}
            }
        }
    }
    if user_turns == 0 && assistant_msgs == 0 && tool_calls == 0 {
        return None;
    }

    let fallback_label = label_prompt
        .as_deref()
        .or(first_prompt.as_deref())
        .and_then(parse::label_from_prompt)
        .map(|l| (l, "prompt"))
        .or_else(|| first_command.map(|c| (c, "command")));
    let (title, title_source) = match (p.title, fallback_label) {
        (Some((t, s)), _) => (t, s),
        (None, Some((t, s))) => (t, s),
        (None, None) => ("Untitled session".to_string(), "none"),
    };
    let project = project_of(p.cwd.as_deref());
    let row = SessionRow {
        path: file.path.clone(),
        source: p.source.as_str().into(),
        native_id: p.native_id,
        cwd: p.cwd,
        project,
        title,
        title_source: title_source.into(),
        first_prompt,
        started_at: p.started_at.or(first_ts),
        updated_at: last_ts.or(Some(file.mtime)),
        user_turns,
        assistant_msgs,
        tool_calls,
        input_tokens: p.tokens.input,
        output_tokens: p.tokens.output,
        cache_read_tokens: p.tokens.cache_read,
        models: p.models,
        git_branch: p.git_branch,
        is_subagent: p.is_subagent,
        parent_id: p.parent_id,
        size: file.size,
        imported_from: p.imported_from,
        branches: p.branches,
        fork_entry: p.fork_entry,
        fork_turn: p.fork_turn,
        error_count,
    };
    Some((row, body))
}

/// The directory a session is grouped under. Claude Code worktrees live inside
/// the repo they were cut from, so they fold back into it.
/// Rejects titles that carry no information (a bare branch name) and unwraps
/// slash-command markup into "/command args".
fn clean_title(t: &str, branch: Option<&str>) -> Option<String> {
    let t = if t.contains("<command-name>") {
        parse::claude::command_label(t)
    } else {
        t.trim().to_string()
    };
    if t.is_empty() || t.starts_with('<') {
        return None;
    }
    if !t.contains(char::is_whitespace)
        && (Some(t.as_str()) == branch
            || matches!(t.as_str(), "main" | "master" | "develop" | "trunk")
            || t.contains('-')
            || t.contains('/'))
    {
        return None;
    }
    Some(t)
}

fn project_of(cwd: Option<&str>) -> String {
    let Some(cwd) = cwd.filter(|c| !c.is_empty()) else {
        return "(unknown)".into();
    };
    for marker in ["/.claude/worktrees/", "/.worktrees/"] {
        if let Some(i) = cwd.find(marker) {
            return cwd[..i].to_string();
        }
    }
    cwd.trim_end_matches('/').to_string()
}

type SidecarCache = std::sync::Mutex<HashMap<PathBuf, std::sync::Arc<HashMap<String, String>>>>;

/// Files Codex keeps next to its sessions folder, read once per index pass.
#[derive(Default)]
struct CodexSidecars {
    titles: SidecarCache,
    imports: SidecarCache,
}

impl CodexSidecars {
    fn title(&self, rollout: &Path, id: &str) -> Option<String> {
        lookup_sidecar(&self.titles, rollout, "session_index.jsonl", read_codex_index, id)
    }

    fn imported_from(&self, rollout: &Path, id: &str) -> Option<String> {
        lookup_sidecar(&self.imports, rollout, "external_agent_session_imports.json", read_codex_imports, id)
    }
}

fn lookup_sidecar(
    cache: &SidecarCache,
    rollout: &Path,
    file_name: &str,
    read: fn(&Path) -> HashMap<String, String>,
    id: &str,
) -> Option<String> {
    let file = rollout
        .ancestors()
        .skip(1)
        .take(6)
        .map(|a| a.join(file_name))
        .find(|p| p.is_file())?;
    let map = {
        let mut cache = cache.lock().unwrap();
        cache.entry(file.clone()).or_insert_with(|| std::sync::Arc::new(read(&file))).clone()
    };
    map.get(id).cloned()
}

/// Maps imported Codex thread ids to the agent the conversation came from. The
/// original files are often deleted after import, so the source path is the
/// only record of where a thread started.
fn read_codex_imports(path: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(text) = std::fs::read_to_string(path) else { return map };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return map };
    for r in v.get("records").and_then(|r| r.as_array()).into_iter().flatten() {
        let (Some(id), Some(src)) = (
            r.get("imported_thread_id").and_then(|x| x.as_str()),
            r.get("source_path").and_then(|x| x.as_str()),
        ) else {
            continue;
        };
        let agent = if src.contains("/.claude/") {
            "Claude Code"
        } else if src.contains("/.pi/") {
            "pi"
        } else {
            "another agent"
        };
        map.insert(id.to_string(), agent.to_string());
    }
    map
}

fn read_codex_index(path: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let _ = parse::for_each_line(path, |line| {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line)
            && let (Some(id), Some(name)) = (
                v.get("id").and_then(|x| x.as_str()),
                v.get("thread_name").and_then(|x| x.as_str()),
            )
            && !name.trim().is_empty()
        {
            map.insert(id.to_string(), name.to_string());
        }
    });
    map
}

fn apply_codex_titles(conn: &Connection) -> anyhow::Result<()> {
    let rows: Vec<(String, String, Option<String>)> = {
        let mut stmt = conn.prepare("SELECT path, native_id, git_branch FROM sessions WHERE source='codex'")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?
    };
    let sidecars = CodexSidecars::default();
    for (path, id, branch) in rows {
        if let Some(t) = sidecars
            .title(Path::new(&path), &id)
            .and_then(|t| clean_title(&t, branch.as_deref()))
        {
            conn.execute(
                "UPDATE sessions SET title=?1, title_source='thread' WHERE path=?2 AND title<>?1",
                params![t, path],
            )?;
        }
    }
    Ok(())
}

pub fn list(conn: &Connection) -> anyhow::Result<Vec<SessionRow>> {
    let mut stmt = conn.prepare(
        "SELECT path, source, native_id, cwd, project, title, title_source, first_prompt,
                started_at, updated_at, user_turns, assistant_msgs, tool_calls, input_tokens,
                output_tokens, cache_read_tokens, models, git_branch, is_subagent, parent_id, size,
                imported_from, branches, fork_entry, fork_turn, error_count
         FROM sessions ORDER BY updated_at DESC",
    )?;
    let mut rows: Vec<SessionRow> = stmt
        .query_map([], |r| {
            let models: String = r.get(16)?;
            Ok(SessionRow {
                path: r.get(0)?,
                source: r.get(1)?,
                native_id: r.get(2)?,
                cwd: r.get(3)?,
                project: r.get(4)?,
                title: r.get(5)?,
                title_source: r.get(6)?,
                first_prompt: r.get(7)?,
                started_at: r.get(8)?,
                updated_at: r.get(9)?,
                user_turns: r.get(10)?,
                assistant_msgs: r.get(11)?,
                tool_calls: r.get(12)?,
                input_tokens: r.get(13)?,
                output_tokens: r.get(14)?,
                cache_read_tokens: r.get(15)?,
                models: serde_json::from_str(&models).unwrap_or_default(),
                git_branch: r.get(17)?,
                is_subagent: r.get(18)?,
                parent_id: r.get(19)?,
                size: r.get(20)?,
                imported_from: r.get(21)?,
                branches: serde_json::from_str(&r.get::<_, String>(22)?).unwrap_or_default(),
                fork_entry: r.get(23)?,
                fork_turn: r.get(24)?,
                error_count: r.get(25)?,
            })
        })?
        .collect::<Result<_, _>>()?;

    // Codex worktrees live under ~/.codex/worktrees/<hash>/<repo-name>. Fold them
    // into the project with the same folder name when exactly one exists.
    let mut by_name: HashMap<String, HashSet<String>> = HashMap::new();
    for r in rows.iter().filter(|r| !r.project.contains("/.codex/worktrees/")) {
        if let Some(name) = Path::new(&r.project).file_name().and_then(|n| n.to_str()) {
            by_name.entry(name.to_string()).or_default().insert(r.project.clone());
        }
    }
    for r in rows.iter_mut() {
        if let Some(i) = r.project.find("/.codex/worktrees/") {
            let rest = &r.project[i + "/.codex/worktrees/".len()..];
            let name = rest.split('/').nth(1).unwrap_or("");
            if let Some(set) = by_name.get(name)
                && set.len() == 1
            {
                r.project = set.iter().next().unwrap().clone();
            }
        }
    }
    Ok(rows)
}

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub path: String,
    pub snippet: String,
    pub rank: f64,
}

pub fn search(conn: &Connection, query: &str) -> anyhow::Result<Vec<SearchHit>> {
    let fts_query = to_fts_query(query);
    if fts_query.is_empty() {
        return Ok(vec![]);
    }
    let mut stmt = conn.prepare(
        "SELECT path, snippet(session_fts, 2, '\u{1}', '\u{2}', ' … ', 14), bm25(session_fts, 0.0, 4.0, 1.0)
         FROM session_fts WHERE session_fts MATCH ?1 ORDER BY 3 LIMIT 500",
    )?;
    let hits = stmt
        .query_map([fts_query], |r| Ok(SearchHit { path: r.get(0)?, snippet: r.get(1)?, rank: r.get(2)? }))?
        .collect::<Result<_, _>>()?;
    Ok(hits)
}

/// Converts free text into an FTS5 query: each word becomes a quoted prefix
/// term so punctuation in user input cannot produce a syntax error.
fn to_fts_query(q: &str) -> String {
    q.split_whitespace()
        .map(|w| w.replace('"', ""))
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn root_counts(conn: &Connection) -> anyhow::Result<HashMap<String, i64>> {
    let mut stmt = conn.prepare("SELECT root, COUNT(*) FROM sessions GROUP BY root")?;
    let map = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Indexes the real agent directories on this machine into a throwaway
    /// database. Run with: SP_TEST_DB=/tmp/x.sqlite cargo test -- --ignored --nocapture
    #[test]
    #[ignore]
    fn index_local_sessions() {
        let db = std::env::var("SP_TEST_DB").unwrap_or_else(|_| "/tmp/session-player-test.sqlite".into());
        let _ = std::fs::remove_file(&db);
        let mut conn = open(Path::new(&db)).unwrap();
        seed_roots(&conn).unwrap();
        let stats = run(&mut conn, false, &|_, _| {}).unwrap();
        println!("{stats:?}");
        let again = run(&mut conn, false, &|_, _| {}).unwrap();
        println!("second pass: {again:?}");
        for r in list(&conn).unwrap() {
            println!(
                "{:<6} {:<8} {:<45} | {:<70} | u{} a{} t{}{}",
                r.source,
                r.title_source,
                r.project.replace("/Users/danmactough/", "~/"),
                r.title,
                r.user_turns,
                r.assistant_msgs,
                r.tool_calls,
                if r.is_subagent { " [sub]" } else { "" }
            );
        }
        let failed: Vec<(String, String)> = conn
            .prepare("SELECT path, error FROM files WHERE error IS NOT NULL")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        println!("failed: {failed:?}");
        let undetected: i64 = conn
            .query_row("SELECT COUNT(*) FROM files WHERE source IS NULL", [], |r| r.get(0))
            .unwrap();
        println!("undetected: {undetected}");
        println!("search 'feedparser': {:?}", search(&conn, "async iterator").unwrap().into_iter().take(3).collect::<Vec<_>>());
    }
}

#[cfg(test)]
mod perf {
    use super::*;

    /// Times a full parse of the largest local session files and reports the
    /// size of the payload that would be sent to the UI.
    #[test]
    #[ignore]
    fn parse_largest_sessions() {
        let mut files: Vec<(u64, PathBuf)> = default_roots()
            .iter()
            .flat_map(|r| WalkDir::new(r).into_iter().filter_map(Result::ok))
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("jsonl"))
            .map(|e| (e.metadata().map(|m| m.len()).unwrap_or(0), e.path().to_path_buf()))
            .collect();
        files.sort_by(|a, b| b.0.cmp(&a.0));
        for (size, path) in files.into_iter().take(8) {
            let t = std::time::Instant::now();
            let source = parse::detect(&path).unwrap();
            let events = parse::parse_file(&path, source).unwrap().events;
            let json = serde_json::to_string(&events).unwrap();
            println!(
                "{:?} file={}KB events={} payload={}KB images={} took={}ms",
                source,
                size / 1024,
                events.len(),
                json.len() / 1024,
                events.iter().map(|e| e.images.len()).sum::<usize>(),
                t.elapsed().as_millis()
            );
        }
    }
}
