//! Runs operations where the route says: in this process on a Ctx, or over HTTP through the
//! client crate. Commands call `call`, `raw` and `run` and never care which.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde::Serialize;
use serde::de::DeserializeOwned;
use session_player_client::{Client, RawCall};
use session_player_core::ops::{Ctx, Job, JobId, Op, RawBody, ServerEvent};

use crate::route::Route;

pub enum Backend {
    InProcess(Arc<Ctx>),
    Http(Client),
}

pub struct OpenParams<'a> {
    pub route: Route,
    /// The data folder: the database for `Local`, `servers.json` for `Remote`.
    pub data: &'a Path,
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

impl Backend {
    /// In process: opens the Ctx and indexes in the foreground. Over HTTP: connects and checks
    /// the server's API version (`check_compat`).
    pub fn open(params: OpenParams) -> anyhow::Result<Backend> {
        let _ = params;
        todo!("Backend::open")
    }

    pub fn call<Req: Serialize, Res: DeserializeOwned>(&self, op: &Op<Req, Res>, req: &Req) -> anyhow::Result<Res> {
        let _ = (op, req);
        todo!("Backend::call")
    }

    pub fn raw(&self, call: RawCall) -> anyhow::Result<RawBody> {
        let _ = call;
        todo!("Backend::raw")
    }

    /// Starts a job and waits for it to leave Running. Over HTTP it subscribes to events before
    /// starting the job, then polls `job` for completion; it never infers completion from events.
    pub fn run(&self, params: RunParams) -> anyhow::Result<Job> {
        let _ = params;
        todo!("Backend::run")
    }
}
