//! A session's metadata and annotations.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{Ctx, DeviceId, OpError, internal, sessions};
use crate::annotations::{self, Annotation, SessionMeta, Summary};
use crate::model::Source;
use crate::{bundle, generate, index, parse};

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
pub(super) fn session_notes(ctx: &Arc<Ctx>, _device: &DeviceId, req: &SessionNotesRequest) -> Result<SessionNotes, OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    let meta = annotations::meta(&conn, &req.key).map_err(internal)?;
    let mut list = annotations::annotations(&conn, &req.key).map_err(internal)?;
    let generated = meta.as_ref().is_some_and(|m| m.generated_by.is_some()) || list.iter().any(|a| a.revision.is_some());
    let mut revision = None;
    if generated || list.iter().any(|a| !a.anchors.is_empty()) {
        let events = sessions::session_events(&req.path, req.branch.as_deref()).map_err(internal)?;
        if generated {
            revision = Some(generate::revision(&events));
        }
        // pi annotations made on another branch of the file are told apart from lost ones.
        let p = Path::new(&req.path);
        let entries = match parse::detect(p) {
            Some(Source::Pi) if !bundle::is_bundle_path(p) => parse::pi::entry_ids(p).ok(),
            _ => None,
        };
        annotations::resolve(&mut list, &events, entries.as_ref());
    }
    Ok(SessionNotes { meta, annotations: list, revision })
}

/// Saves what a person edited and returns the metadata as stored (None when it was emptied).
pub(super) fn save_meta(ctx: &Arc<Ctx>, _device: &DeviceId, req: &SaveMetaRequest) -> Result<Option<SessionMeta>, OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    annotations::save_meta(&conn, &req.meta).map_err(internal)?;
    annotations::meta(&conn, &req.meta.key).map_err(internal)
}

pub(super) fn save_annotation(ctx: &Arc<Ctx>, _device: &DeviceId, req: &SaveAnnotationRequest) -> Result<Annotation, OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    annotations::save_annotation(&conn, req.annotation.clone()).map_err(internal)
}

pub(super) fn delete_annotation(ctx: &Arc<Ctx>, _device: &DeviceId, req: &DeleteAnnotationRequest) -> Result<(), OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    annotations::delete_annotation(&conn, &req.id).map_err(internal)
}

/// Titles, outcomes, tags and annotation kinds for the session list.
pub(super) fn note_summaries(ctx: &Arc<Ctx>, _device: &DeviceId, _req: &NoteSummariesRequest) -> Result<Vec<Summary>, OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    annotations::summaries(&conn).map_err(internal)
}
