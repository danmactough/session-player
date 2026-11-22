//! The remote servers this machine paired with, as the page sees them: never their tokens.

use serde::Serialize;
use session_player_client::{Client, PairedServer, ServersStore, check_compat};
use session_player_core::ops::{HELLO, HelloRequest};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerRef {
    pub name: String,
    pub address: String,
    pub port: u16,
    /// Whether the connection is TLS with a pinned certificate.
    pub pinned: bool,
}

impl From<&PairedServer> for ServerRef {
    fn from(server: &PairedServer) -> ServerRef {
        ServerRef { name: server.name.clone(), address: server.address.clone(), port: server.port, pinned: server.fingerprint.is_some() }
    }
}

pub fn list(store: &ServersStore) -> Result<Vec<ServerRef>, String> {
    Ok(store.list().map_err(|e| format!("{e:#}"))?.iter().map(ServerRef::from).collect())
}

pub struct ConnectParams<'a> {
    pub store: &'a ServersStore,
    pub pairing: &'a str,
    /// What to save the server as; None uses its address.
    pub name: Option<&'a str>,
    /// What the server lists this machine as.
    pub device_name: &'a str,
}

/// Redeems the pairing string, checks the server speaks this API version, and saves it.
pub fn connect(params: ConnectParams) -> Result<ServerRef, String> {
    let mut server = session_player_client::connect(session_player_client::ConnectParams { pairing: params.pairing, name: params.device_name }).map_err(|e| e.message)?;
    server.name = match params.name.map(str::trim) {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => server.address.clone(),
    };
    let client = Client::paired(&server).map_err(|e| format!("{e:#}"))?;
    let hello = client.call(&HELLO, &HelloRequest {}).map_err(|e| e.message)?;
    check_compat(&hello)?;
    let saved = ServerRef::from(&server);
    params.store.save(server).map_err(|e| format!("{e:#}"))?;
    Ok(saved)
}

/// Forgets the server. Its device token stays valid on the server until it is unpaired there.
pub fn remove(store: &ServersStore, name: &str) -> Result<(), String> {
    if store.remove(name).map_err(|e| format!("{e:#}"))? {
        Ok(())
    } else {
        Err(format!("No paired server is named {name:?}."))
    }
}
