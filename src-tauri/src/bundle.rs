//! Portable session bundles: one session (plus the subagents the user chose)
//! exported to a single JSON file that another Session Player can import and
//! replay without the agent's files. The format is described in
//! docs/bundle-format.md; keep the two in step.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};
use base64::Engine;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::model::{Event, EventKind, Parsed, Source, Tokens};
use crate::parse;

pub const FORMAT: &str = "session-player.bundle";
pub const VERSION: u32 = 1;
pub const EXTENSION: &str = "spsession";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bundle {
    pub format: String,
    pub version: u32,
    pub exported_at: i64,
    /// The app and version that wrote the bundle.
    pub exporter: String,
    /// The session the user exported comes first; any others are its subagents.
    pub sessions: Vec<BundledSession>,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BundledSession {
    /// Stable across exports: "<source>:<native id>", plus "@<leaf>" for a pi branch.
    pub id: String,
    pub source: Source,
    pub native_id: String,
    pub branch_leaf: Option<String>,
    pub title: String,
    pub title_source: String,
    pub project: String,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub models: Vec<String>,
    pub tokens: Tokens,
    pub started_at: Option<i64>,
    pub is_subagent: bool,
    pub parent_id: Option<String>,
    pub imported_from: Option<String>,
    /// File name of the session log it was read from; never the full path.
    pub source_file: String,
    /// Related sessions that are not in the bundle, so the recipient knows
    /// what was left out.
    pub omitted: Vec<Related>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Relation {
    /// The session this one is a subagent or fork of.
    Parent,
    Subagent,
    /// Another branch of the same pi session.
    Branch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Related {
    pub relation: Relation,
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentKind {
    Image,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentStatus {
    Included,
    Omitted,
}

/// Something attached to an event. Images are carried here (base64 in
/// `data`) and referenced from the event as "attachment:<id>"; a file's
/// contents stay inline in the prompt's `<file>` card, so its entry only
/// records whether they were captured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    /// The `id` of the session it belongs to.
    pub session: String,
    /// The event it is attached to.
    pub anchor: String,
    pub kind: AttachmentKind,
    pub name: Option<String>,
    pub mime: Option<String>,
    pub bytes: Option<u64>,
    pub status: AttachmentStatus,
    /// Why an omitted attachment is not in the bundle.
    pub reason: Option<String>,
    pub data: Option<String>,
}

pub struct ExportRequest<'a> {
    /// The session file, as indexed (a library bundle for an imported session).
    pub path: &'a str,
    /// A pi branch's leaf entry, to export that branch instead of the active one.
    pub branch: Option<&'a str>,
    /// Paths of the subagents to include; None includes all of them.
    pub subagents: Option<Vec<String>>,
}

pub fn is_bundle_path(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some(EXTENSION)
}

/// Builds the bundle for one session from its file and index row. Nothing is
/// written; see `write`.
pub fn build(conn: &Connection, req: ExportRequest, exporter: &str) -> anyhow::Result<Bundle> {
    let mut attachments = Vec::new();
    let mut primary = bundled_session(conn, req.path, req.branch, &mut attachments)?;
    let mut sessions = Vec::new();
    // A pi branch is a path through one file; subagents belong to the session as a whole.
    let subagents = if req.branch.is_some() { Vec::new() } else { subagents_of(conn, req.path)? };
    for (path, title) in subagents {
        if req.subagents.as_ref().is_none_or(|wanted| wanted.contains(&path)) {
            sessions.push(bundled_session(conn, &path, None, &mut attachments)?);
        } else {
            primary.omitted.push(Related { relation: Relation::Subagent, id: session_id_of(conn, &path)?, title });
        }
    }
    sessions.insert(0, primary);
    Ok(Bundle {
        format: FORMAT.into(),
        version: VERSION,
        exported_at: chrono::Utc::now().timestamp_millis(),
        exporter: exporter.into(),
        sessions,
        attachments,
    })
}

/// Paths and titles of the subagents indexed under the session at `path`.
/// An imported session only has the subagents imported with it, and a local
/// one only local subagents, even when both copies are indexed.
pub fn subagents_of(conn: &Connection, path: &str) -> anyhow::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT c.path, c.title FROM sessions c JOIN sessions p
           ON c.source = p.source AND c.parent_id = p.native_id AND c.from_bundle = p.from_bundle
         WHERE p.path = ?1 AND c.is_subagent = 1
         ORDER BY c.started_at, c.path",
    )?;
    let rows = stmt.query_map([path], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    Ok(rows)
}

fn session_id_of(conn: &Connection, path: &str) -> anyhow::Result<String> {
    Ok(conn.query_row("SELECT source || ':' || native_id FROM sessions WHERE path=?1", [path], |r| r.get(0))?)
}

/// The parent of the session at `path`, as an omitted related session.
fn parent_of(conn: &Connection, path: &str) -> anyhow::Result<Option<Related>> {
    Ok(conn
        .query_row(
            "SELECT c.source, c.parent_id, p.title FROM sessions c LEFT JOIN sessions p
               ON p.source = c.source AND p.native_id = c.parent_id AND p.from_bundle = c.from_bundle
             WHERE c.path = ?1 AND c.parent_id IS NOT NULL",
            [path],
            |r| {
                let (source, parent): (String, String) = (r.get(0)?, r.get(1)?);
                Ok(Related {
                    relation: Relation::Parent,
                    id: format!("{source}:{parent}"),
                    title: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                })
            },
        )
        .optional()?)
}

struct IndexedMeta {
    title: String,
    title_source: String,
    project: String,
    imported_from: Option<String>,
}

fn indexed_meta(conn: &Connection, path: &str) -> anyhow::Result<IndexedMeta> {
    conn.query_row(
        "SELECT title, title_source, project, imported_from FROM sessions WHERE path=?1",
        [path],
        |r| Ok(IndexedMeta { title: r.get(0)?, title_source: r.get(1)?, project: r.get(2)?, imported_from: r.get(3)? }),
    )
    .optional()?
    .ok_or_else(|| anyhow!("This session isn't in the index yet. Wait for indexing to finish and try again."))
}

fn bundled_session(
    conn: &Connection,
    path: &str,
    branch: Option<&str>,
    attachments: &mut Vec<Attachment>,
) -> anyhow::Result<BundledSession> {
    if is_bundle_path(Path::new(path)) {
        return library_session(path, attachments);
    }
    let meta = indexed_meta(conn, path)?;
    let file = Path::new(path);
    let source = parse::detect(file).ok_or_else(|| anyhow!("Unrecognized session format: {path}"))?;
    let parsed = match (branch, source) {
        (Some(leaf), Source::Pi) => parse::pi::parse_chain(file, Some(leaf))?,
        _ => parse::parse_file(file, source)?,
    };
    let session_id = format!("{}:{}", source.as_str(), parsed.native_id);
    let id = match branch {
        Some(leaf) => format!("{session_id}@{leaf}"),
        None => session_id.clone(),
    };
    // Only the chosen path through a pi session is exported; every other
    // branch, the active one included, is listed as left out.
    let mut title = meta.title;
    let mut omitted: Vec<Related> = parent_of(conn, path)?.into_iter().collect();
    if branch.is_some() {
        omitted.push(Related { relation: Relation::Branch, id: session_id.clone(), title: title.clone() });
    }
    for b in &parsed.branches {
        if Some(b.leaf.as_str()) == branch {
            title = b.label.clone();
        } else {
            omitted.push(Related { relation: Relation::Branch, id: format!("{session_id}@{}", b.leaf), title: b.label.clone() });
        }
    }
    let mut events = parsed.events;
    for (i, e) in events.iter_mut().enumerate() {
        let anchor = format!("e{i}");
        for image in e.images.iter_mut() {
            let attachment = image_attachment(format!("a{}", attachments.len()), &id, &anchor, image);
            *image = format!("attachment:{}", attachment.id);
            attachments.push(attachment);
        }
        if e.kind == EventKind::User {
            for (name, contents) in file_cards(&e.text) {
                attachments.push(file_attachment(format!("a{}", attachments.len()), &id, &anchor, name, contents));
            }
        }
        e.anchor = Some(anchor);
    }
    Ok(BundledSession {
        id,
        source,
        native_id: parsed.native_id,
        branch_leaf: branch.map(str::to_string),
        title,
        title_source: if branch.is_some() { "branch".into() } else { meta.title_source },
        project: meta.project,
        cwd: parsed.cwd,
        git_branch: parsed.git_branch,
        models: parsed.models,
        tokens: parsed.tokens,
        started_at: parsed.started_at,
        is_subagent: parsed.is_subagent,
        parent_id: parsed.parent_id,
        imported_from: meta.imported_from,
        source_file: file.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string(),
        omitted,
        events,
    })
}

/// A session imported earlier, exported again as it was imported. Its
/// attachments are renumbered to follow the ones already collected.
fn library_session(path: &str, attachments: &mut Vec<Attachment>) -> anyhow::Result<BundledSession> {
    let mut bundle = read(Path::new(path))?;
    let mut session = bundle.sessions.swap_remove(0);
    let mut renamed = std::collections::HashMap::new();
    for mut a in bundle.attachments {
        let id = format!("a{}", attachments.len());
        renamed.insert(std::mem::replace(&mut a.id, id.clone()), id);
        attachments.push(a);
    }
    for image in session.events.iter_mut().flat_map(|e| e.images.iter_mut()) {
        if let Some(new) = image.strip_prefix("attachment:").and_then(|old| renamed.get(old)) {
            *image = format!("attachment:{new}");
        }
    }
    Ok(session)
}

/// The `<file name="…" note="…">contents</file>` cards in a prompt (see
/// `parse::normalize_prompt`), as (name, contents).
fn file_cards(text: &str) -> Vec<(&str, &str)> {
    let mut cards = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<file name=\"") {
        let tag = &rest[start + "<file name=\"".len()..];
        let (Some(name_end), Some(open_end)) = (tag.find('"'), tag.find('>')) else { break };
        let body = &tag[open_end + 1..];
        let Some(close) = body.find("</file>") else { break };
        cards.push((&tag[..name_end], &body[..close]));
        rest = &body[close + "</file>".len()..];
    }
    cards
}

fn file_attachment(id: String, session: &str, anchor: &str, name: &str, contents: &str) -> Attachment {
    let captured = !contents.trim().is_empty();
    Attachment {
        id,
        session: session.to_string(),
        anchor: anchor.to_string(),
        kind: AttachmentKind::File,
        name: Some(name.to_string()),
        mime: None,
        bytes: captured.then_some(contents.len() as u64),
        status: if captured { AttachmentStatus::Included } else { AttachmentStatus::Omitted },
        reason: (!captured).then(|| "the agent recorded only the path, not the contents".to_string()),
        data: None,
    }
}

/// An image as the parsers deliver it: a data URL, "omitted:large" for one
/// too big to load, or a link to an image hosted elsewhere.
fn image_attachment(id: String, session: &str, anchor: &str, image: &str) -> Attachment {
    let mut a = Attachment {
        id,
        session: session.to_string(),
        anchor: anchor.to_string(),
        kind: AttachmentKind::Image,
        name: None,
        mime: None,
        bytes: None,
        status: AttachmentStatus::Omitted,
        reason: None,
        data: None,
    };
    match image.strip_prefix("data:").and_then(|d| d.split_once(";base64,")) {
        Some((mime, data)) => {
            let padding = data.bytes().rev().take_while(|b| *b == b'=').count();
            a.mime = Some(mime.to_string());
            a.bytes = Some((data.len() / 4 * 3).saturating_sub(padding) as u64);
            a.status = AttachmentStatus::Included;
            a.data = Some(data.to_string());
        }
        None if image.starts_with("omitted") => a.reason = Some("larger than Session Player loads".into()),
        None => a.reason = Some(format!("linked, not embedded: {image}")),
    }
    a
}

/// Writes the bundle to `dest` through a temporary file, so a failed write
/// never leaves a partial bundle under the real name.
pub fn write(bundle: &Bundle, dest: &Path) -> anyhow::Result<()> {
    let tmp = dest.with_extension("partial");
    std::fs::write(&tmp, serde_json::to_vec(bundle)?).with_context(|| format!("Couldn't write {}", tmp.display()))?;
    std::fs::rename(&tmp, dest).with_context(|| format!("Couldn't write {}", dest.display()))?;
    Ok(())
}

/// Reads a bundle file and checks that it is one this version can replay:
/// see `validate`.
pub fn read(path: &Path) -> anyhow::Result<Bundle> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
    let size = std::fs::metadata(path).with_context(|| format!("Couldn't read {name}"))?.len();
    if size > MAX_BUNDLE_BYTES {
        bail!("{name} is {} MB, more than the {} MB a bundle can be", size >> 20, MAX_BUNDLE_BYTES >> 20);
    }
    let bytes = std::fs::read(path).with_context(|| format!("Couldn't read {name}"))?;
    let not_a_bundle = || anyhow!("{name} isn't a Session Player session bundle");
    // Check the header before the body, so an unrelated JSON file or a bundle
    // from a newer version gets a clear message rather than a field error.
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| not_a_bundle())?;
    if value.get("format").and_then(|f| f.as_str()) != Some(FORMAT) {
        return Err(not_a_bundle());
    }
    match value.get("version").and_then(|v| v.as_u64()) {
        None => bail!("{name} has no version, so Session Player can't tell how to read it"),
        Some(v) if v > VERSION as u64 => {
            bail!("{name} was made by a newer version of Session Player (bundle version {v}). Update Session Player to open it.")
        }
        Some(v) if v < 1 => bail!("{name} has bundle version {v}, which Session Player doesn't support"),
        Some(_) => {}
    }
    let bundle: Bundle = serde_json::from_value(value).with_context(|| format!("{name} is damaged"))?;
    validate(&bundle).with_context(|| format!("{name} is damaged"))?;
    Ok(bundle)
}

/// Bundles carry screenshots inline; this keeps a mistaken pick (a disk
/// image renamed .spsession) from being read into memory.
const MAX_BUNDLE_BYTES: u64 = 512 << 20;

/// The rules a bundle must follow beyond its shape (docs/bundle-format.md):
/// unique session ids, extra sessions are subagents of the first, anchors
/// unique per session, and every attachment reference resolves.
fn validate(bundle: &Bundle) -> anyhow::Result<()> {
    let Some(primary) = bundle.sessions.first() else { bail!("it has no sessions") };
    let mut ids = HashSet::new();
    for s in &bundle.sessions {
        if !ids.insert(s.id.as_str()) {
            bail!("session {} appears more than once", s.id);
        }
        let mut anchors = HashSet::new();
        for anchor in s.events.iter().filter_map(|e| e.anchor.as_deref()) {
            if !anchors.insert(anchor) {
                bail!("in session {}, anchor {anchor} appears more than once", s.id);
            }
        }
    }
    for s in &bundle.sessions[1..] {
        if s.source != primary.source || s.parent_id.as_deref() != Some(primary.native_id.as_str()) {
            bail!("session {} isn't a subagent of {}", s.id, primary.id);
        }
    }
    let mut attachment_ids = HashSet::new();
    for a in &bundle.attachments {
        if !attachment_ids.insert(a.id.as_str()) {
            bail!("attachment {} appears more than once", a.id);
        }
        if !ids.contains(a.session.as_str()) {
            bail!("attachment {} belongs to session {}, which isn't in the bundle", a.id, a.session);
        }
        match (a.status, &a.data) {
            (AttachmentStatus::Included, None) if a.kind == AttachmentKind::Image => {
                bail!("attachment {} is marked included but has no data", a.id)
            }
            (AttachmentStatus::Included, Some(data)) => {
                base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(|_| anyhow!("attachment {} isn't valid base64", a.id))?;
            }
            (AttachmentStatus::Omitted, Some(_)) => bail!("attachment {} is marked omitted but carries data", a.id),
            _ => {}
        }
    }
    for s in &bundle.sessions {
        for image in s.events.iter().flat_map(|e| &e.images) {
            let Some(id) = image.strip_prefix("attachment:") else {
                bail!("session {} has an image that isn't an attachment reference", s.id);
            };
            if !bundle.attachments.iter().any(|a| a.id == id && a.session == s.id) {
                bail!("session {} refers to missing attachment {id}", s.id);
            }
        }
    }
    Ok(())
}

/// What to do with a session whose id is already in the library with
/// different contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnConflict {
    Replace,
    Keep,
}

#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    /// Session ids written to the library.
    pub imported: Vec<String>,
    /// Already in the library with the same contents.
    pub unchanged: Vec<String>,
    /// Left as they were because the user chose to keep the library's copy.
    pub kept: Vec<String>,
    /// Waiting for a decision; when any are listed, nothing was written.
    pub conflicts: Vec<Conflict>,
}

#[derive(Debug, Serialize)]
pub struct Conflict {
    pub id: String,
    pub existing_title: String,
    pub incoming_title: String,
}

/// Copies each session in the bundle at `path` into `library` as a
/// single-session bundle named after its id, so one file is one indexed
/// session. A session already there with the same contents is skipped. One
/// with different contents is a conflict: without `on_conflict` the import
/// stops before writing anything and reports the conflicts, so the user can
/// choose to replace or keep the library's copies.
pub fn import(library: &Path, path: &Path, on_conflict: Option<OnConflict>) -> anyhow::Result<ImportReport> {
    let bundle = read(path)?;
    let mut report = ImportReport::default();
    let mut writes: Vec<(PathBuf, Bundle)> = Vec::new();
    for s in &bundle.sessions {
        let single = Bundle {
            sessions: vec![s.clone()],
            attachments: bundle.attachments.iter().filter(|a| a.session == s.id).cloned().collect(),
            ..bundle.clone()
        };
        let dest = library_file(library, &s.id);
        if dest.exists() {
            let existing = read(&dest)?;
            if existing.sessions == single.sessions && existing.attachments == single.attachments {
                report.unchanged.push(s.id.clone());
                continue;
            }
            match on_conflict {
                Some(OnConflict::Replace) => {}
                Some(OnConflict::Keep) => {
                    report.kept.push(s.id.clone());
                    continue;
                }
                None => {
                    report.conflicts.push(Conflict {
                        id: s.id.clone(),
                        existing_title: existing.sessions[0].title.clone(),
                        incoming_title: s.title.clone(),
                    });
                    continue;
                }
            }
        }
        writes.push((dest, single));
    }
    if !report.conflicts.is_empty() {
        return Ok(report);
    }
    std::fs::create_dir_all(library).with_context(|| format!("Couldn't create {}", library.display()))?;
    for (dest, single) in writes {
        write(&single, &dest)?;
        report.imported.push(single.sessions[0].id.clone());
    }
    Ok(report)
}

/// The id is encoded so any id maps to its own safe file name.
fn library_file(library: &Path, id: &str) -> PathBuf {
    let name = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(id);
    library.join(format!("{name}.{EXTENSION}"))
}

/// The session in a library bundle, in the shape the indexer builds rows from.
pub fn to_parsed(s: BundledSession) -> Parsed {
    let mut p = Parsed::new(s.source, s.native_id);
    p.cwd = s.cwd;
    p.project = Some(s.project);
    p.git_branch = s.git_branch;
    p.models = s.models;
    p.tokens = s.tokens;
    p.events = s.events;
    p.is_subagent = s.is_subagent;
    p.parent_id = s.parent_id;
    p.started_at = s.started_at;
    p.imported_from = s.imported_from;
    p
}

/// The transcript of a library bundle.
/// The transcript of a library bundle, with each image attachment back in
/// the form the parsers produce.
pub fn load_events(path: &Path) -> anyhow::Result<Vec<Event>> {
    let mut bundle = read(path)?;
    let mut events = bundle.sessions.swap_remove(0).events;
    for image in events.iter_mut().flat_map(|e| e.images.iter_mut()) {
        let Some(id) = image.strip_prefix("attachment:") else { continue };
        let attachment = bundle.attachments.iter().find(|a| a.id == id);
        *image = match attachment.and_then(|a| Some((a.mime.as_deref()?, a.data.as_deref()?))) {
            Some((mime, data)) => format!("data:{mime};base64,{data}"),
            None => "omitted:large".into(),
        };
    }
    Ok(events)
}

/// What the export sheet shows before anything is saved: the bundle's
/// contents without the attachment data, and its exact size on disk.
#[derive(Debug, Serialize)]
pub struct Summary {
    pub bytes: u64,
    /// A file name for the save dialog, from the session's title.
    pub file_name: String,
    pub sessions: Vec<SessionSummary>,
    pub omitted: Vec<Related>,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub is_subagent: bool,
    pub prompts: usize,
    pub events: usize,
}

pub fn summarize(bundle: &Bundle) -> Summary {
    let primary = &bundle.sessions[0];
    let safe: String = primary
        .title
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':') { '-' } else { c })
        .take(80)
        .collect();
    let stem = match safe.trim().trim_start_matches('.') {
        "" => "session",
        s => s,
    };
    Summary {
        bytes: serde_json::to_vec(bundle).map_or(0, |b| b.len() as u64),
        file_name: format!("{stem}.{EXTENSION}"),
        sessions: bundle
            .sessions
            .iter()
            .map(|s| SessionSummary {
                id: s.id.clone(),
                title: s.title.clone(),
                is_subagent: s.is_subagent,
                prompts: s.events.iter().filter(|e| e.kind == EventKind::User && !e.text.is_empty()).count(),
                events: s.events.len(),
            })
            .collect(),
        omitted: bundle.sessions.iter().flat_map(|s| s.omitted.iter().cloned()).collect(),
        attachments: bundle.attachments.iter().map(|a| Attachment { data: None, ..a.clone() }).collect(),
    }
}
