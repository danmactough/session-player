//! The one set of operations every caller reaches: the server's `POST /api/v1/<op>`, the in-process
//! CLI and the client crate. See .plans/server-api/00-design.md.

pub mod wire;

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

pub use wire::*;

use crate::generate::Claude;
use crate::model::SessionRow;

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
}

impl Ctx {
    /// Fails when two operations share a name. Does not index.
    pub fn open(_params: OpenParams) -> anyhow::Result<Arc<Ctx>> {
        todo!()
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// The events after `after`, waiting up to `wait` for one when there are none. An id older
    /// than the ring keeps gets one Snapshot instead.
    pub fn events_since(&self, _since: EventsSince) -> Vec<ServerEvent> {
        todo!()
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
pub fn specs(_ctx: &Ctx) -> Vec<OpSpec> {
    let _ = core_specs;
    todo!()
}

pub fn spec_of(_ctx: &Ctx, _name: &str) -> Option<OpSpec> {
    todo!()
}

/// Runs an operation by name. The HTTP server's one entry point.
pub fn dispatch(_ctx: &Arc<Ctx>, _params: DispatchParams) -> Result<Output, OpError> {
    todo!()
}

/// Runs a JSON operation in process, as the local device.
pub fn call<Req, Res>(_ctx: &Arc<Ctx>, op: &Op<Req, Res>, _req: &Req) -> Result<Res, OpError> {
    let _ = op.handler;
    todo!()
}

/// Runs a raw-body operation in process, as the local device.
pub fn call_raw(_ctx: &Arc<Ctx>, _name: &str, _input: Input) -> Result<Output, OpError> {
    todo!()
}

fn hello(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &HelloRequest) -> Result<HelloResponse, OpError> {
    todo!()
}

fn job(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &JobRequest) -> Result<Job, OpError> {
    todo!()
}

fn cancel_job(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &CancelJobRequest) -> Result<(), OpError> {
    todo!()
}

fn list_sessions(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &ListSessionsRequest) -> Result<Vec<SessionRow>, OpError> {
    todo!()
}

fn reindex(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &ReindexRequest) -> Result<JobId, OpError> {
    todo!()
}
