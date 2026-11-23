//! Reading one session: full-text search and the transcript with its images.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use base64::Engine;
use serde::{Deserialize, Serialize};

use super::{Ctx, DeviceId, OpError, internal};
use crate::index::{self, SearchHit};
use crate::model::{Event, Source};
use crate::{bundle, parse};

/// How many sessions' images are kept, so two clients loading sessions don't invalidate each
/// other's image URLs.
pub const IMAGE_SETS: usize = 4;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadSessionRequest {
    pub path: String,
    /// A pi branch's leaf entry.
    #[serde(default)]
    pub branch: Option<String>,
}

/// The three numbers of an image URL `/api/v1/image/{generation}/{event}/{image}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRef {
    pub generation: u64,
    pub event: usize,
    pub image: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// The data URL's own media type, such as "image/png".
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// The data URLs of the most recently loaded sessions, by generation: for each event, its images
/// (an empty string for one that wasn't a data URL).
pub(super) struct ImageCache {
    next: u64,
    sets: VecDeque<(u64, Vec<Vec<String>>)>,
}

impl ImageCache {
    pub(super) fn new(first: u64) -> ImageCache {
        ImageCache { next: first, sets: VecDeque::new() }
    }
}

impl Ctx {
    /// The decoded image a `load_session` URL points at. NotFound for a generation no longer
    /// kept, an index out of range, or an image that wasn't a data URL.
    pub fn image(&self, image: ImageRef) -> Result<Image, OpError> {
        let not_found = || OpError::NotFound("No such image".to_string());
        let cache = self.images.lock().unwrap();
        let (_, set) = cache.sets.iter().find(|(g, _)| *g == image.generation).ok_or_else(not_found)?;
        let data_url = set.get(image.event).and_then(|e| e.get(image.image)).ok_or_else(not_found)?;
        let (head, b64) = data_url.split_once(";base64,").ok_or_else(not_found)?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|_| not_found())?;
        Ok(Image { content_type: head.trim_start_matches("data:").to_string(), bytes })
    }
}

pub(super) fn search(ctx: &Arc<Ctx>, _device: &DeviceId, req: &SearchRequest) -> Result<Vec<SearchHit>, OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    index::search(&conn, &req.query).map_err(internal)
}

/// The transcript as the UI shows it. Each `data:` image becomes
/// `/api/v1/image/{generation}/{event}/{image}`, served by `Ctx::image`.
pub(super) fn load_session(ctx: &Arc<Ctx>, _device: &DeviceId, req: &LoadSessionRequest) -> Result<Vec<Event>, OpError> {
    let mut events = session_events(&req.path, req.branch.as_deref()).map_err(internal)?;
    let mut cache = ctx.images.lock().unwrap();
    let generation = cache.next;
    cache.next += 1;
    let mut images = Vec::with_capacity(events.len());
    for (i, e) in events.iter_mut().enumerate() {
        let mut kept = Vec::new();
        for (j, img) in e.images.iter_mut().enumerate() {
            if img.starts_with("data:") {
                kept.push(std::mem::replace(img, format!("/api/v1/image/{generation}/{i}/{j}")));
            } else {
                kept.push(String::new());
            }
        }
        images.push(kept);
    }
    cache.sets.push_back((generation, images));
    while cache.sets.len() > IMAGE_SETS {
        cache.sets.pop_front();
    }
    Ok(events)
}

/// The transcript as the UI shows it: a library bundle, a session file, or one branch of a pi
/// session.
pub(super) fn session_events(path: &str, branch: Option<&str>) -> anyhow::Result<Vec<Event>> {
    let p = Path::new(path);
    if bundle::is_bundle_path(p) {
        return bundle::load_events(p);
    }
    let source = parse::detect(p).context("Unrecognized session format")?;
    let parsed = match (branch, source) {
        (Some(leaf), Source::Pi) => parse::pi::parse_branch(p, Some(leaf)),
        _ => parse::parse_file(p, source),
    };
    Ok(parsed?.events)
}
