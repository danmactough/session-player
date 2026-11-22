//! The UI's settings, the frontend's log line, and library maintenance.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{Ctx, DeviceId, OpError};
use crate::index::LibraryError;

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

pub(super) fn get_setting(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &GetSettingRequest) -> Result<Option<String>, OpError> {
    todo!("get_setting")
}

pub(super) fn set_setting(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &SetSettingRequest) -> Result<(), OpError> {
    todo!("set_setting")
}

/// Appends one line to `SERVER_LOG`: the message cut to `LOG_FRONTEND_MAX` with line breaks
/// escaped, so a paired device can't forge the server's own log lines.
pub(super) fn log_frontend(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &LogFrontendRequest) -> Result<(), OpError> {
    todo!("log_frontend")
}

pub(super) fn library_errors(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &LibraryErrorsRequest) -> Result<Vec<LibraryError>, OpError> {
    todo!("library_errors")
}

/// Deletes an imported session (and the subagents imported with it) from the library, then
/// re-indexes.
pub(super) fn remove_imported(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &RemoveImportedRequest) -> Result<(), OpError> {
    todo!("remove_imported")
}
