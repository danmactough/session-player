//! The UI's settings, the frontend's log line, and library maintenance.

use std::path::Path;
use std::sync::Arc;

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use super::{Ctx, DeviceId, OpError, index_now, internal};
use crate::index::{self, LibraryError};
use crate::{bundle, runlog};

/// The longest setting key accepted, in characters.
pub const SETTING_KEY_MAX: usize = 200;

/// The longest frontend log message written, in characters; the rest is cut.
pub const LOG_FRONTEND_MAX: usize = 2_000;

/// The file in `Paths::logs` that `log_frontend` appends to.
pub const SERVER_LOG: &str = "server.log";

/// `key` is bare on the wire; it is stored as `ui.<key>`, so the index's own settings
/// (`seeded`, `index_version`) can't be reached.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetSettingRequest {
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetSettingRequest {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogFrontendRequest {
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LibraryErrorsRequest {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoveImportedRequest {
    pub path: String,
}

/// The key as stored, or BadRequest naming the key (never the value).
fn stored_key(key: &str) -> Result<String, OpError> {
    if key.is_empty() {
        return Err(OpError::BadRequest("A setting needs a key".to_string()));
    }
    if key.chars().count() > SETTING_KEY_MAX {
        return Err(OpError::BadRequest(format!("The setting key {key:?} is longer than {SETTING_KEY_MAX} characters")));
    }
    if key.chars().any(char::is_control) {
        return Err(OpError::BadRequest(format!("The setting key {key:?} holds a control character")));
    }
    Ok(format!("ui.{key}"))
}

pub(super) fn get_setting(ctx: &Arc<Ctx>, _device: &DeviceId, req: &GetSettingRequest) -> Result<Option<String>, OpError> {
    let key = stored_key(&req.key)?;
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    conn.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0)).optional().map_err(|e| internal(e.into()))
}

pub(super) fn set_setting(ctx: &Arc<Ctx>, _device: &DeviceId, req: &SetSettingRequest) -> Result<(), OpError> {
    let key = stored_key(&req.key)?;
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    conn.execute("INSERT OR REPLACE INTO settings(key, value) VALUES(?1, ?2)", [key, req.value.clone()]).map_err(|e| internal(e.into()))?;
    Ok(())
}

/// Appends one line to `SERVER_LOG`: the message cut to `LOG_FRONTEND_MAX` with line breaks
/// escaped, so a paired device can't forge the server's own log lines.
pub(super) fn log_frontend(ctx: &Arc<Ctx>, _device: &DeviceId, req: &LogFrontendRequest) -> Result<(), OpError> {
    let escape = |text: &str, max: usize| -> String {
        text.chars().take(max).map(|c| if c.is_control() { c.escape_default().to_string() } else { c.to_string() }).collect()
    };
    let line = format!("[webview:{}] {}", escape(&req.level, 20), escape(&req.message, LOG_FRONTEND_MAX));
    runlog::append(&ctx.paths.logs.join(SERVER_LOG), &line);
    Ok(())
}

pub(super) fn library_errors(ctx: &Arc<Ctx>, _device: &DeviceId, _req: &LibraryErrorsRequest) -> Result<Vec<LibraryError>, OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    index::library_errors(&conn, &ctx.paths.library).map_err(internal)
}

/// Deletes an imported session (and the subagents imported with it) from the library, then
/// re-indexes.
pub(super) fn remove_imported(ctx: &Arc<Ctx>, device: &DeviceId, req: &RemoveImportedRequest) -> Result<(), OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    bundle::remove(&conn, &ctx.paths.library, Path::new(&req.path)).map_err(internal)?;
    drop(conn);
    index_now(ctx, device)?;
    Ok(())
}
