//! Types that cross the wire: what the server sends and receives, and what the client reads back.

use serde::{Deserialize, Serialize};

use crate::index::IndexStats;

pub const API_VERSION: u32 = 1;
pub const API_MINOR: u32 = 0;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HelloRequest {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloResponse {
    pub api_version: u32,
    pub api_minor: u32,
    pub server_version: String,
    pub home: String,
    pub loopback_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Unauthenticated,
    ForbiddenOrigin,
    UnknownOperation,
    NotFound,
    MethodNotAllowed,
    Conflict,
    TooLarge,
    BadRequest,
    Internal,
}

impl ErrorCode {
    /// The HTTP status the server answers with.
    pub fn status(self) -> u16 {
        todo!()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OpError {
    Unauthenticated,
    /// No operation has this name. The server answers `unknown_operation` (404).
    UnknownOperation(String),
    BadRequest(String),
    NotFound(String),
    Conflict(String),
    TooLarge,
    Internal(String),
}

/// Who is calling: `"local"` for the local token, random hex for a paired device.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(pub String);

/// `j<start-ms>-<n>`, so ids from two runs of the server never collide.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Index,
    Generate,
    Analyze,
    Report,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Finished,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: JobId,
    pub kind: JobKind,
    pub state: JobState,
    /// The session key of a generate job.
    pub key: Option<String>,
    pub progress: Option<Progress>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerateProgress {
    pub key: String,
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerateDone {
    pub key: String,
    pub error: Option<String>,
    pub cancelled: bool,
    /// Suggested annotations dropped because they pointed at events that don't exist.
    pub rejected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportProgress {
    pub job: String,
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportDone {
    pub job: String,
    /// The saved report, when it was written.
    pub report: Option<String>,
    pub error: Option<String>,
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyzeProgress {
    pub job: JobId,
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyzeDone {
    pub job: JobId,
    pub analyzed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub cancelled: bool,
}

/// What the app's frontend used to receive through Tauri's `emit`, under the same event names.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "name", content = "payload", rename_all = "kebab-case")]
pub enum PushedEvent {
    IndexStarted,
    IndexProgress(Progress),
    IndexDone(IndexStats),
    IndexError(String),
    GenerateProgress(GenerateProgress),
    GenerateDone(GenerateDone),
    ReportProgress(ReportProgress),
    ReportDone(ReportDone),
    AnalyzeProgress(AnalyzeProgress),
    AnalyzeDone(AnalyzeDone),
    /// Sent in place of the events a caller missed: the jobs running now.
    Snapshot { running: Vec<Job> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerEvent {
    pub id: u64,
    pub at: u64,
    pub event: PushedEvent,
}

#[derive(Debug, Clone, Copy)]
pub struct EventsSince {
    /// The last id the caller has seen. None means "from now".
    pub after: Option<u64>,
    /// How long to wait for a newer event when there is none yet.
    pub wait: std::time::Duration,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobRequest {
    pub id: JobId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CancelJobRequest {
    pub id: JobId,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListSessionsRequest {
    /// A repository name as `sync::repo_name` gives it.
    pub repo: Option<String>,
    /// Read as `sp --since` reads it: a date (2026-11-01), a date and time, or an age (7d).
    pub since: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReindexRequest {
    #[serde(default)]
    pub full: bool,
}
