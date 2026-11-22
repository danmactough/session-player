//! Reports: writing one as a job, and reading, answering and deleting saved ones.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::bail;
use serde::{Deserialize, Serialize};

use super::analyze::{AnalyzeMany, analyze_many, analyzed_keys};
use super::{
    CancelJobRequest, Ctx, DeviceId, JobId, JobKind, JobState, OpError, Progress, PushedEvent, ReportDone, ReportProgress, cancel_job, first_line,
    focus_label, internal, saved_focus,
};
use crate::model::SessionRow;
use crate::reports::{self, ActionResponse, ActionStatus, CitedAnnotation, LatestVerdict, Report, ReportSummary, UserImpact};
use crate::{annotations, generate, index, runlog};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerateReportRequest {
    /// Session files as `list_sessions` gives them.
    pub paths: Vec<String>,
    /// A model alias such as "sonnet"; None or empty is `reports::DEFAULT_MODEL`.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub focus: Option<String>,
    /// Analyze the sessions without an analysis first; otherwise they are left out.
    #[serde(default)]
    pub analyze_first: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CancelReportRequest {
    /// The report job's id.
    pub job: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListReportsRequest {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetReportRequest {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportAnnotationsRequest {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteReportRequest {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionResponsesRequest {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetActionResponseRequest {
    pub input: ActionResponseChange,
}

/// `reports::ActionResponseInput` on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionResponseChange {
    pub report_id: String,
    pub action_index: usize,
    #[serde(default)]
    pub status: Option<ActionStatus>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub user_impact: Option<UserImpact>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionVerdictsRequest {
    pub id: String,
}

/// Starts a Report job. Progress arrives as report-progress events and the end as one report-done
/// naming the saved report. BadRequest with no sessions; Conflict while another Report job runs.
pub(super) fn generate_report(ctx: &Arc<Ctx>, _device: &DeviceId, req: &GenerateReportRequest) -> Result<JobId, OpError> {
    if req.paths.is_empty() {
        return Err(OpError::BadRequest("Pick at least one session to report on.".to_string()));
    }
    let rows = ctx.rows_of(&req.paths)?;
    let model = req.model.as_deref().map(str::trim).filter(|m| !m.is_empty()).unwrap_or(reports::DEFAULT_MODEL).to_string();
    let mut running = ctx.reporting.lock().unwrap();
    if running.is_some() {
        return Err(OpError::Conflict("Another report is being written".to_string()));
    }
    let (id, cancel) = ctx.start_cancellable(JobKind::Report, None);
    *running = Some(id.clone());
    drop(running);
    runlog::append(
        &ctx.analyze_log(),
        &format!("Report {} started on {} sessions with {model} and {}", id.0, rows.len(), focus_label(req.focus.as_deref())),
    );

    let ctx = ctx.clone();
    let req = req.clone();
    let job = id.clone();
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<String> {
            let claude = ctx.claude()?;
            let conn = index::open(&ctx.paths.db)?;
            if req.analyze_first {
                let analyzed = analyzed_keys(&conn)?;
                let unanalyzed: Vec<SessionRow> = rows.iter().filter(|r| !analyzed.contains(&annotations::session_key(r))).cloned().collect();
                // The saved Analyze focus applies here; the request's focus is for the report.
                let focus = saved_focus(&conn, "analyzePrompt");
                analyze_many(&ctx, &AnalyzeMany { claude: &claude, rows: &unanalyzed, focus: focus.as_deref(), cancel: &cancel }, &|_| {});
                if cancel.load(Ordering::SeqCst) {
                    bail!("Cancelled");
                }
            }
            let analyzed = analyzed_keys(&conn)?;
            let picks: Vec<reports::SessionPick> = rows
                .iter()
                .filter(|r| analyzed.contains(&annotations::session_key(r)))
                .map(|r| reports::SessionPick { key: annotations::session_key(r), title: r.title.clone() })
                .collect();
            if picks.is_empty() {
                bail!("None of the {} sessions has an analysis to report on.", rows.len());
            }
            let focus = req.focus.clone().or_else(|| saved_focus(&conn, "reportPrompt"));
            let mut ask = |system: &str, input: &str, schema: &serde_json::Value| generate::run_claude_model(&claude, &model, system, input, schema, &cancel);
            let report = reports::generate(&conn, &picks, focus.as_deref(), &mut ask, &mut |done, total| {
                ctx.jobs.progress(&job, Progress { done, total });
                ctx.publish(PushedEvent::ReportProgress(ReportProgress { job: job.0.clone(), done, total }));
            })?;
            reports::save_report(&conn, &report)?;
            Ok(report.id)
        })();
        let log = ctx.analyze_log();
        let (done, state) = match result {
            Ok(report) => {
                runlog::append(&log, &format!("Report {} done, saved as {report}", job.0));
                (ReportDone { job: job.0.clone(), report: Some(report), error: None, cancelled: false }, JobState::Finished)
            }
            Err(e) if generate::is_cancelled(&e) || cancel.load(Ordering::SeqCst) => {
                runlog::append(&log, &format!("Report {} cancelled", job.0));
                (ReportDone { job: job.0.clone(), report: None, error: None, cancelled: true }, JobState::Cancelled)
            }
            Err(e) => {
                runlog::append(&log, &format!("Report {} FAILED: {e:#}", job.0));
                (ReportDone { job: job.0.clone(), report: None, error: Some(first_line(&e)), cancelled: false }, JobState::Failed)
            }
        };
        let error = done.error.clone();
        ctx.publish(PushedEvent::ReportDone(done));
        *ctx.reporting.lock().unwrap() = None;
        ctx.end_job(&job, state, error);
    });
    Ok(id)
}

/// Cancels the Report job and kills its `claude`.
pub(super) fn cancel_report(ctx: &Arc<Ctx>, device: &DeviceId, req: &CancelReportRequest) -> Result<(), OpError> {
    cancel_job(ctx, device, &CancelJobRequest { id: JobId(req.job.clone()) })
}

pub(super) fn list_reports(ctx: &Arc<Ctx>, _device: &DeviceId, _req: &ListReportsRequest) -> Result<Vec<ReportSummary>, OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    reports::reports(&conn).map_err(internal)
}

pub(super) fn get_report(ctx: &Arc<Ctx>, _device: &DeviceId, req: &GetReportRequest) -> Result<Option<Report>, OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    reports::report(&conn, &req.id).map_err(internal)
}

/// The annotations a report was given, as it saw them, with their versions now.
pub(super) fn report_annotations(ctx: &Arc<Ctx>, _device: &DeviceId, req: &ReportAnnotationsRequest) -> Result<Vec<CitedAnnotation>, OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    let Some(report) = reports::report(&conn, &req.id).map_err(internal)? else { return Ok(Vec::new()) };
    reports::cited_annotations(&conn, &report).map_err(internal)
}

pub(super) fn delete_report(ctx: &Arc<Ctx>, _device: &DeviceId, req: &DeleteReportRequest) -> Result<(), OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    reports::delete_report(&conn, &req.id).map_err(internal)
}

pub(super) fn action_responses(ctx: &Arc<Ctx>, _device: &DeviceId, req: &ActionResponsesRequest) -> Result<Vec<ActionResponse>, OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    reports::action_responses(&conn, &req.id).map_err(internal)
}

/// Replaces the response to one action; None when nothing is left to keep.
pub(super) fn set_action_response(ctx: &Arc<Ctx>, _device: &DeviceId, req: &SetActionResponseRequest) -> Result<Option<ActionResponse>, OpError> {
    let c = &req.input;
    let input = reports::ActionResponseInput {
        report_id: c.report_id.clone(),
        action_index: c.action_index,
        status: c.status,
        note: c.note.clone(),
        user_impact: c.user_impact,
    };
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    reports::set_action_response(&conn, &input).map_err(internal)
}

/// The latest verdict later reports gave on each of this report's actions.
pub(super) fn action_verdicts(ctx: &Arc<Ctx>, _device: &DeviceId, req: &ActionVerdictsRequest) -> Result<Vec<LatestVerdict>, OpError> {
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    reports::action_verdicts(&conn, &req.id).map_err(internal)
}
