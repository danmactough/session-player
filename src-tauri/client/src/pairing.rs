//! Pairing strings and the servers they pair with.

use serde::{Deserialize, Serialize};
use session_player_core::ops::{ApiError, DeviceId};

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
        let _ = text;
        todo!("PairingString::parse")
    }

    pub fn encode(&self) -> String {
        todo!("PairingString::encode")
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
    let _ = params;
    todo!("connect")
}
