//! The session folders the index reads.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{Ctx, DeviceId, OpError, PushedEvent, ReindexRequest, internal, reindex};
use crate::index;

/// Where the agents keep their sessions, under the home folder.
const AGENT_FOLDERS: [&str; 4] = [".claude/projects", ".codex/sessions", ".codex/archived_sessions", ".pi/agent/sessions"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootInfo {
    pub path: String,
    pub exists: bool,
    /// Sessions the index holds from this folder.
    pub sessions: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GetRootsRequest {}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SuggestedRootsRequest {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AddRootRequest {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoveRootRequest {
    pub path: String,
}

pub(super) fn get_roots(ctx: &Arc<Ctx>, _device: &DeviceId, _req: &GetRootsRequest) -> Result<Vec<RootInfo>, OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    let counts = index::root_counts(&conn).map_err(internal)?;
    Ok(index::roots(&conn)
        .map_err(internal)?
        .into_iter()
        .map(|path| RootInfo { exists: Path::new(&path).is_dir(), sessions: counts.get(&path).copied().unwrap_or(0), path })
        .collect())
}

/// The agents' session folders under the server's home that exist and aren't registered yet.
pub(super) fn suggested_roots(ctx: &Arc<Ctx>, _device: &DeviceId, _req: &SuggestedRootsRequest) -> Result<Vec<String>, OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    let existing = index::roots(&conn).map_err(internal)?;
    Ok(agent_folders(&ctx.home).into_iter().filter(|p| !existing.contains(p)).collect())
}

/// The agents' session folders under `home` that exist.
pub(super) fn agent_folders(home: &Path) -> Vec<String> {
    AGENT_FOLDERS.iter().map(|f| home.join(f)).filter(|p| p.is_dir()).map(|p| p.to_string_lossy().into_owned()).collect()
}

/// Registers the folder and starts an index pass that reads it.
pub(super) fn add_root(ctx: &Arc<Ctx>, device: &DeviceId, req: &AddRootRequest) -> Result<RootInfo, OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    index::add_root(&conn, &req.path).map_err(internal)?;
    let sessions = index::root_counts(&conn).map_err(internal)?.get(&req.path).copied().unwrap_or(0);
    drop(conn);
    reindex(ctx, device, &ReindexRequest { full: false })?;
    Ok(RootInfo { path: req.path.clone(), exists: Path::new(&req.path).is_dir(), sessions })
}

pub(super) fn remove_root(ctx: &Arc<Ctx>, _device: &DeviceId, req: &RemoveRootRequest) -> Result<(), OpError> {
    let mut conn = index::open(&ctx.paths.db).map_err(internal)?;
    index::remove_root(&mut conn, &req.path).map_err(internal)?;
    // The list changed without an index pass, so listeners are told to read it again.
    ctx.publish(PushedEvent::IndexDone(index::IndexStats::default()));
    Ok(())
}
