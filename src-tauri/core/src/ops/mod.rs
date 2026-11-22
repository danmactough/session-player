//! The one set of operations every caller reaches: the server's `POST /api/v1/<op>`, the in-process
//! CLI and the client crate. See .plans/server-api/00-design.md.

mod analyze;
mod events;
mod export;
mod generate_job;
mod import;
mod jobs;
mod notes;
mod report_ops;
mod roots;
mod sessions;
mod settings;
pub mod wire;

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, bail};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone};
use serde::Serialize;
use serde::de::DeserializeOwned;
pub use analyze::*;
pub use export::*;
pub use generate_job::*;
pub use import::*;
pub use notes::*;
pub use report_ops::*;
pub use roots::*;
pub use sessions::*;
pub use settings::*;
pub use wire::*;

use crate::generate::Claude;
use crate::model::{Event, SessionRow};
use crate::{annotations, index, reports, sync};
use events::{Events, Since};
use jobs::Jobs;

/// Where the app keeps its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub data: PathBuf,
    pub db: PathBuf,
    pub library: PathBuf,
    pub logs: PathBuf,
}

/// Milliseconds since the Unix epoch. Tests pass a fixed one.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

pub struct OpenParams {
    pub paths: Paths,
    /// The home folder the agents keep their sessions under.
    pub home: PathBuf,
    /// The server version `hello` reports.
    pub version: String,
    /// Used in place of finding the `claude` command, so tests can run a stand-in.
    pub claude: Option<Claude>,
    pub clock: Option<Clock>,
    /// How many events the ring keeps before older ones are answered with a Snapshot.
    pub event_capacity: Option<usize>,
    /// The most bytes an upload (`import_bundle`) may hold; None is `DEFAULT_UPLOAD_CAP`.
    pub upload_cap: Option<u64>,
    /// Operations other crates add: `pull` from the client crate, the pairing and service
    /// operations from the server crate.
    pub extensions: Vec<Extension>,
}

pub struct Ctx {
    paths: Paths,
    home: PathBuf,
    version: String,
    claude: Option<Claude>,
    clock: Clock,
    upload_cap: u64,
    extensions: Vec<Extension>,
    jobs: Jobs,
    events: Events,
    gate: Mutex<IndexGate>,
    /// The flag that stops each running job that can be cancelled.
    cancels: Mutex<HashMap<JobId, Arc<AtomicBool>>>,
    /// Session keys with a running Generate job, and that job.
    generating: Mutex<HashMap<String, JobId>>,
    analyzing: Mutex<Option<JobId>>,
    reporting: Mutex<Option<JobId>>,
    exports: Mutex<HashMap<DeviceId, export::PendingExport>>,
    images: Mutex<sessions::ImageCache>,
}

/// Coalesces index requests: a request during a pass joins it and leaves at most one follow-up.
struct IndexGate {
    running: Option<JobId>,
    pending: bool,
    pending_full: bool,
}

impl Ctx {
    /// Fails when two operations share a name. Does not index.
    pub fn open(params: OpenParams) -> anyhow::Result<Arc<Ctx>> {
        let mut names = HashSet::new();
        for spec in core_specs().iter().chain(params.extensions.iter().map(|e| &e.spec)) {
            if !names.insert(spec.name) {
                bail!("Two operations are named {}", spec.name);
            }
        }
        let clock = params.clock.unwrap_or_else(|| Arc::new(|| chrono::Utc::now().timestamp_millis() as u64));
        let started = clock();
        Ok(Arc::new(Ctx {
            paths: params.paths,
            home: params.home,
            version: params.version,
            claude: params.claude,
            clock,
            upload_cap: params.upload_cap.unwrap_or(DEFAULT_UPLOAD_CAP),
            extensions: params.extensions,
            jobs: Jobs::new(started),
            events: Events::new(params.event_capacity.unwrap_or(events::DEFAULT_CAPACITY), started),
            gate: Mutex::new(IndexGate { running: None, pending: false, pending_full: false }),
            cancels: Mutex::new(HashMap::new()),
            generating: Mutex::new(HashMap::new()),
            analyzing: Mutex::new(None),
            reporting: Mutex::new(None),
            exports: Mutex::new(HashMap::new()),
            // Generations start at the start time, so an image URL from an earlier run is never
            // mistaken for one of this run's.
            images: Mutex::new(sessions::ImageCache::new(started)),
        }))
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// The events after `after`, waiting up to `wait` for one when there are none. An id older
    /// than the ring keeps gets one Snapshot instead.
    pub fn events_since(&self, since: EventsSince) -> Vec<ServerEvent> {
        match self.events.since(since.after, since.wait) {
            Since::Events(events) => events,
            Since::Gone { newest } => {
                vec![ServerEvent { id: newest, at: (self.clock)(), event: PushedEvent::Snapshot { running: self.jobs.running() } }]
            }
        }
    }

    fn publish(&self, event: PushedEvent) {
        self.events.publish(event, (self.clock)());
    }

    fn now(&self) -> u64 {
        (self.clock)()
    }

    /// The log of Analyze and report runs; see `runlog`.
    fn analyze_log(&self) -> PathBuf {
        self.paths.logs.join("analyze.log")
    }

    /// Starts a job that `cancel_job` can stop through the returned flag.
    fn start_cancellable(&self, kind: JobKind, key: Option<String>) -> (JobId, Arc<AtomicBool>) {
        let id = self.jobs.start(kind, key, self.now());
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancels.lock().unwrap().insert(id.clone(), cancel.clone());
        (id, cancel)
    }

    fn end_job(&self, id: &JobId, state: JobState, error: Option<String>) {
        self.cancels.lock().unwrap().remove(id);
        self.jobs.finish(id, state, error, self.now());
    }

    /// The `claude` command: the stand-in the Ctx was opened with, else the one installed, with
    /// this process's environment and the Claude settings.
    fn claude(&self) -> anyhow::Result<Claude> {
        if let Some(claude) = &self.claude {
            return Ok(claude.clone());
        }
        let home = dirs::home_dir().unwrap_or_default();
        let on_path: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).map(|d| d.join("claude")).collect()).unwrap_or_default();
        let bin = on_path
            .into_iter()
            .chain([home.join(".local/bin/claude"), home.join(".claude/local/claude")])
            .find(|p| p.is_file())
            .context("Couldn't find the claude command. Install Claude Code and sign in, then try again.")?;
        let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join(".claude"));
        let mut claude = Claude::new(bin, std::env::vars().collect(), &config_dir)?;
        claude.log = Some(self.analyze_log());
        Ok(claude)
    }

    /// The index rows of `paths`, one per session key (a pi session's branches share one).
    fn rows_of(&self, paths: &[String]) -> Result<Vec<SessionRow>, OpError> {
        let conn = index::open(&self.paths.db).map_err(internal)?;
        let rows = index::list(&conn).map_err(internal)?;
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        for path in paths {
            let row = rows.iter().find(|r| r.path == *path).ok_or_else(|| OpError::NotFound(format!("No session is indexed at {path}")))?;
            if seen.insert(annotations::session_key(row)) {
                found.push(row.clone());
            }
        }
        Ok(found)
    }
}

/// What a caller is shown of an error: its first line. The rest (claude's own output) is in the log.
fn first_line(e: &anyhow::Error) -> String {
    format!("{e:#}").lines().next().unwrap_or_default().to_string()
}

/// The focus text the UI saved under `field` of its `prefs` setting, if any.
fn saved_focus(conn: &rusqlite::Connection, field: &str) -> Option<String> {
    let saved: Option<String> = conn.query_row("SELECT value FROM settings WHERE key='ui.prefs'", [], |r| r.get(0)).ok();
    saved
        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
        .and_then(|prefs| prefs.get(field).and_then(|v| v.as_str()).map(str::to_string))
        .filter(|text| !text.trim().is_empty())
}

fn focus_label(focus: Option<&str>) -> &'static str {
    if focus.is_some_and(|f| !f.trim().is_empty()) { "your own focus" } else { "the built-in focus" }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Body {
    Json,
    Raw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Authenticated,
    /// Reachable without a token: `connect` only.
    Anonymous,
    /// Never reachable over HTTP: `service` only.
    NotOverHttp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpSpec {
    pub name: &'static str,
    pub body: Body,
    /// Runs on the server's request thread because it is cheap; every other operation runs where
    /// it may block.
    pub inline: bool,
    pub access: Access,
}

/// A typed operation, declared once by `operations!`.
pub struct Op<Req: 'static, Res: 'static> {
    pub spec: OpSpec,
    handler: fn(&Arc<Ctx>, &DeviceId, &Req) -> Result<Res, OpError>,
}

/// An operation with a raw body in or out, declared once by `operations!` and run by name with
/// `call_raw` or `dispatch`.
pub struct RawOp {
    pub spec: OpSpec,
    handler: ExtensionHandler,
}

pub type ExtensionHandler = fn(&Arc<Ctx>, &DeviceId, Input) -> Result<Output, OpError>;

/// An operation another crate adds through `OpenParams.extensions`.
#[derive(Clone, Copy)]
pub struct Extension {
    pub spec: OpSpec,
    pub handler: ExtensionHandler,
}

pub enum Input {
    Json(serde_json::Value),
    /// An uploaded body. `query` holds the decoded URL query pairs, which carry a raw operation's
    /// options (such as `import_bundle`'s `on_conflict`).
    Raw { body: Box<dyn Read + Send>, content_type: Option<String>, query: Vec<(String, String)> },
}

pub enum RawData {
    Bytes(Vec<u8>),
    Stream(Box<dyn Read + Send>),
}

pub struct RawBody {
    pub data: RawData,
    pub content_type: String,
    /// Offered to a browser as the download's name.
    pub filename: Option<String>,
}

pub enum Output {
    Json(serde_json::Value),
    Raw(RawBody),
}

pub struct DispatchParams {
    pub name: String,
    pub device: DeviceId,
    pub input: Input,
}

macro_rules! operations {
    (
        typed {
            $($stat:ident = $name:literal ($req:ty => $res:ty) { body: $body:ident, inline: $inline:literal, access: $access:ident } => $handler:path;)*
        }
        raw {
            $($rstat:ident = $rname:literal { body: $rbody:ident, inline: $rinline:literal, access: $raccess:ident } => $rhandler:path;)*
        }
    ) => {
        $(
            pub static $stat: Op<$req, $res> = Op {
                spec: OpSpec { name: $name, body: Body::$body, inline: $inline, access: Access::$access },
                handler: $handler,
            };
        )*
        $(
            pub static $rstat: RawOp = RawOp {
                spec: OpSpec { name: $rname, body: Body::$rbody, inline: $rinline, access: Access::$raccess },
                handler: $rhandler,
            };
        )*

        fn core_specs() -> Vec<OpSpec> {
            vec![$($stat.spec,)* $($rstat.spec,)*]
        }

        fn dispatch_core(ctx: &Arc<Ctx>, device: &DeviceId, name: String, input: Input) -> Result<Output, OpError> {
            match name.as_str() {
                $($name => dispatch_json(ctx, device, &$stat, input),)*
                $($rname => ($rstat.handler)(ctx, device, input),)*
                _ => Err(OpError::UnknownOperation(name)),
            }
        }
    };
}

operations! {
    typed {
        HELLO = "hello" (HelloRequest => HelloResponse) { body: Json, inline: true, access: Authenticated } => hello;
        JOB = "job" (JobRequest => Job) { body: Json, inline: true, access: Authenticated } => job;
        CANCEL_JOB = "cancel_job" (CancelJobRequest => ()) { body: Json, inline: true, access: Authenticated } => cancel_job;
        LIST_SESSIONS = "list_sessions" (ListSessionsRequest => Vec<SessionRow>) { body: Json, inline: false, access: Authenticated } => list_sessions;
        REINDEX = "reindex" (ReindexRequest => JobId) { body: Json, inline: false, access: Authenticated } => reindex;

        GET_ROOTS = "get_roots" (GetRootsRequest => Vec<RootInfo>) { body: Json, inline: false, access: Authenticated } => roots::get_roots;
        SUGGESTED_ROOTS = "suggested_roots" (SuggestedRootsRequest => Vec<String>) { body: Json, inline: false, access: Authenticated } => roots::suggested_roots;
        ADD_ROOT = "add_root" (AddRootRequest => RootInfo) { body: Json, inline: false, access: Authenticated } => roots::add_root;
        REMOVE_ROOT = "remove_root" (RemoveRootRequest => ()) { body: Json, inline: false, access: Authenticated } => roots::remove_root;

        SEARCH = "search" (SearchRequest => Vec<index::SearchHit>) { body: Json, inline: false, access: Authenticated } => sessions::search;
        LOAD_SESSION = "load_session" (LoadSessionRequest => Vec<Event>) { body: Json, inline: false, access: Authenticated } => sessions::load_session;

        GET_SETTING = "get_setting" (GetSettingRequest => Option<String>) { body: Json, inline: false, access: Authenticated } => settings::get_setting;
        SET_SETTING = "set_setting" (SetSettingRequest => ()) { body: Json, inline: false, access: Authenticated } => settings::set_setting;
        LOG_FRONTEND = "log_frontend" (LogFrontendRequest => ()) { body: Json, inline: false, access: Authenticated } => settings::log_frontend;
        LIBRARY_ERRORS = "library_errors" (LibraryErrorsRequest => Vec<index::LibraryError>) { body: Json, inline: false, access: Authenticated } => settings::library_errors;
        REMOVE_IMPORTED = "remove_imported" (RemoveImportedRequest => ()) { body: Json, inline: false, access: Authenticated } => settings::remove_imported;

        EXPORT_PREVIEW = "export_preview" (ExportPreviewRequest => ExportPreview) { body: Json, inline: false, access: Authenticated } => export::export_preview;
        EXPORT_CANCEL = "export_cancel" (ExportCancelRequest => ()) { body: Json, inline: false, access: Authenticated } => export::export_cancel;

        SESSION_NOTES = "session_notes" (SessionNotesRequest => SessionNotes) { body: Json, inline: false, access: Authenticated } => notes::session_notes;
        SAVE_META = "save_meta" (SaveMetaRequest => Option<annotations::SessionMeta>) { body: Json, inline: false, access: Authenticated } => notes::save_meta;
        SAVE_ANNOTATION = "save_annotation" (SaveAnnotationRequest => annotations::Annotation) { body: Json, inline: false, access: Authenticated } => notes::save_annotation;
        DELETE_ANNOTATION = "delete_annotation" (DeleteAnnotationRequest => ()) { body: Json, inline: false, access: Authenticated } => notes::delete_annotation;
        NOTE_SUMMARIES = "note_summaries" (NoteSummariesRequest => Vec<annotations::Summary>) { body: Json, inline: false, access: Authenticated } => notes::note_summaries;

        GENERATE_NOTES = "generate_notes" (GenerateNotesRequest => JobId) { body: Json, inline: false, access: Authenticated } => generate_job::generate_notes;
        CANCEL_GENERATE = "cancel_generate" (CancelGenerateRequest => ()) { body: Json, inline: false, access: Authenticated } => generate_job::cancel_generate;
        GENERATING = "generating" (GeneratingRequest => Vec<String>) { body: Json, inline: false, access: Authenticated } => generate_job::generating;
        DEFAULT_PROMPTS = "default_prompts" (DefaultPromptsRequest => DefaultPrompts) { body: Json, inline: false, access: Authenticated } => generate_job::default_prompts;
        ANALYZE_SESSIONS = "analyze_sessions" (AnalyzeSessionsRequest => JobId) { body: Json, inline: false, access: Authenticated } => analyze::analyze_sessions;

        GENERATE_REPORT = "generate_report" (GenerateReportRequest => JobId) { body: Json, inline: false, access: Authenticated } => report_ops::generate_report;
        CANCEL_REPORT = "cancel_report" (CancelReportRequest => ()) { body: Json, inline: false, access: Authenticated } => report_ops::cancel_report;
        LIST_REPORTS = "list_reports" (ListReportsRequest => Vec<reports::ReportSummary>) { body: Json, inline: false, access: Authenticated } => report_ops::list_reports;
        GET_REPORT = "get_report" (GetReportRequest => Option<reports::Report>) { body: Json, inline: false, access: Authenticated } => report_ops::get_report;
        REPORT_ANNOTATIONS = "report_annotations" (ReportAnnotationsRequest => Vec<reports::CitedAnnotation>) { body: Json, inline: false, access: Authenticated } => report_ops::report_annotations;
        DELETE_REPORT = "delete_report" (DeleteReportRequest => ()) { body: Json, inline: false, access: Authenticated } => report_ops::delete_report;
        ACTION_RESPONSES = "action_responses" (ActionResponsesRequest => Vec<reports::ActionResponse>) { body: Json, inline: false, access: Authenticated } => report_ops::action_responses;
        SET_ACTION_RESPONSE = "set_action_response" (SetActionResponseRequest => Option<reports::ActionResponse>) { body: Json, inline: false, access: Authenticated } => report_ops::set_action_response;
        ACTION_VERDICTS = "action_verdicts" (ActionVerdictsRequest => Vec<reports::LatestVerdict>) { body: Json, inline: false, access: Authenticated } => report_ops::action_verdicts;
    }
    raw {
        // JSON in, the bundle file out (Bytes).
        EXPORT_SESSION = "export_session" { body: Json, inline: false, access: Authenticated } => export::export_session;
        // The bundle file in, an ImportReport out.
        IMPORT_BUNDLE = "import_bundle" { body: Raw, inline: false, access: Authenticated } => import::import_bundle;
        // JSON in, a sync stream out (Stream).
        EXPORT_PROJECT = "export_project" { body: Json, inline: false, access: Authenticated } => import::export_project;
    }
}

/// Every operation this Ctx answers: the core ones and the extensions it was opened with.
pub fn specs(ctx: &Ctx) -> Vec<OpSpec> {
    let mut specs = core_specs();
    specs.extend(ctx.extensions.iter().map(|e| e.spec));
    specs
}

pub fn spec_of(ctx: &Ctx, name: &str) -> Option<OpSpec> {
    specs(ctx).into_iter().find(|s| s.name == name)
}

/// Runs an operation by name. The HTTP server's one entry point.
pub fn dispatch(ctx: &Arc<Ctx>, params: DispatchParams) -> Result<Output, OpError> {
    match ctx.extensions.iter().find(|e| e.spec.name == params.name) {
        Some(extension) => (extension.handler)(ctx, &params.device, params.input),
        None => dispatch_core(ctx, &params.device, params.name, params.input),
    }
}

/// Runs a JSON operation in process, as the local device.
pub fn call<Req, Res>(ctx: &Arc<Ctx>, op: &Op<Req, Res>, req: &Req) -> Result<Res, OpError> {
    (op.handler)(ctx, &local_device(), req)
}

/// Runs a raw-body operation in process, as the local device.
pub fn call_raw(ctx: &Arc<Ctx>, name: &str, input: Input) -> Result<Output, OpError> {
    dispatch(ctx, DispatchParams { name: name.to_string(), device: local_device(), input })
}

fn local_device() -> DeviceId {
    DeviceId("local".to_string())
}

fn dispatch_json<Req: DeserializeOwned, Res: Serialize>(
    ctx: &Arc<Ctx>,
    device: &DeviceId,
    op: &Op<Req, Res>,
    input: Input,
) -> Result<Output, OpError> {
    let Input::Json(value) = input else {
        return Err(OpError::BadRequest(format!("{} takes a JSON body", op.spec.name)));
    };
    let req: Req = serde_json::from_value(value).map_err(|e| OpError::BadRequest(e.to_string()))?;
    let res = (op.handler)(ctx, device, &req)?;
    serde_json::to_value(res).map(Output::Json).map_err(|e| OpError::Internal(e.to_string()))
}

fn internal(e: anyhow::Error) -> OpError {
    OpError::Internal(format!("{e:#}"))
}

fn hello(ctx: &Arc<Ctx>, _device: &DeviceId, _req: &HelloRequest) -> Result<HelloResponse, OpError> {
    Ok(HelloResponse {
        api_version: API_VERSION,
        api_minor: API_MINOR,
        server_version: ctx.version.clone(),
        home: ctx.home.to_string_lossy().into_owned(),
        // No device can pair yet, so the server never listens off loopback.
        loopback_only: true,
    })
}

fn job(ctx: &Arc<Ctx>, _device: &DeviceId, req: &JobRequest) -> Result<Job, OpError> {
    ctx.jobs.get(&req.id).ok_or_else(|| OpError::NotFound(format!("No job {}", req.id.0)))
}

fn cancel_job(ctx: &Arc<Ctx>, _device: &DeviceId, req: &CancelJobRequest) -> Result<(), OpError> {
    let job = ctx.jobs.get(&req.id).ok_or_else(|| OpError::NotFound(format!("No job {}", req.id.0)))?;
    match job.kind {
        JobKind::Index => Err(OpError::Conflict("An index pass can't be cancelled".to_string())),
        // The job's own thread ends it as Cancelled once its `claude` is stopped, after publishing
        // its done event.
        _ => {
            if let Some(flag) = ctx.cancels.lock().unwrap().get(&req.id) {
                flag.store(true, Ordering::SeqCst);
            }
            Ok(())
        }
    }
}

fn list_sessions(ctx: &Arc<Ctx>, _device: &DeviceId, req: &ListSessionsRequest) -> Result<Vec<SessionRow>, OpError> {
    let since = req.since.as_deref().map(|s| parse_since(s, (ctx.clock)())).transpose().map_err(|e| OpError::BadRequest(e.to_string()))?;
    let conn = index::open(&ctx.paths.db).map_err(internal)?;
    let rows = index::list(&conn).map_err(internal)?;
    let mut names: HashMap<String, Option<sync::RepoName>> = HashMap::new();
    Ok(rows
        .into_iter()
        .filter(|r| match &req.repo {
            Some(repo) => names.entry(r.project.clone()).or_insert_with(|| sync::repo_name(Path::new(&r.project))).as_ref().is_some_and(|n| n.0 == *repo),
            None => true,
        })
        .filter(|r| since.is_none_or(|s| r.started_at.or(r.updated_at).is_some_and(|t| t >= s)))
        .collect())
}

/// When `since` starts, in ms. The same forms `sp --since` reads; an age counts back from `now`.
fn parse_since(text: &str, now: u64) -> anyhow::Result<i64> {
    let text = text.trim();
    let local = |t: NaiveDateTime| {
        Local.from_local_datetime(&t).earliest().map(|t| t.timestamp_millis()).with_context(|| format!("{text} isn't a time in this time zone"))
    };
    if let Some(unit) = text.chars().last().filter(|c| "hdw".contains(*c))
        && let Ok(n) = text[..text.len() - 1].parse::<i64>()
    {
        let hours = match unit {
            'h' => 1,
            'd' => 24,
            _ => 24 * 7,
        };
        return Ok(now as i64 - n * hours * 3_600_000);
    }
    if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return local(date.and_hms_opt(0, 0, 0).expect("midnight"));
    }
    if let Ok(t) = DateTime::parse_from_rfc3339(text) {
        return Ok(t.timestamp_millis());
    }
    for format in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(t) = NaiveDateTime::parse_from_str(text, format) {
            return local(t);
        }
    }
    bail!("Couldn't read since {text}. Use a date (2026-11-01), a date and time (2026-11-01T09:30) or an age (7d, 12h, 2w).")
}

fn reindex(ctx: &Arc<Ctx>, _device: &DeviceId, req: &ReindexRequest) -> Result<JobId, OpError> {
    let mut gate = ctx.gate.lock().unwrap();
    if let Some(running) = gate.running.clone() {
        gate.pending = true;
        gate.pending_full |= req.full;
        return Ok(running);
    }
    let id = ctx.jobs.start(JobKind::Index, None, (ctx.clock)());
    gate.running = Some(id.clone());
    drop(gate);
    let ctx = ctx.clone();
    let job = id.clone();
    let full = req.full;
    std::thread::spawn(move || run_index(&ctx, &job, full));
    Ok(id)
}

/// Requests an index pass and waits for it, so what changed is listed when the caller returns.
fn index_now(ctx: &Arc<Ctx>, device: &DeviceId) -> Result<(), OpError> {
    let id = reindex(ctx, device, &ReindexRequest { full: false })?;
    while ctx.jobs.get(&id).is_some_and(|j| j.state == JobState::Running) {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    Ok(())
}

/// Runs index passes for one job until no request is pending. Requests that arrive during a pass
/// share this job, so its id stays Running until their follow-up pass is done.
fn run_index(ctx: &Arc<Ctx>, id: &JobId, mut full: bool) {
    loop {
        ctx.publish(PushedEvent::IndexStarted);
        let result = index::open(&ctx.paths.db).and_then(|mut conn| {
            index::run(&mut conn, full, Some(&ctx.paths.library), &|done, total| {
                ctx.jobs.progress(id, Progress { done, total });
                ctx.publish(PushedEvent::IndexProgress(Progress { done, total }));
            })
        });
        let error = match result {
            Ok(stats) => {
                ctx.publish(PushedEvent::IndexDone(stats));
                None
            }
            Err(e) => {
                let message = format!("{e:#}");
                ctx.publish(PushedEvent::IndexError(message.clone()));
                Some(message)
            }
        };
        let mut gate = ctx.gate.lock().unwrap();
        if !gate.pending {
            gate.running = None;
            let state = if error.is_some() { JobState::Failed } else { JobState::Finished };
            ctx.jobs.finish(id, state, error, (ctx.clock)());
            return;
        }
        full = gate.pending_full;
        gate.pending = false;
        gate.pending_full = false;
    }
}
