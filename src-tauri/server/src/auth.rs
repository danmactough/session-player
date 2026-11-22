//! Who is calling, and whether the Host and Origin of a request are ones this server answers.
//! Every check here runs before the router learns which operation was asked for.

use axum::response::{IntoResponse, Response};
use session_player_core::ops::{ApiError, DeviceId, ErrorCode};

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
    let device_of = |hash: [u8; 32]| params.devices.iter().find(|(_, h)| *h == hash).map(|(id, _)| id.clone());
    if let Some(authorization) = params.authorization {
        let token = authorization.strip_prefix("Bearer ").ok_or(Refusal::Unauthenticated)?;
        let hash = hash_token(token);
        if params.listener == Listener::Loopback && hash == *params.local_token {
            return Ok(DeviceId("local".to_string()));
        }
        return device_of(hash).ok_or(Refusal::Unauthenticated);
    }
    let token = params.cookie.and_then(cookie_value).ok_or(Refusal::Unauthenticated)?;
    let device = device_of(hash_token(token)).ok_or(Refusal::Unauthenticated)?;
    // A browser sends the cookie with any request to this origin, including ones another site
    // makes it send, so a request that changes something must prove it came from our own page.
    if params.method != "GET" {
        let own = params.origin.is_some_and(|o| params.own_origins.iter().any(|own| own == o));
        if params.csrf.is_none() || !own {
            return Err(Refusal::Forbidden);
        }
    }
    Ok(device)
}

fn cookie_value(header: &str) -> Option<&str> {
    header.split(';').filter_map(|pair| pair.trim().split_once('=')).find(|(name, _)| *name == COOKIE).map(|(_, value)| value)
}

pub fn error_response(code: ErrorCode, message: &str) -> Response {
    let status = axum::http::StatusCode::from_u16(code.status()).expect("a valid status");
    (status, axum::Json(ApiError { code, message: message.to_string() })).into_response()
}

/// The one 401 response, so a refused caller can't tell a wrong token from a missing one or an
/// unknown operation from a known one.
pub fn unauthenticated() -> Response {
    error_response(ErrorCode::Unauthenticated, "A valid token is required.")
}

pub fn forbidden() -> Response {
    error_response(ErrorCode::ForbiddenOrigin, "This request isn't allowed from here.")
}

pub fn hash_token(token: &str) -> [u8; 32] {
    ring::digest::digest(&ring::digest::SHA256, token.as_bytes()).as_ref().try_into().expect("SHA-256 is 32 bytes")
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn from_hex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
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
    let Some(host) = params.host else { return false };
    let port = params.port;
    match params.listener {
        Listener::Loopback => host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}"),
        Listener::External => params.advertise.is_some_and(|a| host == format!("{a}:{port}")),
    }
}

/// CORS: the app's webview (`tauri://localhost` on macOS, `http://tauri.localhost` on Linux and
/// Windows) and the listener's own origins, never `*`.
pub fn origin_allowed(origin: &str, own_origins: &[String]) -> bool {
    origin == "tauri://localhost" || origin == "http://tauri.localhost" || own_origins.iter().any(|o| o == origin)
}
