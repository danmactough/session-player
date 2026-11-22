//! The session folders the index reads.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{Ctx, DeviceId, OpError};

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

pub(super) fn get_roots(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &GetRootsRequest) -> Result<Vec<RootInfo>, OpError> {
    todo!("get_roots")
}

/// The agents' session folders under the server's home that exist and aren't registered yet.
pub(super) fn suggested_roots(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &SuggestedRootsRequest) -> Result<Vec<String>, OpError> {
    todo!("suggested_roots")
}

/// Registers the folder and starts an index pass that reads it.
pub(super) fn add_root(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &AddRootRequest) -> Result<RootInfo, OpError> {
    todo!("add_root")
}

pub(super) fn remove_root(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &RemoveRootRequest) -> Result<(), OpError> {
    todo!("remove_root")
}
