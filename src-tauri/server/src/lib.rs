//! The Session Player server: `POST /api/v1/<operation>` and one event stream over the operations in
//! session-player-core. See .plans/server-api/ for the design.

pub mod auth;
pub mod listener;
pub mod lockfile;
pub mod pairing;
pub mod stream;
pub mod tls;

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use session_player_core::ops::Ctx;

pub use pairing::SERVER_EXTENSIONS;

// Used by the implementation that replaces the stubs.
use {axum_server as _, rcgen as _};

/// The loopback listener's port unless `server.json` says otherwise. The external listener
/// defaults to the next port, because a wildcard bind on a port conflicts with 127.0.0.1 on it.
pub const DEFAULT_PORT: u16 = 47800;

/// What `server.json` holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// The loopback listener (plain HTTP on 127.0.0.1). 0 binds any free port (tests only).
    pub port: u16,
    /// The external listener. Never equal to `port`; 0 binds any free port (tests only).
    pub external_port: u16,
    /// None: the server never listens off loopback.
    pub external_bind: Option<IpAddr>,
    /// The host name or address paired devices reach the server at. It goes into pairing
    /// strings and the generated certificate.
    pub advertise: Option<String>,
    pub tls: TlsMode,
}

impl Default for ServerConfig {
    fn default() -> ServerConfig {
        ServerConfig { port: DEFAULT_PORT, external_port: DEFAULT_PORT + 1, external_bind: None, advertise: None, tls: TlsMode::SelfSigned }
    }
}

/// How the external listener serves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum TlsMode {
    /// A certificate generated into `tls/`, pinned by fingerprint.
    SelfSigned,
    /// The user's own certificate and key (PEM).
    OwnCert { cert: PathBuf, key: PathBuf },
    /// Plain HTTP off loopback, only with `--allow-insecure-http`.
    Plain,
}

pub struct ServeParams {
    /// Opened with `SERVER_EXTENSIONS` among its extensions.
    pub ctx: Arc<Ctx>,
    /// The folder holding server.lock, server.token, devices.json and tls/.
    pub data: PathBuf,
    pub config: ServerConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundAddrs {
    pub loopback: SocketAddr,
    /// The external listener's address while it is open.
    pub external: Option<SocketAddr>,
}

pub struct ServerHandle {
    _todo: (),
}

impl ServerHandle {
    pub fn addrs(&self) -> BoundAddrs {
        todo!("ServerHandle::addrs")
    }

    /// Stops both listeners and releases server.lock before returning.
    pub fn shutdown(self) {
        todo!("ServerHandle::shutdown")
    }
}

/// Takes server.lock, writes a new server.token, binds the loopback listener and starts the
/// external listener controller on the server's own tokio runtime.
pub fn serve(params: ServeParams) -> anyhow::Result<ServerHandle> {
    let _ = params;
    todo!("serve")
}
