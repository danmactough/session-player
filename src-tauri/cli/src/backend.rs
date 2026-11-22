//! Runs operations where the route says: in this process on a Ctx, or over HTTP through the
//! client crate. Commands call `call`, `call_named` and `run` and never care which.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use serde::Serialize;
use serde::de::DeserializeOwned;
use session_player_client::{Client, PULL, ServersStore, check_compat};
use session_player_core::index;
use session_player_core::ops::{
    self, ApiError, CANCEL_JOB, CancelJobRequest, Ctx, EventsSince, HELLO, HelloRequest, Input, JOB, Job, JobId, JobKind, JobRequest, JobState, Op, OpError,
    Output, Paths, PushedEvent, ServerEvent,
};
use session_player_server::SERVER_EXTENSIONS;

use crate::route::Route;

/// How long a job is followed between checks of its state.
const POLL: Duration = Duration::from_millis(100);

/// The most `run` waits, after a job on a server ended, for the job's end event. That event was
/// published before the job ended, so it is already on its way; this only bounds a broken stream.
const END_EVENT_WAIT: Duration = Duration::from_secs(5);

pub enum Backend {
    InProcess(Arc<Ctx>),
    Http(Client),
}

pub struct OpenParams<'a> {
    pub route: Route,
    /// This machine's folders: the database for `Local`, `servers.json` for `Remote`.
    pub paths: &'a Paths,
    /// In process: index new and changed sessions first, as every command but `sp index` does.
    pub index: bool,
}

pub struct RunParams<'a> {
    /// Starts the job. Over HTTP it is called after the event stream is open, so no event of
    /// the job is missed.
    pub start: &'a dyn Fn(&Backend) -> anyhow::Result<JobId>,
    pub on_event: &'a mut dyn FnMut(&ServerEvent),
    /// Set by Ctrl-C. In process it cancels the job; over HTTP it detaches and the job keeps
    /// running on the server (`Detached`).
    pub cancel: &'a AtomicBool,
}

/// The error `run` ends with when Ctrl-C detaches from a job on a server.
#[derive(Debug)]
pub struct Detached(pub JobId);

impl std::fmt::Display for Detached {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "still running as {id}; stop it with sp cancel {id}", id = self.0.0)
    }
}

impl std::error::Error for Detached {}

/// A Ctx on this machine's data folder with every extension: the server's (pairing, service) and
/// the client's (pull). Registers the session folders but doesn't index.
pub fn open_ctx(paths: &Paths) -> anyhow::Result<Arc<Ctx>> {
    crate::register_session_folders(&index::open(&paths.db)?)?;
    let mut extensions = SERVER_EXTENSIONS.to_vec();
    extensions.push(PULL);
    Ctx::open(ops::OpenParams {
        paths: paths.clone(),
        home: dirs::home_dir().context("Couldn't find the home folder")?,
        version: env!("CARGO_PKG_VERSION").to_string(),
        claude: None,
        clock: None,
        event_capacity: None,
        upload_cap: None,
        extensions,
    })
}

impl Backend {
    /// In process: opens the Ctx and indexes in the foreground. Over HTTP: connects and checks
    /// the server's API version (`check_compat`).
    pub fn open(params: OpenParams) -> anyhow::Result<Backend> {
        let client = match params.route {
            Route::Local => {
                if params.index {
                    crate::open_indexed(params.paths)?;
                }
                return Ok(Backend::InProcess(open_ctx(params.paths)?));
            }
            Route::LocalServer { port, token } => Client::local(port, &token)?,
            Route::Remote { name } => {
                let server = ServersStore::new(&params.paths.data)
                    .get(&name)?
                    .with_context(|| format!("No server is named {name}. Pair with it first: sp connect <pairing string> --name {name}"))?;
                Client::paired(&server)?
            }
        };
        let hello = client.call(&HELLO, &HelloRequest {}).map_err(api_error)?;
        check_compat(&hello).map_err(|e| anyhow!(e))?;
        Ok(Backend::Http(client))
    }

    pub fn call<Req: Serialize, Res: DeserializeOwned>(&self, op: &Op<Req, Res>, req: &Req) -> anyhow::Result<Res> {
        match self {
            Backend::InProcess(ctx) => ops::call(ctx, op, req).map_err(op_error),
            Backend::Http(client) => client.call(op, req).map_err(api_error),
        }
    }

    /// An extension operation, which has no typed `Op`: pull, pair, devices, unpair, service.
    pub fn call_named<Req: Serialize, Res: DeserializeOwned>(&self, name: &str, req: &Req) -> anyhow::Result<Res> {
        match self {
            Backend::InProcess(ctx) => match ops::call_raw(ctx, name, Input::Json(serde_json::to_value(req)?)).map_err(op_error)? {
                Output::Json(value) => Ok(serde_json::from_value(value)?),
                Output::Raw(_) => bail!("{name} answered with a file"),
            },
            Backend::Http(client) => client.call_named(name, req).map_err(api_error),
        }
    }

    /// Starts a job and waits for it to leave Running. Over HTTP it subscribes to events before
    /// starting the job, then polls `job` for completion; it never infers completion from events.
    pub fn run(&self, params: RunParams) -> anyhow::Result<Job> {
        match self {
            Backend::InProcess(ctx) => self.run_in_process(ctx, params),
            Backend::Http(client) => self.run_over_http(client, params),
        }
    }

    fn run_in_process(&self, ctx: &Arc<Ctx>, params: RunParams) -> anyhow::Result<Job> {
        // A cursor past the newest event answers with a Snapshot holding the newest id.
        let mut cursor = ctx.events_since(EventsSince { after: Some(u64::MAX), wait: Duration::ZERO })[0].id;
        let id = (params.start)(self)?;
        let mut cancelling = false;
        loop {
            let job = ops::call(ctx, &JOB, &JobRequest { id: id.clone() }).map_err(op_error)?;
            let running = job.state == JobState::Running;
            // Every event of an ended job is already in the ring, so the last read doesn't wait.
            for event in ctx.events_since(EventsSince { after: Some(cursor), wait: if running { POLL } else { Duration::ZERO } }) {
                cursor = event.id;
                (params.on_event)(&event);
            }
            if !running {
                return Ok(job);
            }
            if !cancelling && params.cancel.load(Ordering::SeqCst) {
                cancelling = true;
                // An index pass can't be cancelled; it then runs to its end.
                let _ = ops::call(ctx, &CANCEL_JOB, &CancelJobRequest { id: id.clone() });
            }
        }
    }

    fn run_over_http(&self, client: &Client, params: RunParams) -> anyhow::Result<Job> {
        let mut events = client.events(None).map_err(api_error)?;
        let id = (params.start)(self)?;
        let mut end_seen = false;
        loop {
            let job = client.call(&JOB, &JobRequest { id: id.clone() }).map_err(api_error)?;
            if job.state != JobState::Running {
                // Only so the command can report what the end event says (an index pass's counts,
                // the saved report's id); the job is already over.
                let deadline = Instant::now() + END_EVENT_WAIT;
                while !end_seen {
                    match events.next_within(deadline.saturating_duration_since(Instant::now())) {
                        Some(Ok(event)) => follow(&event, &job, &mut end_seen, params.on_event),
                        _ => break,
                    }
                }
                return Ok(job);
            }
            if params.cancel.load(Ordering::SeqCst) {
                return Err(Detached(id).into());
            }
            let mut wait = POLL;
            while let Some(event) = events.next_within(wait) {
                follow(&event.map_err(api_error)?, &job, &mut end_seen, params.on_event);
                wait = Duration::ZERO;
            }
        }
    }
}

/// Hands `event` on and notes whether it is the end of `job`. An index job can run several passes,
/// so a pass starting after an end means the job's end is still to come.
fn follow(event: &ServerEvent, job: &Job, end_seen: &mut bool, on_event: &mut dyn FnMut(&ServerEvent)) {
    on_event(event);
    match (job.kind, &event.event) {
        (JobKind::Index, PushedEvent::IndexStarted) => *end_seen = false,
        (JobKind::Index, PushedEvent::IndexDone(_) | PushedEvent::IndexError(_)) => *end_seen = true,
        (JobKind::Analyze, PushedEvent::AnalyzeDone(done)) if done.job == job.id => *end_seen = true,
        (JobKind::Report, PushedEvent::ReportDone(done)) if done.job == job.id.0 => *end_seen = true,
        (JobKind::Generate, PushedEvent::GenerateDone(done)) if job.key.as_ref() == Some(&done.key) => *end_seen = true,
        // The stream fell behind and skipped events, the end among them, perhaps.
        (_, PushedEvent::Snapshot { .. }) => *end_seen = true,
        _ => {}
    }
}

fn api_error(e: ApiError) -> anyhow::Error {
    anyhow!(e.message)
}

pub fn op_error(e: OpError) -> anyhow::Error {
    match e {
        OpError::Unauthenticated => anyhow!("Not signed in to the server"),
        OpError::UnknownOperation(name) => anyhow!("No operation is named {name}"),
        OpError::TooLarge => anyhow!("The upload is too large"),
        OpError::BadRequest(m) | OpError::NotFound(m) | OpError::Conflict(m) | OpError::Internal(m) => anyhow!(m),
    }
}
