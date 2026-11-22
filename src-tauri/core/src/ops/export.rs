//! Exporting one session as a bundle: a preview per device, then one download of exactly the
//! bundle previewed. Ported from the app's `PendingExport` (src-tauri/src/lib.rs at 37c3c34).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{Ctx, DeviceId, Input, OpError, Output};
use crate::bundle;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportPreviewRequest {
    pub path: String,
    #[serde(default)]
    pub branch: Option<String>,
    /// Paths of the subagents to include; None includes all of them.
    #[serde(default)]
    pub subagents: Option<Vec<String>>,
    /// Increases with every preview the export sheet asks for; only the newest is kept.
    pub request: u64,
}

#[derive(Debug, Serialize)]
pub struct ExportPreview {
    /// Pass back to `export_session` to download the bundle previewed here.
    pub token: u64,
    pub summary: bundle::Summary,
    pub subagents: Vec<SubagentChoice>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubagentChoice {
    pub path: String,
    pub title: String,
    pub included: bool,
}

/// `export_session`'s request: the bundle file comes back as the raw body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSessionRequest {
    pub token: u64,
    pub exported_by: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExportCancelRequest {}

/// The bundle shown in one device's export sheet. Downloading takes this exact bundle, so what
/// the user reviewed is what leaves the machine. Previews can overlap and finish in any order, so
/// the sheet numbers its requests and only the newest one's bundle is kept.
#[derive(Default)]
#[expect(dead_code, reason = "used by the export handlers once they are implemented")]
pub(super) struct PendingExport {
    latest: u64,
    bundle: Option<(u64, bundle::Bundle)>,
}

#[cfg_attr(not(test), expect(dead_code, reason = "used by the export handlers once they are implemented"))]
impl PendingExport {
    fn begin(&mut self, _request: u64) {
        todo!("PendingExport::begin")
    }

    fn finish(&mut self, _token: u64, _built: bundle::Bundle) {
        todo!("PendingExport::finish")
    }

    fn take(&mut self, _token: u64) -> Option<bundle::Bundle> {
        todo!("PendingExport::take")
    }

    /// Returns a bundle whose download failed, so the user can try again.
    fn put_back(&mut self, _token: u64, _built: bundle::Bundle) {
        todo!("PendingExport::put_back")
    }

    fn cancel(&mut self) {
        todo!("PendingExport::cancel")
    }
}

/// Builds the bundle for a session into the caller's slot and describes it for the export sheet.
pub(super) fn export_preview(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &ExportPreviewRequest) -> Result<ExportPreview, OpError> {
    todo!("export_preview")
}

/// The caller's previewed bundle as a file (`RawData::Bytes`), once. NotFound when the caller has
/// no bundle for `token`; BadRequest without a name.
pub(super) fn export_session(_ctx: &Arc<Ctx>, _device: &DeviceId, _input: Input) -> Result<Output, OpError> {
    todo!("export_session")
}

/// Drops the caller's previewed bundle when the export sheet closes without saving.
pub(super) fn export_cancel(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &ExportCancelRequest) -> Result<(), OpError> {
    todo!("export_cancel")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle_titled(title: &str) -> bundle::Bundle {
        bundle::Bundle {
            format: bundle::FORMAT.into(),
            version: bundle::VERSION,
            exported_at: 0,
            exporter: title.into(),
            exported_by: None,
            sessions: Vec::new(),
            attachments: Vec::new(),
        }
    }

    #[test]
    fn a_preview_that_finishes_late_does_not_replace_a_newer_one() {
        let mut pending = PendingExport::default();
        let (first, second) = (1, 2);
        // Requests can also start out of order.
        pending.begin(second);
        pending.begin(first);
        pending.finish(second, bundle_titled("second"));
        pending.finish(first, bundle_titled("first"));
        assert!(pending.take(first).is_none());
        assert_eq!(pending.take(second).unwrap().exporter, "second");
    }

    #[test]
    fn a_failed_save_keeps_the_preview_and_a_cancelled_one_is_dropped() {
        let mut pending = PendingExport::default();
        let token = 1;
        pending.begin(token);
        pending.finish(token, bundle_titled("shown"));
        let taken = pending.take(token).unwrap();
        pending.put_back(token, taken);
        assert!(pending.take(token).is_some());

        let late = 2;
        pending.begin(late);
        pending.cancel();
        pending.finish(late, bundle_titled("late"));
        assert!(pending.take(late).is_none());
    }
}
