//! Reading one session: full-text search and the transcript with its images.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{Ctx, DeviceId, OpError};
use crate::index::SearchHit;
use crate::model::Event;

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

impl Ctx {
    /// The decoded image a `load_session` URL points at. NotFound for a generation no longer
    /// kept, an index out of range, or an image that wasn't a data URL.
    pub fn image(&self, _image: ImageRef) -> Result<Image, OpError> {
        todo!("Ctx::image")
    }
}

pub(super) fn search(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &SearchRequest) -> Result<Vec<SearchHit>, OpError> {
    todo!("search")
}

/// The transcript as the UI shows it. Each `data:` image becomes
/// `/api/v1/image/{generation}/{event}/{image}`, served by `Ctx::image`.
pub(super) fn load_session(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &LoadSessionRequest) -> Result<Vec<Event>, OpError> {
    todo!("load_session")
}
