//! Session metadata and annotations that people (or a model) add to a
//! session. They live in the app's own tables, never in the agent's files,
//! and the index rebuild leaves them alone. Sessions are identified by a key
//! that survives re-indexing: "<local|bundle>:<source>:<native id>", plus
//! "@<leaf>" for a pi branch.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::model::{Event, EventKind, SessionRow};

/// Creates the tables if needed. Called whenever the database is opened.
pub fn create_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_meta (key TEXT PRIMARY KEY, json TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS annotations (
            id TEXT PRIMARY KEY, key TEXT NOT NULL, json TEXT NOT NULL);
         CREATE INDEX IF NOT EXISTS annotations_key ON annotations(key);
         CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
            key UNINDEXED, item UNINDEXED, text, tokenize='porter unicode61');",
    )
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    #[default]
    Unknown,
    Succeeded,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub url: String,
    pub label: Option<String>,
}

/// What a person has written about a session. Every field is optional; the
/// transcript's own title and details stay on the session row.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionMeta {
    pub key: String,
    pub title: Option<String>,
    pub goal: Option<String>,
    pub context: Option<String>,
    pub outcome: Outcome,
    pub outcome_note: Option<String>,
    pub tags: Vec<String>,
    pub links: Vec<Link>,
    pub updated_at: i64,
}

impl SessionMeta {
    fn is_empty(&self) -> bool {
        let blank = |s: &Option<String>| s.as_deref().is_none_or(|s| s.trim().is_empty());
        blank(&self.title)
            && blank(&self.goal)
            && blank(&self.context)
            && blank(&self.outcome_note)
            && self.outcome == Outcome::Unknown
            && self.tags.is_empty()
            && self.links.is_empty()
    }

    fn search_text(&self) -> String {
        let mut parts: Vec<&str> = [&self.title, &self.goal, &self.context, &self.outcome_note]
            .into_iter()
            .filter_map(|s| s.as_deref())
            .collect();
        parts.extend(self.tags.iter().map(String::as_str));
        parts.extend(self.links.iter().flat_map(|l| [Some(l.url.as_str()), l.label.as_deref()]).flatten());
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

/// Saves the metadata for `meta.key`. Metadata with nothing filled in is
/// removed rather than stored.
pub fn save_meta(conn: &Connection, meta: &SessionMeta) -> anyhow::Result<()> {
    // Links are shown as clickable; only web addresses open in the browser.
    if let Some(bad) = meta.links.iter().find(|l| !(l.url.starts_with("https://") || l.url.starts_with("http://"))) {
        anyhow::bail!("{} isn't a web address (http:// or https://)", bad.url);
    }
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM session_meta WHERE key=?1", [&meta.key])?;
    tx.execute("DELETE FROM notes_fts WHERE key=?1 AND item='meta'", [&meta.key])?;
    if !meta.is_empty() {
        let mut meta = meta.clone();
        meta.tags.retain(|t| !t.trim().is_empty());
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Set by `resolve`: whether the event is in the transcript as loaded.
    #[serde(default)]
    pub found: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    /// Empty until saved; never changes after that.
    pub id: String,
    pub key: String,
    pub kind: Kind,
    pub note: String,
    /// No anchors means the annotation is about the whole session.
    pub anchors: Vec<EventRef>,
    pub author: Author,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Annotation {
    pub fn new(key: &str, kind: Kind, note: &str, anchors: Vec<EventRef>, author: Author) -> Annotation {
        Annotation { id: String::new(), key: key.into(), kind, note: note.into(), anchors, author, created_at: 0, updated_at: 0 }
    }
}

/// Unique across machines, so annotations from different people can be
/// merged later without clashing.
fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let random = std::collections::hash_map::RandomState::new().build_hasher().finish();
    format!("{:x}-{random:016x}", now())
}

/// Creates the annotation (when `id` is empty) or updates the saved one,
/// returning it as stored.
pub fn save_annotation(conn: &Connection, mut a: Annotation) -> anyhow::Result<Annotation> {
    if a.note.trim().is_empty() && a.anchors.is_empty() {
        anyhow::bail!("An annotation needs a note or an event to point at");
    }
    let tx = conn.unchecked_transaction()?;
    if a.id.is_empty() {
        a.id = new_id();
        a.created_at = now();
        a.updated_at = a.created_at;
    } else {
        let existing: Option<String> = tx.query_row("SELECT json FROM annotations WHERE id=?1", [&a.id], |r| r.get(0)).optional()?;
        let Some(existing) = existing else { anyhow::bail!("This annotation no longer exists") };
        let existing: Annotation = serde_json::from_str(&existing)?;
        a.created_at = existing.created_at;
        a.key = existing.key;
        a.updated_at = now().max(a.created_at);
    }
    for r in &mut a.anchors {
        r.found = false;
    }
    tx.execute(
        // An upsert keeps the row's rowid, which orders annotations made in the same millisecond.
        "INSERT INTO annotations(id, key, json) VALUES(?1, ?2, ?3) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
        params![a.id, a.key, serde_json::to_string(&a)?],
    )?;
    tx.execute("DELETE FROM notes_fts WHERE item=?1", [&a.id])?;
    tx.execute("INSERT INTO notes_fts(key, item, text) VALUES(?1, ?2, ?3)", params![a.key, a.id, a.note])?;
    tx.commit()?;
    Ok(a)
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

pub fn delete_annotation(conn: &Connection, id: &str) -> anyhow::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM annotations WHERE id=?1", [id])?;
    tx.execute("DELETE FROM notes_fts WHERE item=?1", [id])?;
    tx.commit()?;
    Ok(())
}

/// Marks which anchors point at an event in `events`. An anchor whose event
/// has moved (a parser change renumbered the transcript) is moved to the
/// first event with the same kind and timestamp; one with no match is left
/// as it is, unfound, so the annotation is shown as unanchored, not lost.
pub fn resolve(list: &mut [Annotation], events: &[Event]) {
    let by_anchor: HashMap<&str, &Event> = events.iter().filter_map(|e| Some((e.anchor.as_deref()?, e))).collect();
    for r in list.iter_mut().flat_map(|a| a.anchors.iter_mut()) {
        let same = |e: &Event| e.kind == r.kind && e.ts == r.ts;
        if by_anchor.get(r.anchor.as_str()).is_some_and(|e| same(e)) {
            r.found = true;
        } else if let Some(moved) = events.iter().filter(|_| r.ts.is_some()).find(|e| same(e)).and_then(|e| e.anchor.clone()) {
            r.anchor = moved;
            r.found = true;
        } else {
            r.found = false;
        }
    }
}

/// The key metadata and annotations are stored under for a session row,
/// or for one of its pi branches.
pub fn session_key(row: &SessionRow, branch: Option<&str>) -> String {
    let origin = if row.from_bundle { "bundle" } else { "local" };
    match branch {
        Some(leaf) => format!("{origin}:{}:{}@{leaf}", row.source, row.native_id),
        None => format!("{origin}:{}:{}", row.source, row.native_id),
    }
}

/// What the session list needs to show and filter by.
#[derive(Debug, Serialize)]
pub struct Summary {
    pub key: String,
    pub title: Option<String>,
    pub outcome: Outcome,
    pub tags: Vec<String>,
    /// The kinds of the session's annotations, each once.
    pub kinds: Vec<Kind>,
}

pub fn summaries(conn: &Connection) -> anyhow::Result<Vec<Summary>> {
    let mut out: HashMap<String, Summary> = HashMap::new();
    let blank = |key: &str| Summary { key: key.to_string(), title: None, outcome: Outcome::Unknown, tags: Vec::new(), kinds: Vec::new() };
    let mut stmt = conn.prepare("SELECT json FROM session_meta")?;
    for json in stmt.query_map([], |r| r.get::<_, String>(0))? {
        let meta: SessionMeta = serde_json::from_str(&json?)?;
        let s = out.entry(meta.key.clone()).or_insert_with(|| blank(&meta.key));
        s.title = meta.title.filter(|t| !t.trim().is_empty());
        s.outcome = meta.outcome;
        s.tags = meta.tags;
    }
    let mut stmt = conn.prepare("SELECT json FROM annotations")?;
    for json in stmt.query_map([], |r| r.get::<_, String>(0))? {
        let a: Annotation = serde_json::from_str(&json?)?;
        let s = out.entry(a.key.clone()).or_insert_with(|| blank(&a.key));
        if !s.kinds.contains(&a.kind) {
            s.kinds.push(a.kind);
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
