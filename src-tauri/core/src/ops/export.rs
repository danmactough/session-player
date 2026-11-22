//! Exporting one session as a bundle: a preview per device, then one download of exactly the
//! bundle previewed. Ported from the app's `PendingExport` (src-tauri/src/lib.rs at 37c3c34).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{Ctx, DeviceId, Input, OpError, Output, RawBody, RawData, internal};
use crate::{bundle, index};

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
pub(super) struct PendingExport {
    latest: u64,
    bundle: Option<(u64, bundle::Bundle)>,
}

impl PendingExport {
    fn begin(&mut self, request: u64) {
        if request > self.latest {
            self.latest = request;
            self.bundle = None;
        }
    }

    fn finish(&mut self, token: u64, built: bundle::Bundle) {
        if token == self.latest {
            self.bundle = Some((token, built));
        }
    }

    fn take(&mut self, token: u64) -> Option<bundle::Bundle> {
        match self.bundle.take() {
            Some((t, built)) if t == token => Some(built),
            other => {
                self.bundle = other;
                None
            }
        }
    }

    /// Returns a bundle whose download failed, so the user can try again.
    fn put_back(&mut self, token: u64, built: bundle::Bundle) {
        if token == self.latest && self.bundle.is_none() {
            self.bundle = Some((token, built));
        }
    }

    fn cancel(&mut self) {
        self.latest += 1;
        self.bundle = None;
    }
}

/// Builds the bundle for a session into the caller's slot and describes it for the export sheet.
pub(super) fn export_preview(ctx: &Arc<Ctx>, device: &DeviceId, req: &ExportPreviewRequest) -> Result<ExportPreview, OpError> {
    let token = req.request;
    ctx.exports.lock().unwrap().entry(device.clone()).or_default().begin(token);
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    let choices = if req.branch.is_some() { Vec::new() } else { bundle::subagents_of(&conn, &req.path).map_err(internal)? };
    let request = bundle::ExportRequest { path: &req.path, branch: req.branch.as_deref(), subagents: req.subagents.clone() };
    let built = bundle::build(&conn, request, &format!("Session Player {}", ctx.version)).map_err(internal)?;
    let summary = bundle::summarize(&built);
    ctx.exports.lock().unwrap().entry(device.clone()).or_default().finish(token, built);
    Ok(ExportPreview {
        token,
        summary,
        subagents: choices
            .into_iter()
            .map(|(path, title)| SubagentChoice { included: req.subagents.as_ref().is_none_or(|wanted| wanted.contains(&path)), path, title })
            .collect(),
    })
}

/// The caller's previewed bundle as a file (`RawData::Bytes`), once. NotFound when the caller has
/// no bundle for `token`; BadRequest without a name.
pub(super) fn export_session(ctx: &Arc<Ctx>, device: &DeviceId, input: Input) -> Result<Output, OpError> {
    let Input::Json(value) = input else { return Err(OpError::BadRequest("export_session takes a JSON body".to_string())) };
    let req: ExportSessionRequest = serde_json::from_value(value).map_err(|e| OpError::BadRequest(e.to_string()))?;
    let exported_by = req.exported_by.trim().to_string();
    if exported_by.is_empty() {
        return Err(OpError::BadRequest("Set your name before exporting.".to_string()));
    }
    let taken = ctx.exports.lock().unwrap().get_mut(device).and_then(|slot| slot.take(req.token));
    let Some(mut built) = taken else {
        return Err(OpError::NotFound("This preview is out of date. Review the export again before saving.".to_string()));
    };
    built.exported_by = Some(exported_by);
    match serde_json::to_vec(&built) {
        Ok(bytes) => {
            let filename = bundle::summarize(&built).file_name;
            Ok(Output::Raw(RawBody { data: RawData::Bytes(bytes), content_type: "application/octet-stream".to_string(), filename: Some(filename) }))
        }
        Err(e) => {
            ctx.exports.lock().unwrap().entry(device.clone()).or_default().put_back(req.token, built);
            Err(OpError::Internal(e.to_string()))
        }
    }
}

/// Drops the caller's previewed bundle when the export sheet closes without saving.
pub(super) fn export_cancel(ctx: &Arc<Ctx>, device: &DeviceId, _req: &ExportCancelRequest) -> Result<(), OpError> {
    if let Some(slot) = ctx.exports.lock().unwrap().get_mut(device) {
        slot.cancel();
    }
    Ok(())
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
