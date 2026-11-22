//! The `pull` operation the client crate adds to an in-process Ctx: reads a project's sessions
//! from a paired server (`export_project` over pinned HTTPS) into this library with
//! `ops::import_stream`.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use session_player_core::ops::{self, Access, Body, Ctx, DeviceId, ExportProjectRequest, Extension, ImportStreamParams, Input, OpError, OpSpec, Output, RawData};
use session_player_core::sync;

use crate::{Client, RawCall, ServersStore};

pub const PULL: Extension = Extension { spec: OpSpec { name: "pull", body: Body::Json, inline: false, access: Access::Authenticated }, handler: pull };

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    /// A server's name in `servers.json` (the name `sp connect` saved it under).
    pub server: String,
    /// The local checkout the sessions are listed under; its repository name is what's asked for.
    pub project: String,
    /// Read as `sp --since` reads it.
    #[serde(default)]
    pub since: Option<String>,
    /// Report what the pull would do, writing nothing.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullChange {
    Added,
    Replaced,
    /// The copy here is newer, so it stays.
    KeptLocal,
    Unchanged,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullCounts {
    pub added: usize,
    pub replaced: usize,
    pub kept_local: usize,
    pub unchanged: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullFile {
    pub path: String,
    pub change: PullChange,
    /// False for a file an agent keeps beside a session.
    pub is_session: bool,
}

/// `sync::PullSummary` on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullResponse {
    pub exporter: String,
    pub dry_run: bool,
    pub files: Vec<PullFile>,
    pub sessions: PullCounts,
    pub analyses: PullCounts,
    pub annotations: PullCounts,
    pub responses: PullCounts,
    pub reports_added: usize,
}

fn pull(ctx: &Arc<Ctx>, _device: &DeviceId, input: Input) -> Result<Output, OpError> {
    let Input::Json(value) = input else { return Err(OpError::BadRequest("pull takes a JSON body".to_string())) };
    let req: PullRequest = serde_json::from_value(value).map_err(|e| OpError::BadRequest(e.to_string()))?;
    let internal = |e: anyhow::Error| OpError::Internal(format!("{e:#}"));
    let server = ServersStore::new(&ctx.paths().data)
        .get(&req.server)
        .map_err(internal)?
        .ok_or_else(|| OpError::NotFound(format!("No server is named {}. Pair with it first: sp connect <pairing string>", req.server)))?;
    let project = PathBuf::from(&req.project);
    let repo = sync::repo_name(&project).ok_or_else(|| OpError::BadRequest(format!("{} has no repository name", project.display())))?;
    let client = Client::paired(&server).map_err(internal)?;
    let body = serde_json::to_vec(&ExportProjectRequest { repo: repo.0, since: req.since }).map_err(|e| OpError::Internal(e.to_string()))?;
    let call = RawCall { name: "export_project".to_string(), body: RawData::Bytes(body), content_type: "application/json".to_string(), query: Vec::new() };
    let exported = client.call_raw(call).map_err(|e| OpError::Internal(format!("{} answered: {}", server.name, e.message)))?;
    let mut input: Box<dyn Read + Send> = match exported.data {
        RawData::Stream(reader) => reader,
        RawData::Bytes(bytes) => Box::new(std::io::Cursor::new(bytes)),
    };
    let summary = ops::import_stream(ctx, ImportStreamParams { project, dry_run: req.dry_run, input: &mut input })?;
    let response = PullResponse {
        exporter: summary.exporter,
        dry_run: req.dry_run,
        files: summary.files.into_iter().map(|f| PullFile { path: f.path.to_string_lossy().into_owned(), change: change(f.change), is_session: f.is_session }).collect(),
        sessions: counts(summary.sessions),
        analyses: counts(summary.analyses),
        annotations: counts(summary.annotations),
        responses: counts(summary.responses),
        reports_added: summary.reports_added,
    };
    serde_json::to_value(response).map(Output::Json).map_err(|e| OpError::Internal(e.to_string()))
}

fn change(change: sync::Change) -> PullChange {
    match change {
        sync::Change::Added => PullChange::Added,
        sync::Change::Replaced => PullChange::Replaced,
        sync::Change::KeptLocal => PullChange::KeptLocal,
        sync::Change::Unchanged => PullChange::Unchanged,
    }
}

fn counts(counts: sync::Counts) -> PullCounts {
    PullCounts { added: counts.added, replaced: counts.replaced, kept_local: counts.kept_local, unchanged: counts.unchanged }
}
