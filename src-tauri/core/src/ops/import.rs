//! Moving sessions in and out: a bundle upload, a whole project's sync stream, and reading such a
//! stream into this library.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{Ctx, DeviceId, Input, OpError, Output};
use crate::sync;

/// The most bytes an uploaded bundle may hold unless `OpenParams.upload_cap` says otherwise.
pub const DEFAULT_UPLOAD_CAP: u64 = 512 * 1024 * 1024;

/// `export_project`'s request: the sync stream comes back as the raw body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportProjectRequest {
    /// A repository name as `sync::repo_name` gives it.
    pub repo: String,
    /// Read as `sp --since` reads it.
    #[serde(default)]
    pub since: Option<String>,
}

pub struct ImportStreamParams<'a> {
    /// The local checkout the sessions are listed under.
    pub project: PathBuf,
    /// Report what the import would do, writing nothing.
    pub dry_run: bool,
    pub input: &'a mut dyn Read,
}

/// Reads an upload of a bundle file (query `on_conflict=replace|keep`) and imports it into the
/// library, counting the bytes actually read against the cap. Over the cap is TooLarge and leaves
/// nothing behind. With conflicts and no `on_conflict`, nothing is written and the conflicts are
/// returned.
pub(super) fn import_bundle(_ctx: &Arc<Ctx>, _device: &DeviceId, _input: Input) -> Result<Output, OpError> {
    todo!("import_bundle")
}

/// Streams the repository's sessions (`RawData::Stream`) from a producer thread with its own
/// database connection. Dropping the reader stops the producer.
pub(super) fn export_project(_ctx: &Arc<Ctx>, _device: &DeviceId, _input: Input) -> Result<Output, OpError> {
    todo!("export_project")
}

/// Reads what `export_project` wrote into this library, then indexes, so the sessions are listed
/// when it returns. A dry run stores nothing.
pub fn import_stream(_ctx: &Arc<Ctx>, _params: ImportStreamParams<'_>) -> Result<sync::PullSummary, OpError> {
    todo!("import_stream")
}
