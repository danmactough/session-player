//! Session metadata and annotations that people (or a model) add to a
//! session. They live in the app's own tables, never in the agent's files,
//! and the index rebuild leaves them alone. Sessions are identified by a key
//! that survives re-indexing: "<local|bundle>:<source>:<native id>", plus
//! "@<leaf>" for a pi branch.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::model::{Event, EventKind, SessionRow};

/// Creates the tables if needed. Called whenever the database is opened.
///
/// `annotations` holds each annotation's current version. Every save that
/// changes an annotation adds a row to `annotation_versions`, and a row there
/// never changes, so a report can cite an (id, version) and always find the
/// same text. A version is kept only while something refers to it: it is the
/// annotation's current version, or `report_citations` cites it. A deleted
/// annotation leaves `annotations` but keeps its cited versions.
pub fn create_tables(conn: &Connection) -> rusqlite::Result<()> {
    let versioned: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='annotation_versions')",
        [],
        |r| r.get(0),
    )?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_meta (key TEXT PRIMARY KEY, json TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS annotations (
            id TEXT PRIMARY KEY, key TEXT NOT NULL, json TEXT NOT NULL);
         CREATE INDEX IF NOT EXISTS annotations_key ON annotations(key);
         CREATE TABLE IF NOT EXISTS report_citations (
            report_id TEXT NOT NULL, annotation_id TEXT NOT NULL, version INTEGER NOT NULL,
            PRIMARY KEY (report_id, annotation_id, version));
         CREATE INDEX IF NOT EXISTS report_citations_annotation ON report_citations(annotation_id, version);
         CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
            key UNINDEXED, item UNINDEXED, text, tokenize='porter unicode61');",
    )?;
    if !versioned {
        // Annotations saved before versions existed become version 1. The
        // table is made in the same transaction, so a failed migration runs again.
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE annotation_versions (
                id TEXT NOT NULL, version INTEGER NOT NULL, key TEXT NOT NULL, json TEXT NOT NULL,
                PRIMARY KEY (id, version));
             UPDATE annotations SET json=json_set(json, '$.version', 1) WHERE json_extract(json, '$.version') IS NULL;
             INSERT OR IGNORE INTO annotation_versions(id, version, key, json)
                SELECT id, json_extract(json, '$.version'), key, json FROM annotations;
             COMMIT;",
        )?;
    }
    Ok(())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    #[default]
    Unknown,
    Succeeded,
    Partial,
    Failed,
}

/// The analysis of a session: how it turned out and which parts of the
/// repository it worked on. Every field is optional.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionMeta {
    pub key: String,
    pub outcome: Outcome,
    /// Why the session succeeded or didn't, in a sentence.
    pub outcome_note: Option<String>,
    /// Repository paths or packages worked on, such as "packages/api".
    pub components: Vec<String>,
    pub updated_at: i64,
    /// Fields ("outcome", "components") holding a model's suggestion that
    /// no one has edited yet.
    pub generated: Vec<String>,
    pub generated_by: Option<Generation>,
    /// Fields a person emptied on purpose; generating leaves them empty
    /// until someone fills them in again.
    pub cleared: Vec<String>,
}

/// Which model made the suggestions, when, and from which transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Generation {
    pub model: String,
    pub at: i64,
    /// `generate::revision` of the transcript; a different one now means the
    /// suggestions are out of date.
    pub revision: String,
}

/// The fields `SessionMeta::generated` and `cleared` can name, in display order.
pub const META_FIELDS: [&str; 2] = ["outcome", "components"];

pub fn field_is_empty(meta: &SessionMeta, name: &str) -> bool {
    match name {
        "outcome" => meta.outcome == Outcome::Unknown && meta.outcome_note.as_deref().is_none_or(|s| s.trim().is_empty()),
        "components" => meta.components.iter().all(|c| c.trim().is_empty()),
        _ => true,
    }
}

/// The value of one of the fields `SessionMeta::generated` can name. The
/// outcome's explanation goes with the outcome.
pub fn meta_field(meta: &SessionMeta, name: &str) -> serde_json::Value {
    match name {
        "outcome" => serde_json::json!([meta.outcome, meta.outcome_note]),
        "components" => serde_json::json!(meta.components),
        _ => serde_json::Value::Null,
    }
}

impl SessionMeta {
    fn is_empty(&self) -> bool {
        META_FIELDS.iter().all(|f| field_is_empty(self, f))
            // Kept so generating doesn't refill what a person emptied.
            && self.cleared.is_empty()
    }

    fn search_text(&self) -> String {
        let mut parts: Vec<&str> = self.outcome_note.as_deref().into_iter().collect();
        parts.extend(self.components.iter().map(String::as_str));
        parts.join("\n")
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn meta(conn: &Connection, key: &str) -> anyhow::Result<Option<SessionMeta>> {
    let json: Option<String> = conn
        .query_row("SELECT json FROM session_meta WHERE key=?1", [key], |r| r.get(0))
        .optional()?;
    Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
}

/// Saves metadata a person edited. A field stays marked as a suggestion
/// only if `meta.generated` still lists it and its value didn't change, so
/// editing a suggestion, or leaving it out of `generated`, accepts it.
pub fn save_meta(conn: &Connection, meta: &SessionMeta) -> anyhow::Result<()> {
    let existing = self::meta(conn, &meta.key)?;
    let mut meta = meta.clone();
    let edited = meta.clone();
    meta.generated.retain(|name| {
        existing
            .as_ref()
            .is_some_and(|old| old.generated.contains(name) && meta_field(old, name) == meta_field(&edited, name))
    });
    // Emptying a field records that; filling it lifts it.
    meta.cleared = existing.as_ref().map(|old| old.cleared.clone()).unwrap_or_default();
    for name in META_FIELDS {
        let filled = |m: &SessionMeta| !field_is_empty(m, name);
        if filled(&meta) {
            meta.cleared.retain(|c| c != name);
        } else if existing.as_ref().is_some_and(filled) && !meta.cleared.iter().any(|c| c == name) {
            meta.cleared.push(name.to_string());
        }
    }
    store_meta(conn, &meta)
}

/// Writes the metadata as given. Metadata with nothing filled in is
/// removed rather than stored.
pub fn store_meta(conn: &Connection, meta: &SessionMeta) -> anyhow::Result<()> {
    let mut meta = meta.clone();
    meta.components = meta.components.iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty() && c != ".").collect();
    meta.generated.retain(|f| META_FIELDS.contains(&f.as_str()));
    meta.cleared.retain(|f| META_FIELDS.contains(&f.as_str()));
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM session_meta WHERE key=?1", [&meta.key])?;
    tx.execute("DELETE FROM notes_fts WHERE key=?1 AND item='meta'", [&meta.key])?;
    if !meta.is_empty() {
        meta.updated_at = now();
        tx.execute("INSERT INTO session_meta(key, json) VALUES(?1, ?2)", params![meta.key, serde_json::to_string(&meta)?])?;
        tx.execute(
            "INSERT INTO notes_fts(key, item, text) VALUES(?1, 'meta', ?2)",
            params![meta.key, meta.search_text()],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// What an annotation marks. The list is a starting point (issue #11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    ToolError,
    /// A person corrected or redirected the agent.
    HumanIntervention,
    /// Looping, or retrying the same fix.
    AgentStuck,
    HumanFrustration,
    WrongAssumption,
    WastedWork,
    Environment,
    MissingContext,
    /// Repeated work worth automating.
    Automatable,
    KeyDecision,
    /// The agent went against an instruction file, skill or agent definition.
    IgnoredInstruction,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Author {
    Person { name: Option<String> },
    Model { model: String },
}

/// An event an annotation points at. The kind and timestamp let the anchor
/// be checked, and found again, when the transcript is parsed differently.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRef {
    pub anchor: String,
    pub kind: EventKind,
    pub ts: Option<i64>,
    /// The pi entry the event came from. pi renumbers events when the user
    /// rewinds, so its events are found by entry instead of by anchor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// For a context document: its name (path), since one pi entry or one
    /// Claude Code attachment carries several documents of the same kind
    /// and time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Set by `resolve`: whether the event is in the transcript as loaded.
    #[serde(default)]
    pub found: bool,
    /// Set by `resolve`: the event is on another branch of the pi session,
    /// so this view leaves the annotation out.
    #[serde(default)]
    pub elsewhere: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    /// Empty until saved; never changes after that.
    pub id: String,
    /// 0 until saved. Set by the database: each change saves a new version,
    /// so a value sent in from outside (the UI, a bundle) is ignored.
    #[serde(default, skip_serializing_if = "is_unsaved")]
    pub version: u32,
    pub key: String,
    pub kind: Kind,
    pub note: String,
    /// The events it is about; at least one.
    pub anchors: Vec<EventRef>,
    pub author: Author,
    pub created_at: i64,
    pub updated_at: i64,
    /// For a model's annotation: someone edited it, so regenerating keeps it.
    #[serde(default)]
    pub edited: bool,
    /// For a model's annotation: why it was made, and how sure the model was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
    /// For a model's annotation: the transcript revision it was made from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

fn is_unsaved(version: &u32) -> bool {
    *version == 0
}

impl Annotation {
    pub fn new(key: &str, kind: Kind, note: &str, anchors: Vec<EventRef>, author: Author) -> Annotation {
        Annotation {
            id: String::new(),
            version: 0,
            key: key.into(),
            kind,
            note: note.into(),
            anchors,
            author,
            created_at: 0,
            updated_at: 0,
            edited: false,
            rationale: None,
            confidence: None,
            revision: None,
        }
    }
}

/// Unique across machines, so annotations from different people can be
/// merged later without clashing.
pub fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let random = std::collections::hash_map::RandomState::new().build_hasher().finish();
    format!("{:x}-{random:016x}", now())
}

/// Creates the annotation (when `id` is empty) or updates the saved one,
/// returning it as stored. A change saves a new version; saving it unchanged
/// doesn't. Changing a model's annotation marks it as edited, so
/// regenerating keeps it.
pub fn save_annotation(conn: &Connection, mut a: Annotation) -> anyhow::Result<Annotation> {
    // An annotation marks a moment; how the session as a whole went is its metadata.
    if a.anchors.is_empty() {
        anyhow::bail!("An annotation needs an event to point at");
    }
    for r in &mut a.anchors {
        r.found = false;
        r.elsewhere = false;
    }
    let tx = conn.unchecked_transaction()?;
    if a.id.is_empty() {
        a.id = new_id();
        a.created_at = now();
        a.updated_at = a.created_at;
    } else {
        let Some(existing) = annotation(&tx, &a.id)? else { anyhow::bail!("This annotation no longer exists") };
        a.created_at = existing.created_at;
        a.key = existing.key.clone();
        a.author = existing.author.clone();
        a.version = existing.version;
        let same_anchors = a.anchors.len() == existing.anchors.len()
            && a.anchors.iter().zip(&existing.anchors).all(|(x, y)| x.anchor == y.anchor && x.kind == y.kind && x.ts == y.ts && x.entry == y.entry);
        let changed = a.kind != existing.kind || a.note != existing.note || !same_anchors;
        a.edited = existing.edited || (changed && matches!(a.author, Author::Model { .. }));
        a.updated_at = if changed { now().max(a.created_at) } else { existing.updated_at };
        if a == existing {
            return Ok(existing);
        }
    }
    write_version(&tx, &mut a)?;
    tx.commit()?;
    Ok(a)
}

/// Saves `a` as the annotation's next version and makes it current.
fn write_version(tx: &Connection, a: &mut Annotation) -> anyhow::Result<()> {
    // Counts deleted annotations' kept versions too, so a re-imported one doesn't reuse a number.
    a.version = tx.query_row("SELECT COALESCE(MAX(version), 0) + 1 FROM annotation_versions WHERE id=?1", [&a.id], |r| r.get(0))?;
    let json = serde_json::to_string(&a)?;
    tx.execute("INSERT INTO annotation_versions(id, version, key, json) VALUES(?1, ?2, ?3, ?4)", params![a.id, a.version, a.key, json])?;
    tx.execute(
        // An upsert keeps the row's rowid, which orders annotations made in the same millisecond.
        "INSERT INTO annotations(id, key, json) VALUES(?1, ?2, ?3) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
        params![a.id, a.key, json],
    )?;
    tx.execute("DELETE FROM notes_fts WHERE item=?1", [&a.id])?;
    tx.execute("INSERT INTO notes_fts(key, item, text) VALUES(?1, ?2, ?3)", params![a.key, a.id, a.note])?;
    drop_unused_versions(tx, &a.id)
}

/// Removes the annotation's versions that are neither current nor cited by a report.
pub(crate) fn drop_unused_versions(tx: &Connection, id: &str) -> anyhow::Result<()> {
    tx.execute(
        "DELETE FROM annotation_versions WHERE id=?1
           AND version IS NOT (SELECT json_extract(json, '$.version') FROM annotations WHERE id=?1)
           AND NOT EXISTS (SELECT 1 FROM report_citations c
                           WHERE c.annotation_id=?1 AND c.version=annotation_versions.version)",
        [id],
    )?;
    Ok(())
}

pub fn annotation(conn: &Connection, id: &str) -> anyhow::Result<Option<Annotation>> {
    let json: Option<String> = conn.query_row("SELECT json FROM annotations WHERE id=?1", [id], |r| r.get(0)).optional()?;
    Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
}

/// The session's annotations, oldest first.
pub fn annotations(conn: &Connection, key: &str) -> anyhow::Result<Vec<Annotation>> {
    let mut stmt =
        conn.prepare("SELECT json FROM annotations WHERE key=?1 ORDER BY json_extract(json, '$.created_at'), rowid")?;
    let list = stmt
        .query_map([key], |r| r.get::<_, String>(0))?
        .map(|j| Ok(serde_json::from_str::<Annotation>(&j?)?))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(list)
}

/// One version of an annotation, current or kept because a report cites it.
pub fn annotation_version(conn: &Connection, id: &str, version: u32) -> anyhow::Result<Option<Annotation>> {
    let json: Option<String> = conn
        .query_row("SELECT json FROM annotation_versions WHERE id=?1 AND version=?2", params![id, version], |r| r.get(0))
        .optional()?;
    Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
}

/// Stores an annotation as given (id, author and times included), for one
/// that arrives in an imported bundle. It becomes a new version when it
/// differs from the one stored; the bundle's version number is ignored.
pub fn put_annotation(conn: &Connection, a: &Annotation) -> anyhow::Result<()> {
    let mut a = a.clone();
    for r in &mut a.anchors {
        r.found = false;
        r.elsewhere = false;
    }
    let tx = conn.unchecked_transaction()?;
    if let Some(existing) = annotation(&tx, &a.id)? {
        // Never moves an annotation from one session to another.
        if existing.key != a.key {
            return Ok(());
        }
        a.version = existing.version;
        if a == existing {
            return Ok(());
        }
    }
    write_version(&tx, &mut a)?;
    tx.commit()?;
    Ok(())
}

/// Deletes the annotation. Versions a report cites are kept, so the report
/// can still show them; everything else about it is removed.
pub fn delete_annotation(conn: &Connection, id: &str) -> anyhow::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM annotations WHERE id=?1", [id])?;
    tx.execute("DELETE FROM notes_fts WHERE item=?1", [id])?;
    drop_unused_versions(&tx, id)?;
    tx.commit()?;
    Ok(())
}

/// Marks which anchors point at an event in `events`, the transcript as the
/// UI shows it. A pi event is found by its entry; `entries` lists every entry
/// in the file, so one on another branch is marked `elsewhere` rather than
/// lost. Any other anchor whose event has moved (a parser change renumbered
/// the transcript) is moved to the one event with the same kind and
/// timestamp. One with no match keeps its anchor, unfound, so the annotation
/// is shown as unanchored, not dropped.
pub fn resolve(list: &mut [Annotation], events: &[Event], entries: Option<&HashSet<String>>) {
    let by_anchor: HashMap<&str, &Event> = events.iter().filter_map(|e| Some((e.anchor.as_deref()?, e))).collect();
    for r in list.iter_mut().flat_map(|a| a.anchors.iter_mut()) {
        r.found = false;
        r.elsewhere = false;
        let named = |e: &Event| r.name.is_none() || e.context.as_ref().map(|c| &c.name) == r.name.as_ref();
        if let Some(entry) = r.entry.clone() {
            match events.iter().find(|e| e.entry.as_deref() == Some(entry.as_str()) && e.kind == r.kind && named(e)) {
                Some(e) => {
                    r.anchor = e.anchor.clone().unwrap_or_default();
                    r.found = true;
                }
                None => r.elsewhere = entries.is_some_and(|all| all.contains(&entry)),
            }
            continue;
        }
        let same = |e: &Event| e.kind == r.kind && e.ts == r.ts && named(e);
        if by_anchor.get(r.anchor.as_str()).is_some_and(|e| same(e)) {
            r.found = true;
            continue;
        }
        // Parallel tool calls can share a timestamp; only an unambiguous match moves the anchor.
        let mut matches = events.iter().filter(|e| r.ts.is_some() && same(e));
        if let (Some(e), None) = (matches.next(), matches.next())
            && let Some(anchor) = e.anchor.clone()
        {
            r.anchor = anchor;
            r.found = true;
        }
    }
}

/// The key metadata and annotations are stored under for a session row.
/// A local pi session's branches share it: a branch's leaf changes whenever
/// the user rewinds or continues it, so it can't identify notes. (An
/// imported pi branch never changes, and its native id carries its leaf.)
pub fn session_key(row: &SessionRow) -> String {
    let origin = if row.from_bundle { "bundle" } else { "local" };
    format!("{origin}:{}:{}", row.source, row.native_id)
}

/// What the session list needs to show and filter by, and what the
/// Analysis view adds up across the sessions shown.
#[derive(Debug, Default, Serialize)]
pub struct Summary {
    pub key: String,
    pub outcome: Outcome,
    pub components: Vec<String>,
    /// The kinds of the session's annotations, each once.
    pub kinds: Vec<Kind>,
    /// How many of the session's annotations have each kind.
    pub kind_counts: HashMap<Kind, usize>,
    pub person_annotations: usize,
    pub model_annotations: usize,
    /// The model's annotations by confidence ("low", "medium", "high").
    pub confidence: HashMap<String, usize>,
    /// Analyze has run on the session.
    pub analyzed: bool,
}

pub fn summaries(conn: &Connection) -> anyhow::Result<Vec<Summary>> {
    let mut out: HashMap<String, Summary> = HashMap::new();
    let blank = |key: &str| Summary { key: key.to_string(), ..Default::default() };
    let mut stmt = conn.prepare("SELECT json FROM session_meta")?;
    for json in stmt.query_map([], |r| r.get::<_, String>(0))? {
        let meta: SessionMeta = serde_json::from_str(&json?)?;
        let s = out.entry(meta.key.clone()).or_insert_with(|| blank(&meta.key));
        s.outcome = meta.outcome;
        s.components = meta.components;
        s.analyzed |= meta.generated_by.is_some();
    }
    let mut stmt = conn.prepare("SELECT json FROM annotations")?;
    for json in stmt.query_map([], |r| r.get::<_, String>(0))? {
        let a: Annotation = serde_json::from_str(&json?)?;
        let s = out.entry(a.key.clone()).or_insert_with(|| blank(&a.key));
        if !s.kinds.contains(&a.kind) {
            s.kinds.push(a.kind);
        }
        *s.kind_counts.entry(a.kind).or_default() += 1;
        match a.author {
            Author::Person { .. } => s.person_annotations += 1,
            Author::Model { .. } => {
                s.model_annotations += 1;
                s.analyzed = true;
                if let Some(c) = a.confidence {
                    *s.confidence.entry(c).or_default() += 1;
                }
            }
        }
    }
    let mut list: Vec<Summary> = out.into_values().collect();
    list.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(list)
}

/// Full-text matches in metadata and annotation notes, as (key, snippet, rank).
pub fn search(conn: &Connection, fts_query: &str) -> anyhow::Result<Vec<(String, String, f64)>> {
    let mut stmt = conn.prepare(
        "SELECT key, snippet(notes_fts, 2, '\u{1}', '\u{2}', ' … ', 14), bm25(notes_fts)
         FROM notes_fts WHERE notes_fts MATCH ?1 ORDER BY 3 LIMIT 500",
    )?;
    let hits = stmt
        .query_map([fts_query], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    Ok(hits)
}
