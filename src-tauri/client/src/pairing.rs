//! Pairing strings and the servers they pair with.

use anyhow::Context;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use session_player_core::ops::{ApiError, DeviceId, ErrorCode};

use crate::{Client, ClientParams};

/// `sp1.` + base64url (no padding) JSON of this struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingString {
    pub address: String,
    /// The port the listener actually bound.
    pub port: u16,
    /// None when the server serves plain HTTP (loopback-only pairing, or `--allow-insecure-http`).
    pub fingerprint: Option<String>,
    pub secret: String,
}

impl PairingString {
    pub fn parse(text: &str) -> anyhow::Result<PairingString> {
        let encoded = text.trim().strip_prefix("sp1.").context("A pairing string starts with sp1.")?;
        let json = URL_SAFE_NO_PAD.decode(encoded).context("The pairing string is damaged")?;
        serde_json::from_slice(&json).context("The pairing string is damaged")
    }

    pub fn encode(&self) -> String {
        format!("sp1.{}", URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("a pairing string serializes")))
    }

    pub fn url(&self) -> String {
        let scheme = if self.fingerprint.is_some() { "https" } else { "http" };
        format!("{scheme}://{}:{}", self.address, self.port)
    }
}

/// A server this machine paired with, as `servers.json` keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedServer {
    pub name: String,
    pub address: String,
    pub port: u16,
    pub fingerprint: Option<String>,
    pub token: String,
    pub device: DeviceId,
}

pub struct ConnectParams<'a> {
    pub pairing: &'a str,
    /// The server lists this machine under this name; it is also the returned server's name.
    pub name: &'a str,
}

/// Redeems a pairing string. A pinned fingerprint that doesn't match fails before the secret is
/// sent.
pub fn connect(params: ConnectParams) -> Result<PairedServer, ApiError> {
    let pairing = PairingString::parse(params.pairing).map_err(|e| ApiError { code: ErrorCode::BadRequest, message: format!("{e:#}") })?;
    let client = Client::new(ClientParams { url: pairing.url(), token: None, fingerprint: pairing.fingerprint.clone() })
        .map_err(|e| ApiError { code: ErrorCode::BadRequest, message: format!("{e:#}") })?;
    let answer: ConnectAnswer = client.call_named("connect", &ConnectAsk { secret: &pairing.secret, name: params.name })?;
    let token = answer.token.ok_or_else(|| ApiError { code: ErrorCode::Internal, message: "The server sent no device token.".to_string() })?;
    Ok(PairedServer { name: params.name.to_string(), address: pairing.address, port: pairing.port, fingerprint: pairing.fingerprint, token, device: answer.device })
}

/// The server crate's ConnectRequest and ConnectResponse, which this crate can't depend on.
#[derive(Serialize)]
struct ConnectAsk<'a> {
    secret: &'a str,
    name: &'a str,
}

#[derive(Deserialize)]
struct ConnectAnswer {
    device: DeviceId,
    token: Option<String>,
}
