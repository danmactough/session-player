//! Calls the Session Player server over HTTP, with certificate pinning for a remote one.
//! See .plans/server-api/ for the design.

mod pairing;
mod probe;
mod store;

use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use session_player_core::ops::{ApiError, HelloResponse, Op, RawBody, RawData, ServerEvent};

pub use pairing::{ConnectParams, PairedServer, PairingString, connect};
pub use probe::{LocalProbe, probe_local};
pub use store::ServersStore;

// Used by the implementation that replaces the stubs.
use {http_body_util as _, hyper_util as _, tokio_rustls as _};

pub struct ClientParams {
    /// `http://127.0.0.1:{port}` for the local server, `https://{host}:{port}` for a paired one.
    pub url: String,
    pub token: Option<String>,
    /// Lowercase hex SHA-256 of the server's certificate. Only this certificate is accepted, and
    /// the handshake signature is still verified.
    pub fingerprint: Option<String>,
}

/// A synchronous client. Its tokio runtime is shared (`Arc`) with every body and event stream it
/// returns, so those keep working after the Client is dropped.
pub struct Client {
    _todo: (),
}

/// An operation with a raw body, by name.
pub struct RawCall {
    pub name: String,
    pub body: RawData,
    pub content_type: String,
    /// Sent as the URL query; a raw operation's options.
    pub query: Vec<(String, String)>,
}

/// A response whatever its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawAnswer {
    pub status: u16,
    /// Lowercase names, in the order received.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RawAnswer {
    /// The first header named `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        let _ = name;
        todo!("RawAnswer::header")
    }
}

impl Client {
    pub fn new(params: ClientParams) -> anyhow::Result<Client> {
        let _ = params;
        todo!("Client::new")
    }

    /// The local server's loopback listener.
    pub fn local(port: u16, token: &str) -> anyhow::Result<Client> {
        let _ = (port, token);
        todo!("Client::local")
    }

    pub fn paired(server: &PairedServer) -> anyhow::Result<Client> {
        let _ = server;
        todo!("Client::paired")
    }

    pub fn call<Req: Serialize, Res: DeserializeOwned>(&self, op: &Op<Req, Res>, req: &Req) -> Result<Res, ApiError> {
        let _ = (op, req);
        todo!("Client::call")
    }

    /// A JSON operation by name: the server crate's extensions, which have no typed `Op`.
    pub fn call_named<Req: Serialize, Res: DeserializeOwned>(&self, name: &str, req: &Req) -> Result<Res, ApiError> {
        let _ = (name, req);
        todo!("Client::call_named")
    }

    /// A non-2xx answer is the server's ApiError.
    pub fn call_raw(&self, call: RawCall) -> Result<RawBody, ApiError> {
        let _ = call;
        todo!("Client::call_raw")
    }

    /// Any status comes back as a RawAnswer; only a failure to reach the server is an error.
    pub fn call_raw_answer(&self, call: RawCall) -> Result<RawAnswer, ApiError> {
        let _ = call;
        todo!("Client::call_raw_answer")
    }

    /// `GET {path}` with the token, such as an image URL from `load_session`.
    pub fn get(&self, path: &str) -> Result<RawAnswer, ApiError> {
        let _ = path;
        todo!("Client::get")
    }

    /// `GET /api/v1/events`, resuming after `last_id` (sent as `Last-Event-ID`).
    pub fn events(&self, last_id: Option<u64>) -> Result<EventStream, ApiError> {
        let _ = last_id;
        todo!("Client::events")
    }
}

/// The server's events in order. Iterating blocks until the next event.
pub struct EventStream {
    _todo: (),
}

impl EventStream {
    /// The next event, or None when none arrives within `wait`.
    pub fn next_within(&mut self, wait: Duration) -> Option<Result<ServerEvent, ApiError>> {
        let _ = wait;
        todo!("EventStream::next_within")
    }
}

impl Iterator for EventStream {
    type Item = Result<ServerEvent, ApiError>;

    fn next(&mut self) -> Option<Self::Item> {
        todo!("EventStream::next")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compat {
    Exact,
    /// Same `api_version`, higher `api_minor`: the server has operations this client doesn't use.
    NewerServer,
    /// Same `api_version`, lower `api_minor`: some operations this client knows may be missing.
    OlderServer,
}

/// Err (saying which side to upgrade and naming the server's version) when `api_version` differs.
pub fn check_compat(hello: &HelloResponse) -> Result<Compat, String> {
    let _ = hello;
    todo!("check_compat")
}
