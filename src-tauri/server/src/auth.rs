//! Who is calling, and whether the Host and Origin of a request are ones this server answers.
//! Every check here runs before the router learns which operation was asked for.

use session_player_core::ops::DeviceId;

/// The browser cookie that carries a paired browser's device token.
pub const COOKIE: &str = "sp_session";

/// A cookie-authenticated request other than GET must carry this header (any value) and an Origin
/// equal to the listener's own.
pub const CSRF_HEADER: &str = "x-sp-csrf";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listener {
    /// Plain HTTP on 127.0.0.1. The only listener that accepts `server.token`.
    Loopback,
    External,
}

pub struct CheckParams<'a> {
    pub listener: Listener,
    pub method: &'a str,
    /// The `Authorization` header.
    pub authorization: Option<&'a str>,
    /// The `Cookie` header.
    pub cookie: Option<&'a str>,
    /// The `x-sp-csrf` header.
    pub csrf: Option<&'a str>,
    pub origin: Option<&'a str>,
    /// The origins this listener serves pages from.
    pub own_origins: &'a [String],
    /// SHA-256 of this run's `server.token`.
    pub local_token: &'a [u8; 32],
    /// SHA-256 of each paired device's token.
    pub devices: &'a [(DeviceId, [u8; 32])],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// 401 with the body `unauthenticated()` gives.
    Unauthenticated,
    /// 403: a cookie request without the CSRF header or with another Origin.
    Forbidden,
}

/// The device a request authenticates as.
pub fn check(params: CheckParams) -> Result<DeviceId, Refusal> {
    let _ = params;
    todo!("auth::check")
}

/// The one 401 response, so a refused caller can't tell a wrong token from a missing one or an
/// unknown operation from a known one.
pub fn unauthenticated() -> axum::response::Response {
    todo!("auth::unauthenticated")
}

pub fn hash_token(token: &str) -> [u8; 32] {
    let _ = token;
    todo!("auth::hash_token")
}

pub struct HostParams<'a> {
    pub listener: Listener,
    /// The `Host` header, or the `:authority` of an HTTP/2 request.
    pub host: Option<&'a str>,
    pub port: u16,
    pub advertise: Option<&'a str>,
}

/// Loopback: `127.0.0.1:{port}` or `localhost:{port}`. External: `{advertise}:{port}`.
pub fn host_allowed(params: HostParams) -> bool {
    let _ = params;
    todo!("auth::host_allowed")
}

/// CORS: `tauri://localhost` (the app's webview) and the listener's own origins, never `*`.
pub fn origin_allowed(origin: &str, own_origins: &[String]) -> bool {
    let _ = (origin, own_origins);
    todo!("auth::origin_allowed")
}
