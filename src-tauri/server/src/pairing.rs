//! Pairing offers and paired devices: the `pair`, `connect`, `devices` and `unpair` operations the
//! server adds to the core set.
//!
//! `pair`, `devices` and `unpair` are local-only: a paired device can't mint, list or revoke devices.
//!
//! An offer is a 32-byte CSPRNG secret, good once, for 300 s by the Ctx clock. At most 5 are open;
//! 5 wrong secrets wipe them all. Every failed connect is the same `Unauthenticated`.
//! Devices live in `devices.json` (0600, temp file + rename, SHA-256 token hashes only).

use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use session_player_core::ops::{Access, Body, Ctx, Device, DeviceId, Extension, Input, OpError, OpSpec, Offer, Output};

use crate::auth::{hash_token, hex};
use crate::listener::PAIR_WAIT;

pub const OFFER_LIFETIME_MS: u64 = 300_000;
pub const MAX_OPEN_OFFERS: usize = 5;
pub const MAX_WRONG_SECRETS: usize = 5;

pub const SERVER_EXTENSIONS: &[Extension] = &[
    Extension { spec: OpSpec { name: "pair", body: Body::Json, inline: false, access: Access::LocalOnly }, handler: pair },
    Extension { spec: OpSpec { name: "connect", body: Body::Json, inline: false, access: Access::Anonymous }, handler: connect },
    Extension { spec: OpSpec { name: "devices", body: Body::Json, inline: false, access: Access::LocalOnly }, handler: devices },
    Extension { spec: OpSpec { name: "unpair", body: Body::Json, inline: false, access: Access::LocalOnly }, handler: unpair },
    crate::service::SERVICE,
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

fn request<T: DeserializeOwned>(input: Input) -> Result<T, OpError> {
    match input {
        Input::Json(value) => serde_json::from_value(value).map_err(|e| OpError::BadRequest(e.to_string())),
        Input::Raw { .. } => Err(OpError::BadRequest("This operation takes a JSON body".to_string())),
    }
}

fn respond<T: Serialize>(value: T) -> Result<Output, OpError> {
    serde_json::to_value(value).map(Output::Json).map_err(|e| OpError::Internal(e.to_string()))
}

fn internal(e: anyhow::Error) -> OpError {
    OpError::Internal(format!("{e:#}"))
}

/// The JSON inside a pairing string, as the client crate's `PairingString` reads it.
#[derive(Serialize)]
struct PairingFields<'a> {
    address: &'a str,
    port: u16,
    fingerprint: Option<&'a str>,
    secret: &'a str,
}

fn pair(ctx: &Arc<Ctx>, _device: &DeviceId, input: Input) -> Result<Output, OpError> {
    let _: PairRequest = request(input)?;
    let server = crate::running(ctx).ok_or_else(|| OpError::Conflict("Pairing needs a running server (sp serve).".to_string()))?;
    let now = ctx.now();
    let secret = crate::random_token(32).map_err(internal)?;
    let expires_at = now + OFFER_LIFETIME_MS;
    {
        let mut offers = ctx.offers();
        offers.open.retain(|o| o.expires_at > now);
        if offers.open.len() >= MAX_OPEN_OFFERS {
            return Err(OpError::Conflict(format!("{MAX_OPEN_OFFERS} pairing offers are already open. Use one or wait for them to expire.")));
        }
        offers.open.push(Offer { secret_sha256: hex(&hash_token(&secret)), expires_at });
    }
    server.nudge.notify_one();
    let (address, port, fingerprint) = match server.config.external_bind {
        None => ("127.0.0.1".to_string(), server.loopback.port(), None),
        Some(_) => {
            let deadline = std::time::Instant::now() + PAIR_WAIT;
            let external = loop {
                if let Some(addr) = server.external() {
                    break addr;
                }
                if std::time::Instant::now() >= deadline {
                    return Err(OpError::Internal("The external listener didn't open. See the server log.".to_string()));
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            (server.advertise.clone().unwrap_or_default(), external.port(), server.fingerprint.clone())
        }
    };
    let fields = PairingFields { address: &address, port, fingerprint: fingerprint.as_deref(), secret: &secret };
    let json = serde_json::to_vec(&fields).map_err(|e| OpError::Internal(e.to_string()))?;
    respond(PairResponse { pairing: format!("sp1.{}", URL_SAFE_NO_PAD.encode(json)), expires_at })
}

fn connect(ctx: &Arc<Ctx>, _device: &DeviceId, input: Input) -> Result<Output, OpError> {
    let req: ConnectRequest = request(input)?;
    let now = ctx.now();
    let presented = hex(&hash_token(&req.secret));
    {
        let mut offers = ctx.offers();
        offers.open.retain(|o| o.expires_at > now);
        match offers.open.iter().position(|o| o.secret_sha256 == presented) {
            Some(i) => {
                offers.open.remove(i);
            }
            None => {
                offers.wrong += 1;
                if offers.wrong >= MAX_WRONG_SECRETS {
                    offers.open.clear();
                    offers.wrong = 0;
                }
                return Err(OpError::Unauthenticated);
            }
        }
    }
    let id = DeviceId(crate::random_hex(16).map_err(internal)?);
    let token = crate::random_token(32).map_err(internal)?;
    ctx.add_device(Device { id: id.clone(), name: req.name, token_sha256: hex(&hash_token(&token)), paired_at: now }).map_err(internal)?;
    if let Some(server) = crate::running(ctx) {
        server.nudge.notify_one();
    }
    // The HTTP layer moves the token into a cookie for a browser.
    respond(ConnectResponse { device: id, token: Some(token) })
}

fn devices(ctx: &Arc<Ctx>, _device: &DeviceId, input: Input) -> Result<Output, OpError> {
    let _: DevicesRequest = request(input)?;
    let listed: Vec<DeviceInfo> = ctx.devices().into_iter().map(|d| DeviceInfo { id: d.id, name: d.name, paired_at: d.paired_at }).collect();
    respond(listed)
}

fn unpair(ctx: &Arc<Ctx>, _device: &DeviceId, input: Input) -> Result<Output, OpError> {
    let req: UnpairRequest = request(input)?;
    if !ctx.remove_device(&req.id).map_err(internal)? {
        return Err(OpError::NotFound(format!("No paired device {}", req.id.0)));
    }
    if let Some(server) = crate::running(ctx) {
        server.nudge.notify_one();
        server.revoked.send_modify(|n| *n += 1);
    }
    respond(())
}
