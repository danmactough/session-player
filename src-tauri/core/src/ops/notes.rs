//! A session's metadata and annotations.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{Ctx, DeviceId, OpError};
use crate::annotations::{Annotation, SessionMeta, Summary};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionNotesRequest {
    pub key: String,
    pub path: String,
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionNotes {
    pub meta: Option<SessionMeta>,
    pub annotations: Vec<Annotation>,
    /// The transcript's current revision, set when anything was generated, so the UI can mark
    /// suggestions made from an older one.
    pub revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveMetaRequest {
    pub meta: SessionMeta,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveAnnotationRequest {
    pub annotation: Annotation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteAnnotationRequest {
    pub id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NoteSummariesRequest {}

/// The metadata and annotations, with each annotation's anchors checked against the transcript at
/// `path` (and `branch`).
pub(super) fn session_notes(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &SessionNotesRequest) -> Result<SessionNotes, OpError> {
    todo!("session_notes")
}

/// Saves what a person edited and returns the metadata as stored (None when it was emptied).
pub(super) fn save_meta(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &SaveMetaRequest) -> Result<Option<SessionMeta>, OpError> {
    todo!("save_meta")
}

pub(super) fn save_annotation(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &SaveAnnotationRequest) -> Result<Annotation, OpError> {
    todo!("save_annotation")
}

pub(super) fn delete_annotation(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &DeleteAnnotationRequest) -> Result<(), OpError> {
    todo!("delete_annotation")
}

/// Titles, outcomes, tags and annotation kinds for the session list.
pub(super) fn note_summaries(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &NoteSummariesRequest) -> Result<Vec<Summary>, OpError> {
    todo!("note_summaries")
}
