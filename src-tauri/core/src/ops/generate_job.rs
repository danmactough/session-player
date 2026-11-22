//! Analyze for one session: `claude` writes its metadata and annotations, as a job.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{Ctx, DeviceId, JobId, OpError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerateNotesRequest {
    pub key: String,
    pub path: String,
    #[serde(default)]
    pub branch: Option<String>,
    /// The user's own focus text; None uses the built-in one.
    #[serde(default)]
    pub focus: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CancelGenerateRequest {
    pub key: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GeneratingRequest {}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DefaultPromptsRequest {}

/// The built-in focus text for Analyze and for reports, which the user can replace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DefaultPrompts {
    pub analyze: String,
    pub report: String,
}

/// Starts a Generate job for the session. Progress arrives as generate-progress events and the
/// end as one generate-done event, published before the job leaves Running. Conflict while a
/// job for the same key runs.
pub(super) fn generate_notes(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &GenerateNotesRequest) -> Result<JobId, OpError> {
    todo!("generate_notes")
}

/// Cancels the running Generate job for `key` and kills its `claude`.
pub(super) fn cancel_generate(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &CancelGenerateRequest) -> Result<(), OpError> {
    todo!("cancel_generate")
}

/// Keys of the sessions a Generate job is running for.
pub(super) fn generating(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &GeneratingRequest) -> Result<Vec<String>, OpError> {
    todo!("generating")
}

pub(super) fn default_prompts(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &DefaultPromptsRequest) -> Result<DefaultPrompts, OpError> {
    todo!("default_prompts")
}
