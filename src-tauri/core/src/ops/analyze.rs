//! Analyze for many sessions as one job.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

use super::generate_job::{AnalyzeSession, analyze_session};
use super::{AnalyzeDone, AnalyzeProgress, Ctx, DeviceId, JobId, JobKind, JobState, OpError, Progress, PushedEvent, internal, saved_focus};
use crate::generate::{self, Claude};
use crate::model::SessionRow;
use crate::{annotations, index, runlog};

/// How many sessions an Analyze job works on at the same time.
pub const ANALYZE_AT_ONCE: usize = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyzeSessionsRequest {
    /// Session files as `list_sessions` gives them; sessions sharing a key are analyzed once.
    pub paths: Vec<String>,
    /// Analyze sessions again that already have an analysis.
    #[serde(default)]
    pub force: bool,
    /// The user's own focus text; None uses the one saved in the UI's settings, else the built-in.
    #[serde(default)]
    pub focus: Option<String>,
}

/// Starts an Analyze job: `ANALYZE_AT_ONCE` sessions at a time, skipping analyzed ones unless
/// `force`. It publishes analyze-progress and ends with one analyze-done holding the counts, before
/// leaving Running. Ends Failed when any session failed. Conflict while another Analyze job runs.
pub(super) fn analyze_sessions(ctx: &Arc<Ctx>, _device: &DeviceId, req: &AnalyzeSessionsRequest) -> Result<JobId, OpError> {
    let rows = ctx.rows_of(&req.paths)?;
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    let analyzed = analyzed_keys(&conn).map_err(internal)?;
    let focus = req.focus.clone().or_else(|| saved_focus(&conn, "analyzePrompt"));
    drop(conn);
    let (todo, skipped): (Vec<SessionRow>, Vec<SessionRow>) = rows.into_iter().partition(|r| req.force || !analyzed.contains(&annotations::session_key(r)));
    let skipped = skipped.len();

    let mut running = ctx.analyzing.lock().unwrap();
    if running.is_some() {
        return Err(OpError::Conflict("Another Analyze job is running".to_string()));
    }
    let (id, cancel) = ctx.start_cancellable(JobKind::Analyze, None);
    *running = Some(id.clone());
    drop(running);

    let ctx = ctx.clone();
    let job = id.clone();
    std::thread::spawn(move || {
        let total = todo.len();
        let progress = |done: usize| {
            ctx.jobs.progress(&job, Progress { done, total });
            ctx.publish(PushedEvent::AnalyzeProgress(AnalyzeProgress { job: job.clone(), done, total }));
        };
        progress(0);
        let counts = match ctx.claude() {
            Ok(claude) => analyze_many(&ctx, &AnalyzeMany { claude: &claude, rows: &todo, focus: focus.as_deref(), cancel: &cancel }, &progress),
            Err(e) => {
                runlog::append(&ctx.analyze_log(), &format!("Analyze FAILED: {e:#}"));
                Counts { analyzed: 0, failed: total }
            }
        };
        let cancelled = cancel.load(Ordering::SeqCst);
        ctx.publish(PushedEvent::AnalyzeDone(AnalyzeDone { job: job.clone(), analyzed: counts.analyzed, skipped, failed: counts.failed, cancelled }));
        *ctx.analyzing.lock().unwrap() = None;
        let (state, error) = if cancelled {
            (JobState::Cancelled, None)
        } else if counts.failed > 0 {
            let log = ctx.analyze_log();
            (JobState::Failed, Some(format!("{} of {total} sessions couldn't be analyzed. Details are in {}.", counts.failed, log.display())))
        } else {
            (JobState::Finished, None)
        };
        ctx.end_job(&job, state, error);
    });
    Ok(id)
}

pub(super) fn analyzed_keys(conn: &rusqlite::Connection) -> anyhow::Result<HashSet<String>> {
    Ok(annotations::summaries(conn)?.into_iter().filter(|s| s.analyzed).map(|s| s.key).collect())
}

pub(super) struct AnalyzeMany<'a> {
    pub claude: &'a Claude,
    pub rows: &'a [SessionRow],
    pub focus: Option<&'a str>,
    pub cancel: &'a Arc<AtomicBool>,
}

pub(super) struct Counts {
    pub analyzed: usize,
    pub failed: usize,
}

/// Analyzes `rows`, `ANALYZE_AT_ONCE` at a time, calling `progress` with the number finished after
/// each. Starts no more once `cancel` is set; a cancelled session counts as neither.
pub(super) fn analyze_many(ctx: &Ctx, m: &AnalyzeMany, progress: &(dyn Fn(usize) + Sync)) -> Counts {
    let next = AtomicUsize::new(0);
    let finished = AtomicUsize::new(0);
    let analyzed = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..ANALYZE_AT_ONCE.min(m.rows.len()) {
            scope.spawn(|| {
                while !m.cancel.load(Ordering::SeqCst)
                    && let Some(row) = m.rows.get(next.fetch_add(1, Ordering::SeqCst))
                {
                    let key = annotations::session_key(row);
                    runlog::append(&ctx.analyze_log(), &format!("Analyze {key} started\n{}", row.path));
                    let session = AnalyzeSession { claude: m.claude, key: &key, path: &row.path, branch: None, focus: m.focus, cancel: m.cancel };
                    match analyze_session(ctx, &session, &mut |_, _| {}) {
                        Ok(_) => {
                            analyzed.fetch_add(1, Ordering::SeqCst);
                        }
                        Err(e) if generate::is_cancelled(&e) => continue,
                        Err(_) => {
                            failed.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                    progress(finished.fetch_add(1, Ordering::SeqCst) + 1);
                }
            });
        }
    });
    Counts { analyzed: analyzed.into_inner(), failed: failed.into_inner() }
}
