//! Portable session bundles: one session (plus the subagents the user chose)
//! exported to a single JSON file that another Session Player can import and
//! replay without the agent's files. The format is described in
//! docs/bundle-format.md; keep the two in step.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};
use base64::Engine;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::annotations::{self, Annotation, EventRef, SessionMeta};
use crate::model::{ContextSource, Event, EventKind, Parsed, Source, Tokens};
use crate::parse;

pub const FORMAT: &str = "session-player.bundle";
/// 2: events can be `context`, and annotations `ignored_instruction`.
pub const VERSION: u32 = 2;
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
    /// What people (or a model) wrote about the session. Their `key` is
    /// empty here; import files them under the imported session's key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<SessionMeta>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
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
        return library_session(conn, path, attachments);
    }
    let meta = indexed_meta(conn, path)?;
    let file = Path::new(path);
    let source = parse::detect(file).ok_or_else(|| anyhow!("Unrecognized session format: {path}"))?;
    // pi forks and branches start at their split point in the app; a bundle
    // keeps the history before it so the transcript reads on its own.
    let parsed = match source {
        Source::Pi => parse::pi::parse_chain(file, branch)?,
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
    // Attachments are numbered per session, so a session's attachments keep
    // their ids whichever other sessions share its bundle.
    let mut next_attachment = 0;
    let mut next_id = || {
        next_attachment += 1;
        format!("a{}", next_attachment - 1)
    };
    // Each remote address is fetched once per session; later uses share the attachment.
    let mut remote: HashMap<String, String> = HashMap::new();
    // Suggestions record the revision of the transcript as the app shows it;
    // export changes the text (remote image addresses), so they are re-stamped.
    let shown_revision = crate::generate::revision(&parsed.events);
    let mut events = parsed.events;
    for e in events.iter_mut() {
        let anchor = e.anchor.clone().unwrap_or_default();
        for image in e.images.iter_mut() {
            let attachment = match remote.get(image.as_str()) {
                Some(existing) => {
                    *image = format!("attachment:{existing}");
                    continue;
                }
                None if is_remote(image) => {
                    let a = remote_image(next_id(), &id, &anchor, image);
                    remote.insert(image.clone(), a.id.clone());
                    a
                }
                None => image_attachment(next_id(), &id, &anchor, image),
            };
            *image = format!("attachment:{}", attachment.id);
            attachments.push(attachment);
        }
        // Images linked from Markdown or HTML in rendered text would otherwise
        // load from the web when the recipient reads the transcript.
        if renders_markdown(e.kind) {
            let spans: Vec<_> = image_link_spans(&e.text).into_iter().filter(|r| is_remote(&e.text[r.clone()])).collect();
            for span in spans.into_iter().rev() {
                let url = e.text[span.clone()].to_string();
                let attachment_id = match remote.get(&url) {
                    Some(existing) => existing.clone(),
                    None => {
                        let a = remote_image(next_id(), &id, &anchor, &url);
                        let new_id = a.id.clone();
                        remote.insert(url, new_id.clone());
                        attachments.push(a);
                        new_id
                    }
                };
                e.text.replace_range(span, &format!("attachment:{attachment_id}"));
            }
        }
        if e.kind == EventKind::User {
            for (name, contents) in file_cards(&e.text) {
                attachments.push(file_attachment(next_id(), &id, &anchor, name, contents));
            }
        }
    }
    BundledSession {
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
        meta: None,
        annotations: Vec::new(),
        events,
    }
    .with_notes(conn, &format!("local:{session_id}"), Some(&shown_revision))
}

/// A session imported earlier, exported again as it was imported, with the
/// notes as they are now in this library.
fn library_session(conn: &Connection, path: &str, attachments: &mut Vec<Attachment>) -> anyhow::Result<BundledSession> {
    let mut bundle = read(Path::new(path))?;
    attachments.append(&mut bundle.attachments);
    let session = bundle.sessions.swap_remove(0);
    let key = format!("bundle:{}", session.id);
    session.with_notes(conn, &key, None)
}

impl BundledSession {
    /// Adds the metadata and annotations stored under `key`. A pi session's
    /// notes cover every branch, so only annotations about events in this
    /// transcript (or the session as a whole) come along.
    fn with_notes(mut self, conn: &Connection, key: &str, shown_revision: Option<&str>) -> anyhow::Result<BundledSession> {
        let bundled_revision = crate::generate::revision(&self.events);
        let restamp = |r: &mut Option<String>| {
            if r.is_some() && r.as_deref() == shown_revision {
                *r = Some(bundled_revision.clone());
            }
        };
        self.meta = annotations::meta(conn, key)?.map(|mut m| {
            if let Some(g) = &mut m.generated_by {
                let mut r = Some(g.revision.clone());
                restamp(&mut r);
                g.revision = r.unwrap_or_default();
            }
            SessionMeta { key: String::new(), ..m }
        });
        let mut list = annotations::annotations(conn, key)?;
        annotations::resolve(&mut list, &self.events, None);
        self.annotations = list
            .into_iter()
            .filter(|a| a.anchors.is_empty() || a.anchors.iter().any(|r| r.found))
            .map(|mut a| {
                a.key = String::new();
                // A bundle is a snapshot: it carries only the current version, unnumbered.
                a.version = 0;
                restamp(&mut a.revision);
                for r in &mut a.anchors {
                    r.found = false;
                    r.elsewhere = false;
                }
                a
            })
            .collect();
        Ok(self)
    }
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

/// The events the transcript renders as Markdown (prompts are plain text).
fn renders_markdown(kind: EventKind) -> bool {
    matches!(kind, EventKind::Assistant | EventKind::Thinking | EventKind::Compaction | EventKind::Context)
}

fn is_remote(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Byte ranges of the image addresses in Markdown images (`![alt](url)`)
/// and HTML `<img src="url">` tags, outside code. Matches never overlap, so
/// callers can replace them from last to first.
pub(crate) fn image_link_spans(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find("![") {
        let start = from + i + 2;
        let Some(close) = text[start..].find("](") else { break };
        let url_start = start + close + 2;
        let url_len = text[url_start..].find(|c: char| c == ')' || c.is_whitespace()).unwrap_or(text.len() - url_start);
        spans.push(url_start..url_start + url_len);
        from = url_start + url_len;
    }
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find("<img") {
        let tag_start = from + i;
        let tag_end = lower[tag_start..].find('>').map_or(text.len(), |n| tag_start + n);
        from = tag_end;
        let tag = &lower[tag_start..tag_end];
        // `src=` as its own attribute, not the end of `data-src=`.
        let Some(src) = tag.match_indices("src=").map(|(n, _)| n).find(|&n| tag[..n].ends_with([' ', '\t', '\n', '/'])) else {
            continue;
        };
        let value = tag_start + src + "src=".len();
        let Some(quote) = text[value..].chars().next().filter(|c| *c == '"' || *c == '\'') else { continue };
        let Some(len) = text[value + 1..tag_end].find(quote) else { continue };
        spans.push(value + 1..value + 1 + len);
    }
    let code = code_ranges(text);
    spans.retain(|r| !code.iter().any(|c| c.contains(&r.start)));
    spans.sort_by_key(|r| r.start);
    let mut last_end = 0;
    spans.retain(|r| {
        let keep = r.start >= last_end;
        if keep {
            last_end = r.end;
        }
        keep
    });
    spans
}

/// Fenced code blocks and inline code spans, where an image link is text to
/// show rather than an image.
fn code_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut fence: Option<(usize, &str)> = None;
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let marker = ["```", "~~~"].into_iter().find(|m| trimmed.starts_with(m));
        match (fence, marker) {
            (None, Some(m)) => fence = Some((line_start, m)),
            (Some((start, open)), Some(m)) if m == open => {
                ranges.push(start..line_start + line.len());
                fence = None;
            }
            (Some(_), _) => {}
            (None, None) => {
                // Inline code: a run of backticks up to the next run of the same length.
                let mut i = 0;
                while let Some(n) = line[i..].find('`') {
                    let open = i + n;
                    let ticks = line[open..].len() - line[open..].trim_start_matches('`').len();
                    let after = open + ticks;
                    let closing = line[after..]
                        .match_indices(&"`".repeat(ticks))
                        .map(|(n, _)| after + n)
                        .find(|&c| !line[c + ticks..].starts_with('`') && !line[..c].ends_with('`'));
                    match closing {
                        Some(c) => {
                            ranges.push(line_start + open..line_start + c + ticks);
                            i = c + ticks;
                        }
                        None => i = after,
                    }
                }
            }
        }
        line_start += line.len();
    }
    if let Some((start, _)) = fence {
        ranges.push(start..text.len());
    }
    ranges
}

/// The largest remote image downloaded, matching what the parsers keep of an
/// embedded one (`parse::MAX_IMAGE_B64` of base64).
const MAX_REMOTE_IMAGE: u64 = 3 << 20;

/// Downloads an image the transcript links to, so the bundle carries it and
/// the recipient's app never fetches it.
fn remote_image(id: String, session: &str, anchor: &str, url: &str) -> Attachment {
    let mut a = Attachment {
        id,
        session: session.to_string(),
        anchor: anchor.to_string(),
        kind: AttachmentKind::Image,
        name: Some(url.to_string()),
        mime: None,
        bytes: None,
        status: AttachmentStatus::Omitted,
        reason: None,
        data: None,
    };
    match download_image(url) {
        Ok((mime, bytes)) => {
            a.mime = Some(mime);
            a.bytes = Some(bytes.len() as u64);
            a.status = AttachmentStatus::Included;
            a.data = Some(base64::engine::general_purpose::STANDARD.encode(bytes));
        }
        Err(why) => a.reason = Some(format!("couldn't download it: {why}")),
    }
    a
}

/// Fetches with the system's curl, which keeps an HTTP client out of the
/// app's dependencies.
fn download_image(url: &str) -> Result<(String, Vec<u8>), String> {
    let out = std::process::Command::new("/usr/bin/curl")
        // -q must come first: it stops curl reading the user's ~/.curlrc.
        .arg("-q")
        .args(["--silent", "--show-error", "--fail", "--location", "--max-time", "20"])
        .args(["--proto", "=http,https", "--proto-redir", "=http,https"])
        .args(["--max-filesize", &MAX_REMOTE_IMAGE.to_string()])
        // The body goes to stdout and the content type to stderr after it.
        .args(["--write-out", "%{stderr}%{content_type}", "--", url])
        .output()
        .map_err(|e| e.to_string())?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        // The content type is written after the error; keep the error line.
        let message = stderr.lines().next().unwrap_or("").trim().trim_start_matches("curl: ");
        return Err(message.split(')').next_back().unwrap_or(message).trim().to_string());
    }
    let mime = stderr.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    if !IMAGE_TYPES.contains(&mime.as_str()) {
        return Err(format!("not an image ({})", if mime.is_empty() { "no type" } else { &mime }));
    }
    if out.stdout.len() as u64 > MAX_REMOTE_IMAGE {
        return Err(format!("larger than {} MB", MAX_REMOTE_IMAGE >> 20));
    }
    Ok((mime, out.stdout))
}

/// Writes the bundle to `dest` through a temporary file, so a failed write
/// never leaves a partial bundle under the real name.
pub fn write(bundle: &Bundle, dest: &Path) -> anyhow::Result<()> {
    let mut tmp = dest.as_os_str().to_owned();
    tmp.push(".partial");
    let tmp = PathBuf::from(tmp);
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

const IMAGE_TYPES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp", "image/svg+xml"];

/// Long enough for any agent's ids; keeps the library file name within
/// the file system's 255-byte limit once base64-encoded.
const MAX_ID_LEN: usize = 180;

/// Bundles carry screenshots inline; this keeps a mistaken pick (a disk
/// image renamed .spsession) from being read into memory.
const MAX_BUNDLE_BYTES: u64 = 512 << 20;

/// The rules a bundle must follow beyond its shape (docs/bundle-format.md):
/// unique session ids, extra sessions are subagents of the first, anchors
/// unique per session, and every attachment reference resolves.
fn validate(bundle: &Bundle) -> anyhow::Result<()> {
    let mut annotation_ids = HashSet::new();
    for s in &bundle.sessions {
        for a in &s.annotations {
            if a.anchors.is_empty() {
                bail!("session {} has an annotation that doesn't point at an event", s.id);
            }
            if a.id.is_empty() || !annotation_ids.insert(a.id.as_str()) {
                bail!("session {} has an annotation with a missing or repeated id", s.id);
            }
        }
    }
    let Some(primary) = bundle.sessions.first() else { bail!("it has no sessions") };
    let mut ids = HashSet::new();
    for s in &bundle.sessions {
        // The id names the library file and decides duplicates, so it must be
        // the one this session's own fields produce.
        let expected = match &s.branch_leaf {
            Some(leaf) => format!("{}:{}@{leaf}", s.source.as_str(), s.native_id),
            None => format!("{}:{}", s.source.as_str(), s.native_id),
        };
        if s.id != expected {
            bail!("session id {:?} doesn't match {expected}", s.id);
        }
        if s.native_id.is_empty() || s.id.len() > MAX_ID_LEN {
            bail!("session id {:?} is empty or too long", s.id);
        }
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
        if !attachment_ids.insert((a.session.as_str(), a.id.as_str())) {
            bail!("attachment {} appears more than once", a.id);
        }
        if !ids.contains(a.session.as_str()) {
            bail!("attachment {} belongs to session {}, which isn't in the bundle", a.id, a.session);
        }
        match (a.status, &a.data) {
            (AttachmentStatus::Included, None) if a.kind == AttachmentKind::Image => {
                bail!("attachment {} is marked included but has no data", a.id)
            }
            (AttachmentStatus::Included, Some(_))
                if a.kind == AttachmentKind::Image && !a.mime.as_deref().is_some_and(|m| IMAGE_TYPES.contains(&m)) =>
            {
                // The type ends up in an HTTP header when the image is shown.
                bail!("attachment {} has unsupported image type {:?}", a.id, a.mime.as_deref().unwrap_or(""))
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
    /// Sessions with no prompt, reply or tool call, which would not show in
    /// the list; they are not imported.
    pub empty: Vec<String>,
    /// Details and annotations the recipient changed since an earlier import,
    /// kept rather than replaced by the bundle's.
    pub kept_notes: usize,
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
///
/// The metadata and annotations of each session written are filed under its
/// key in `conn`. Annotations are matched by id, so one the recipient added
/// stays, and one from the bundle replaces the copy with the same id.
pub fn import(conn: &Connection, library: &Path, path: &Path, on_conflict: Option<OnConflict>) -> anyhow::Result<ImportReport> {
    let bundle = read(path)?;
    let mut report = ImportReport::default();
    // Subagents are imported only with the session they belong to.
    if !has_conversation(&bundle.sessions[0]) {
        report.empty.push(bundle.sessions[0].id.clone());
        return Ok(report);
    }
    // Each write carries the copy it replaces, to tell the recipient's edits apart.
    let mut writes: Vec<(PathBuf, Bundle, Option<BundledSession>)> = Vec::new();
    for s in &bundle.sessions {
        if !has_conversation(s) {
            report.empty.push(s.id.clone());
            continue;
        }
        let single = Bundle {
            format: bundle.format.clone(),
            version: bundle.version,
            exported_at: bundle.exported_at,
            exporter: bundle.exporter.clone(),
            sessions: vec![s.clone()],
            attachments: bundle.attachments.iter().filter(|a| a.session == s.id).cloned().collect(),
        };
        let dest = library_file(library, &s.id);
        let mut previous = None;
        if dest.exists() {
            let existing = read(&dest).map_err(|e| {
                anyhow!("Your library's copy of {} is damaged ({e:#}). Remove it in Settings, under Imported sessions, then import again.", s.id)
            })?;
            if existing.sessions == single.sessions && existing.attachments == single.attachments {
                report.unchanged.push(s.id.clone());
                continue;
            }
            previous = existing.sessions.first().cloned();
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
        writes.push((dest, single, previous));
    }
    if !report.conflicts.is_empty() {
        return Ok(report);
    }
    std::fs::create_dir_all(library).with_context(|| format!("Couldn't create {}", library.display()))?;
    for (dest, single, previous) in writes {
        write(&single, &dest)?;
        let s = &single.sessions[0];
        report.kept_notes += import_notes(conn, &format!("bundle:{}", s.id), s, previous.as_ref())?;
        report.imported.push(s.id.clone());
    }
    Ok(report)
}

/// Files a session's metadata and annotations under `key`, and returns how
/// many the recipient had changed and so kept. A record is replaced only if
/// it is still as the earlier import (`previous`) left it; with no earlier
/// import, the bundle's is taken.
fn import_notes(conn: &Connection, key: &str, s: &BundledSession, previous: Option<&BundledSession>) -> anyhow::Result<usize> {
    let mut kept = 0;
    let bare_meta = |m: &SessionMeta| SessionMeta { key: String::new(), updated_at: 0, ..m.clone() };
    if let Some(meta) = &s.meta {
        let untouched = match (annotations::meta(conn, key)?, previous) {
            (None, _) | (_, None) => true,
            (Some(current), Some(p)) => p.meta.as_ref().map(bare_meta) == Some(bare_meta(&current)),
        };
        if untouched {
            annotations::store_meta(conn, &SessionMeta { key: key.to_string(), ..meta.clone() })?;
        } else {
            kept += 1;
        }
    }
    let bare = |a: &Annotation| Annotation { key: String::new(), version: 0, anchors: a.anchors.iter().map(|r| EventRef { found: false, elsewhere: false, ..r.clone() }).collect(), ..a.clone() };
    for a in &s.annotations {
        let incoming = Annotation { key: key.to_string(), ..a.clone() };
        match annotations::annotation(conn, &a.id)? {
            None => annotations::put_annotation(conn, &incoming)?,
            // The same annotation on another session, such as the local
            // session this bundle was exported from: a copy, with an id of its own.
            Some(other) if other.key != key => annotations::put_annotation(conn, &Annotation { id: annotations::new_id(), ..incoming })?,
            Some(current) => {
                let untouched = previous.is_none_or(|p| p.annotations.iter().any(|old| old.id == a.id && bare(old) == bare(&current)));
                if untouched {
                    annotations::put_annotation(conn, &incoming)?;
                } else {
                    kept += 1;
                }
            }
        }
    }
    Ok(kept)
}

/// Deletes an imported session from the library, along with the subagents
/// imported with it, and returns the files removed. `path` must be a bundle
/// directly inside `library`; a damaged one (not indexed) is removed alone.
pub fn remove(conn: &Connection, library: &Path, path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let library = library.canonicalize().ok();
    let in_library = |p: &Path| {
        is_bundle_path(p) && library.is_some() && p.parent().and_then(|d| d.canonicalize().ok()) == library
    };
    if !in_library(path) || !path.is_file() {
        bail!("{} isn't an imported session", path.display());
    }
    let mut files = vec![path.to_path_buf()];
    for (sub, _) in subagents_of(conn, &path.to_string_lossy())? {
        let sub = PathBuf::from(sub);
        if in_library(&sub) {
            files.push(sub);
        }
    }
    for f in &files {
        std::fs::remove_file(f).with_context(|| format!("Couldn't remove {}", f.display()))?;
    }
    Ok(files)
}

/// The same test the indexer applies (`index::build_row`) before listing a session.
fn has_conversation(session: &BundledSession) -> bool {
    session.events.iter().any(|e| match e.kind {
        EventKind::User | EventKind::Assistant => !e.sidechain || session.is_subagent,
        EventKind::ToolCall => true,
        _ => false,
    })
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
    let data_url = |id: &str| {
        let a = bundle.attachments.iter().find(|a| a.id == id)?;
        Some(format!("data:{};base64,{}", a.mime.as_deref()?, a.data.as_deref()?))
    };
    for e in events.iter_mut().filter(|e| renders_markdown(e.kind)) {
        for span in image_link_spans(&e.text).into_iter().rev() {
            // An image that couldn't be downloaded keeps its reference, which
            // the transcript shows as not included.
            if let Some(url) = e.text[span.clone()].strip_prefix("attachment:").and_then(data_url) {
                e.text.replace_range(span, &url);
            }
        }
    }
    for image in events.iter_mut().flat_map(|e| e.images.iter_mut()) {
        let Some(id) = image.strip_prefix("attachment:") else { continue };
        let attachment = bundle.attachments.iter().find(|a| a.id == id);
        *image = match attachment.and_then(|a| Some((a.mime.as_deref()?, a.data.as_deref()?))) {
            Some((mime, data)) => format!("data:{mime};base64,{data}"),
            None if attachment.and_then(|a| a.name.as_deref()).is_some_and(is_remote) => "omitted:remote".into(),
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
    pub has_details: bool,
    pub annotations: usize,
    /// The instruction files, skills, agent definitions and MCP server
    /// instructions in the transcript, each once.
    pub context: Vec<ContextSource>,
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
                has_details: s.meta.is_some(),
                annotations: s.annotations.len(),
                // A document edited mid-session is listed once.
                context: s.events.iter().filter_map(|e| e.context.clone()).fold(Vec::new(), |mut list: Vec<ContextSource>, c| {
                    if !list.iter().any(|x| x.kind == c.kind && x.name == c.name) {
                        list.push(ContextSource { repeat: false, ..c });
                    }
                    list
                }),
            })
            .collect(),
        omitted: bundle.sessions.iter().flat_map(|s| s.omitted.iter().cloned()).collect(),
        attachments: bundle.attachments.iter().map(|a| Attachment { data: None, ..a.clone() }).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::image_link_spans;

    fn urls(text: &str) -> Vec<&str> {
        image_link_spans(text).into_iter().map(|r| &text[r]).collect()
    }

    #[test]
    fn finds_markdown_and_html_image_addresses() {
        assert_eq!(urls("a ![x](https://a/1.png) b <IMG alt=\"y\" src='https://a/2.png'>"), ["https://a/1.png", "https://a/2.png"]);
        assert_eq!(urls("<img/src=\"https://a/3.png\">"), ["https://a/3.png"]);
    }

    #[test]
    fn nested_matches_keep_only_the_outer_one() {
        let text = "<img src=\"https://a/x ![y](https://b.example/ééééééé)\">";
        assert_eq!(urls(text), ["https://a/x ![y](https://b.example/ééééééé)"]);
    }

    #[test]
    fn data_src_is_not_src() {
        assert_eq!(urls("<img data-src=\"https://a/lazy.png\" src=\"https://a/real.png\">"), ["https://a/real.png"]);
    }

    #[test]
    fn images_inside_code_are_left_alone() {
        let text = "use `![logo](https://a/l.png)` here\n```html\n<img src=\"https://a/f.png\">\n```\n![real](https://a/r.png)";
        assert_eq!(urls(text), ["https://a/r.png"]);
    }
}
