//! The one set of operations every caller reaches: the server's `POST /api/v1/<op>`, the in-process
//! CLI and the client crate. See .plans/server-api/00-design.md.

mod events;
mod jobs;
pub mod wire;

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, bail};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone};
use serde::Serialize;
use serde::de::DeserializeOwned;
pub use wire::*;

use crate::generate::Claude;
use crate::index;
use crate::model::SessionRow;
use crate::sync;
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
    /// Operations other crates add: `pull` from the client crate, the pairing and service
    /// operations from the server crate.
    pub extensions: Vec<Extension>,
}

pub struct Ctx {
    paths: Paths,
    home: PathBuf,
    version: String,
    clock: Clock,
    extensions: Vec<Extension>,
    jobs: Jobs,
    events: Events,
    gate: Mutex<IndexGate>,
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
            clock,
            extensions: params.extensions,
            jobs: Jobs::new(started),
            events: Events::new(params.event_capacity.unwrap_or(events::DEFAULT_CAPACITY), started),
            gate: Mutex::new(IndexGate { running: None, pending: false, pending_full: false }),
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

pub type ExtensionHandler = fn(&Arc<Ctx>, &DeviceId, Input) -> Result<Output, OpError>;

/// An operation another crate adds through `OpenParams.extensions`.
#[derive(Clone, Copy)]
pub struct Extension {
    pub spec: OpSpec,
    pub handler: ExtensionHandler,
}

pub enum Input {
    Json(serde_json::Value),
    Raw { body: Box<dyn Read + Send>, content_type: Option<String> },
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
    ($($stat:ident = $name:literal ($req:ty => $res:ty) { body: $body:ident, inline: $inline:literal, access: $access:ident } => $handler:path;)*) => {
        $(
            pub static $stat: Op<$req, $res> = Op {
                spec: OpSpec { name: $name, body: Body::$body, inline: $inline, access: Access::$access },
                handler: $handler,
            };
        )*

        fn core_specs() -> Vec<OpSpec> {
            vec![$($stat.spec),*]
        }

        fn dispatch_core(ctx: &Arc<Ctx>, device: &DeviceId, name: String, input: Input) -> Result<Output, OpError> {
            match name.as_str() {
                $($name => dispatch_json(ctx, device, &$stat, input),)*
                _ => Err(OpError::UnknownOperation(name)),
            }
        }
    };
}

operations! {
    HELLO = "hello" (HelloRequest => HelloResponse) { body: Json, inline: true, access: Authenticated } => hello;
    JOB = "job" (JobRequest => Job) { body: Json, inline: true, access: Authenticated } => job;
    CANCEL_JOB = "cancel_job" (CancelJobRequest => ()) { body: Json, inline: true, access: Authenticated } => cancel_job;
    LIST_SESSIONS = "list_sessions" (ListSessionsRequest => Vec<SessionRow>) { body: Json, inline: false, access: Authenticated } => list_sessions;
    REINDEX = "reindex" (ReindexRequest => JobId) { body: Json, inline: false, access: Authenticated } => reindex;
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
        _ => {
            ctx.jobs.finish(&req.id, JobState::Cancelled, None, (ctx.clock)());
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
