//! Analyze for one session: `claude` writes its metadata and annotations, as a job.

use std::sync::{Arc, PoisonError};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use super::{Ctx, DeviceId, GenerateDone, GenerateProgress, JobId, JobKind, JobState, OpError, Progress, PushedEvent, first_line, focus_label, sessions};
use crate::generate::{self, Claude, Generated};
use crate::{index, reports, runlog};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerateNotesRequest {
    pub key: String,
    pub path: String,
    #[serde(default)]
    pub branch: Option<String>,
    /// The user's own focus text; None uses the built-in one.
    #[serde(default)]
    pub focus: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CancelGenerateRequest {
    pub key: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GeneratingRequest {}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DefaultPromptsRequest {}

/// The built-in focus text for Analyze and for reports, which the user can replace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DefaultPrompts {
    pub analyze: String,
    pub report: String,
}

/// Starts a Generate job for the session. Progress arrives as generate-progress events and the
/// end as one generate-done event, published before the job leaves Running. Conflict while a
/// job for the same key runs.
pub(super) fn generate_notes(ctx: &Arc<Ctx>, _device: &DeviceId, req: &GenerateNotesRequest) -> Result<JobId, OpError> {
    let (id, cancel) = {
        let mut running = ctx.generating.lock().unwrap();
        if running.contains_key(&req.key) {
            return Err(OpError::Conflict("Already generating notes for this session".to_string()));
        }
        let (id, cancel) = ctx.start_cancellable(JobKind::Generate, Some(req.key.clone()));
        running.insert(req.key.clone(), id.clone());
        (id, cancel)
    };
    let log = ctx.analyze_log();
    let branch = req.branch.as_deref().map(|b| format!(" (branch {b})")).unwrap_or_default();
    runlog::append(&log, &format!("Analyze {} started, with {}\n{}{branch}", req.key, focus_label(req.focus.as_deref()), req.path));
    let ctx = ctx.clone();
    let req = req.clone();
    let job = id.clone();
    std::thread::spawn(move || {
        let running = Running { ctx: ctx.clone(), key: req.key.clone(), job: job.clone() };
        let key = req.key.clone();
        let result = ctx.claude().and_then(|claude| {
            analyze_session(&ctx, &AnalyzeSession { claude: &claude, key: &key, path: &req.path, branch: req.branch.as_deref(), focus: req.focus.as_deref(), cancel: &cancel }, &mut |done, total| {
                ctx.jobs.progress(&job, Progress { done, total });
                ctx.publish(PushedEvent::GenerateProgress(GenerateProgress { key: key.clone(), done, total }));
            })
        });
        running.free();
        let (done, state) = match result {
            Ok(generated) => (GenerateDone { key, error: None, cancelled: false, rejected: generated.rejected }, JobState::Finished),
            Err(e) if generate::is_cancelled(&e) => (GenerateDone { key, error: None, cancelled: true, rejected: 0 }, JobState::Cancelled),
            Err(e) => (GenerateDone { key, error: Some(first_line(&e)), cancelled: false, rejected: 0 }, JobState::Failed),
        };
        let error = done.error.clone();
        ctx.publish(PushedEvent::GenerateDone(done));
        ctx.end_job(&job, state, error);
    });
    Ok(id)
}

/// Held by a Generate job's thread. `free` clears the session's entry so it can be generated
/// again; dropping the guard clears it too and ends the job as Failed unless it already ended, so a
/// thread that panics leaves neither the session marked as generating nor the job Running.
struct Running {
    ctx: Arc<Ctx>,
    key: String,
    job: JobId,
}

impl Running {
    fn free(&self) {
        let mut generating = self.ctx.generating.lock().unwrap_or_else(PoisonError::into_inner);
        if generating.get(&self.key) == Some(&self.job) {
            generating.remove(&self.key);
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.free();
        if std::thread::panicking() {
            runlog::append(&self.ctx.analyze_log(), &format!("Analyze {} stopped unexpectedly", self.key));
        }
        let error = format!("The job stopped unexpectedly. Details are in {}.", self.ctx.analyze_log().display());
        self.ctx.end_job(&self.job, JobState::Failed, Some(error));
    }
}

pub(super) struct AnalyzeSession<'a> {
    pub claude: &'a Claude,
    pub key: &'a str,
    pub path: &'a str,
    pub branch: Option<&'a str>,
    pub focus: Option<&'a str>,
    pub cancel: &'a Arc<AtomicBool>,
}

/// Runs Analyze on one session and saves the result, logging its end.
pub(super) fn analyze_session(ctx: &Ctx, s: &AnalyzeSession, progress: &mut dyn FnMut(usize, usize)) -> anyhow::Result<Generated> {
    let log = ctx.analyze_log();
    let result = (|| {
        let events = sessions::session_events(s.path, s.branch)?;
        let mut ask = |system: &str, input: &str, schema: &serde_json::Value| generate::run_claude(s.claude, system, input, schema, s.cancel);
        let generated = generate::generate(&events, s.focus, &mut ask, progress)?;
        let conn = index::open(&ctx.paths.db)?;
        generate::apply(&conn, s.key, &generated, &events)?;
        Ok(generated)
    })();
    let key = s.key;
    match &result {
        Ok(g) => {
            let mut summary = format!(
                "outcome {} ({}), {} components, {} annotations saved",
                format!("{:?}", g.outcome).to_lowercase(),
                if g.outcome_note.is_some() { "with a reason" } else { "NO REASON GIVEN" },
                g.components.len(),
                g.annotations.len()
            );
            if g.rejected > 0 {
                summary.push_str(&format!(", {} dropped for citing events that don't exist or for missing fields", g.rejected));
            }
            runlog::append(&log, &format!("Analyze {key} done: {summary}"));
        }
        Err(e) if generate::is_cancelled(e) => runlog::append(&log, &format!("Analyze {key} cancelled")),
        Err(e) => runlog::append(&log, &format!("Analyze {key} FAILED: {e:#}")),
    }
    result
}

/// Cancels the running Generate job for `key` and kills its `claude`.
pub(super) fn cancel_generate(ctx: &Arc<Ctx>, _device: &DeviceId, req: &CancelGenerateRequest) -> Result<(), OpError> {
    let Some(id) = ctx.generating.lock().unwrap().get(&req.key).cloned() else { return Ok(()) };
    if let Some(flag) = ctx.cancels.lock().unwrap().get(&id) {
        flag.store(true, Ordering::SeqCst);
    }
    Ok(())
}

/// Keys of the sessions a Generate job is running for.
pub(super) fn generating(ctx: &Arc<Ctx>, _device: &DeviceId, _req: &GeneratingRequest) -> Result<Vec<String>, OpError> {
    Ok(ctx.generating.lock().unwrap().keys().cloned().collect())
}

pub(super) fn default_prompts(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &DefaultPromptsRequest) -> Result<DefaultPrompts, OpError> {
    Ok(DefaultPrompts { analyze: generate::DEFAULT_FOCUS.to_string(), report: reports::DEFAULT_FOCUS.to_string() })
}
