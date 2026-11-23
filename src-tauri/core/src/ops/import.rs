//! Moving sessions in and out: a bundle upload, a whole project's sync stream, and reading such a
//! stream into this library.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use super::{Ctx, DeviceId, Input, OpError, Output, RawBody, RawData, SERVER_LOG, index_now, internal, parse_since, roots};
use crate::{bundle, index, runlog, sync};

/// The most bytes an uploaded bundle may hold unless `OpenParams.upload_cap` says otherwise.
pub const DEFAULT_UPLOAD_CAP: u64 = 512 * 1024 * 1024;

/// `export_project`'s request: the sync stream comes back as the raw body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportProjectRequest {
    /// A repository name as `sync::repo_name` gives it.
    pub repo: String,
    /// Read as `sp --since` reads it.
    #[serde(default)]
    pub since: Option<String>,
}

pub struct ImportStreamParams<'a> {
    /// The local checkout the sessions are listed under.
    pub project: PathBuf,
    /// Report what the import would do, writing nothing.
    pub dry_run: bool,
    pub input: &'a mut dyn Read,
}

/// Reads an upload of a bundle file (query `on_conflict=replace|keep`) and imports it into the
/// library, counting the bytes actually read against the cap. Over the cap is TooLarge and leaves
/// nothing behind. With conflicts and no `on_conflict`, nothing is written and the conflicts are
/// returned.
pub(super) fn import_bundle(ctx: &Arc<Ctx>, device: &DeviceId, input: Input) -> Result<Output, OpError> {
    let Input::Raw { body, query, .. } = input else { return Err(OpError::BadRequest("import_bundle takes the bundle file as its body".to_string())) };
    let on_conflict = match query.iter().find(|(k, _)| k == "on_conflict").map(|(_, v)| v.as_str()) {
        None => None,
        Some("replace") => Some(bundle::OnConflict::Replace),
        Some("keep") => Some(bundle::OnConflict::Keep),
        Some(other) => return Err(OpError::BadRequest(format!("on_conflict is replace or keep, not {other:?}"))),
    };
    // bundle::import reads a file, so the upload is kept in one until the import is done.
    static UPLOADS: AtomicU64 = AtomicU64::new(0);
    let file = ctx.paths.data.join(format!("upload-{}-{}.{}", std::process::id(), UPLOADS.fetch_add(1, Ordering::SeqCst), bundle::EXTENSION));
    struct Remove(PathBuf);
    impl Drop for Remove {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _remove = Remove(file.clone());
    // Owner-only: the upload holds someone's sessions. A file left by a run that crashed is
    // replaced rather than reused, so its permissions can't carry over.
    let _ = std::fs::remove_file(&file);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut out = options.open(&file).map_err(|e| internal(e.into()))?;
    let copied = std::io::copy(&mut body.take(ctx.upload_cap + 1), &mut out).map_err(|e| internal(e.into()))?;
    if copied > ctx.upload_cap {
        return Err(OpError::TooLarge);
    }
    drop(out);
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    let report = bundle::import(&conn, &ctx.paths.library, &file, on_conflict).map_err(|e| OpError::BadRequest(format!("{e:#}")))?;
    drop(conn);
    if !report.imported.is_empty() {
        index_now(ctx, device)?;
    }
    serde_json::to_value(report).map(Output::Json).map_err(|e| OpError::Internal(e.to_string()))
}

/// Streams the repository's sessions (`RawData::Stream`) from a producer thread with its own
/// database connection. Dropping the reader stops the producer. NotFound, before anything streams,
/// when no session of the repository is indexed here. A producer that fails logs why to server.log;
/// the cut stream has no End record, so the importer stores nothing.
pub(super) fn export_project(ctx: &Arc<Ctx>, _device: &DeviceId, input: Input) -> Result<Output, OpError> {
    let Input::Json(value) = input else { return Err(OpError::BadRequest("export_project takes a JSON body".to_string())) };
    let req: ExportProjectRequest = serde_json::from_value(value).map_err(|e| OpError::BadRequest(e.to_string()))?;
    let since = req.since.as_deref().map(|s| parse_since(s, ctx.now())).transpose().map_err(|e| OpError::BadRequest(e.to_string()))?;
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    let repo = sync::RepoName(req.repo.clone());
    let rows = index::list(&conn).map_err(internal)?;
    let mut names: std::collections::HashMap<&str, Option<sync::RepoName>> = std::collections::HashMap::new();
    // As `sync::export` picks sessions: this machine's own, not ones imported from a bundle.
    let known = rows.iter().filter(|r| !r.from_bundle).any(|r| names.entry(&r.project).or_insert_with(|| sync::repo_name(std::path::Path::new(&r.project))).as_ref() == Some(&repo));
    if !known {
        return Err(OpError::NotFound(format!("No sessions of the repository {} are indexed here", req.repo)));
    }
    let (reader, writer) = std::io::pipe().map_err(|e| internal(e.into()))?;
    let home = ctx.home.clone();
    let log = ctx.paths.logs.join(SERVER_LOG);
    let export = sync::ExportRequest { repo, since };
    std::thread::spawn(move || {
        // A reader that goes away makes the next write fail, which ends the export and closes
        // the connection.
        let mut out = std::io::BufWriter::new(writer);
        let result = sync::export(&conn, &home, &export, &mut out).and_then(|()| Ok(out.flush()?));
        if let Err(e) = result {
            runlog::append(&log, &format!("Export of {} stopped: {e:#}", export.repo.0));
        }
    });
    Ok(Output::Raw(RawBody { data: RawData::Stream(Box::new(reader)), content_type: "application/octet-stream".to_string(), filename: None }))
}

/// Reads what `export_project` wrote into this library, then indexes, so the sessions are listed
/// when it returns. A dry run stores nothing.
pub fn import_stream(ctx: &Arc<Ctx>, params: ImportStreamParams<'_>) -> Result<sync::PullSummary, OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    let target = sync::ImportTarget { project: params.project, home: ctx.home.clone(), dry_run: params.dry_run };
    let summary = sync::import(&conn, &target, params.input).map_err(|e| OpError::BadRequest(format!("{e:#}")))?;
    if !params.dry_run {
        // With no folders registered at all, the agents' folders that exist now are, so the
        // sessions pulled into them are listed.
        if index::roots(&conn).map_err(internal)?.is_empty() {
            for root in roots::agent_folders(&ctx.home) {
                index::add_root(&conn, &root).map_err(internal)?;
            }
        }
        let mut conn = conn;
        index::run(&mut conn, false, Some(&ctx.paths.library), &|_, _| {}).map_err(internal)?;
    }
    Ok(summary)
}
