//! The `pull` operation the client crate adds to an in-process Ctx: reads a project's sessions
//! from a paired server (`export_project` over pinned HTTPS) into this library with
//! `ops::import_stream`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use session_player_core::ops::{Access, Body, Ctx, DeviceId, Extension, Input, OpError, OpSpec, Output};

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

fn pull(_ctx: &Arc<Ctx>, _device: &DeviceId, _input: Input) -> Result<Output, OpError> {
    todo!("pull a project from a paired server")
}
