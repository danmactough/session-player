//! Analyze for many sessions as one job.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{Ctx, DeviceId, JobId, OpError};

/// How many sessions an Analyze job works on at the same time.
pub const ANALYZE_AT_ONCE: usize = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyzeSessionsRequest {
    /// Session files as `list_sessions` gives them; sessions sharing a key are analyzed once.
    pub paths: Vec<String>,
    /// Analyze sessions again that already have an analysis.
    #[serde(default)]
    pub force: bool,
    /// The user's own focus text; None uses the one saved in the UI's settings, else the built-in.
    #[serde(default)]
    pub focus: Option<String>,
}

/// Starts an Analyze job: `ANALYZE_AT_ONCE` sessions at a time, skipping analyzed ones unless
/// `force`. It publishes analyze-progress and ends with one analyze-done holding the counts, before
/// leaving Running. Ends Failed when any session failed. Conflict while another Analyze job runs.
pub(super) fn analyze_sessions(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &AnalyzeSessionsRequest) -> Result<JobId, OpError> {
    todo!("analyze_sessions")
}
