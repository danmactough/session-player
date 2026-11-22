//! Pairing offers and paired devices: the `pair`, `connect`, `devices` and `unpair` operations the
//! server adds to the core set.
//!
//! An offer is a 32-byte CSPRNG secret, good once, for 300 s by the Ctx clock. At most 5 are open;
//! 5 wrong secrets wipe them all. Every failed connect is the same `Unauthenticated`.
//! Devices live in `devices.json` (0600, temp file + rename, SHA-256 token hashes only).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use session_player_core::ops::{Access, Body, Ctx, DeviceId, Extension, Input, OpError, OpSpec, Output};

pub const OFFER_LIFETIME_MS: u64 = 300_000;
pub const MAX_OPEN_OFFERS: usize = 5;
pub const MAX_WRONG_SECRETS: usize = 5;

pub const SERVER_EXTENSIONS: &[Extension] = &[
    Extension { spec: OpSpec { name: "pair", body: Body::Json, inline: false, access: Access::Authenticated }, handler: pair },
    Extension { spec: OpSpec { name: "connect", body: Body::Json, inline: false, access: Access::Anonymous }, handler: connect },
    Extension { spec: OpSpec { name: "devices", body: Body::Json, inline: false, access: Access::Authenticated }, handler: devices },
    Extension { spec: OpSpec { name: "unpair", body: Body::Json, inline: false, access: Access::Authenticated }, handler: unpair },
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairResponse {
    /// `sp1.` + base64url JSON: address, port, fingerprint, secret.
    pub pairing: String,
    /// Ms by the Ctx clock.
    pub expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectRequest {
    pub secret: String,
    /// What `devices` lists the new device as.
    pub name: String,
    /// A browser gets its token as an HttpOnly cookie instead of in the body.
    #[serde(default)]
    pub browser: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectResponse {
    pub device: DeviceId,
    /// None for a browser.
    pub token: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicesRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub paired_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnpairRequest {
    pub id: DeviceId,
}

fn pair(ctx: &Arc<Ctx>, device: &DeviceId, input: Input) -> Result<Output, OpError> {
    let _ = (ctx, device, input);
    todo!("pair")
}

fn connect(ctx: &Arc<Ctx>, device: &DeviceId, input: Input) -> Result<Output, OpError> {
    let _ = (ctx, device, input);
    todo!("connect")
}

fn devices(ctx: &Arc<Ctx>, device: &DeviceId, input: Input) -> Result<Output, OpError> {
    let _ = (ctx, device, input);
    todo!("devices")
}

fn unpair(ctx: &Arc<Ctx>, device: &DeviceId, input: Input) -> Result<Output, OpError> {
    let _ = (ctx, device, input);
    todo!("unpair")
}
