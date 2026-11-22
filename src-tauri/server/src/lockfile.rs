//! `server.lock`: one server per data folder. Taken with `File::try_lock` before binding, with port 0
//! in it, and rewritten with the bound port afterwards. `probe_local` (client crate) reads it.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The file's JSON content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockContent {
    pub pid: u32,
    /// 0 until the loopback listener is bound.
    pub port: u16,
}

/// Held for as long as the server runs; dropping it releases the lock.
pub struct ServerLock {
    _todo: (),
}

/// Fails when another server holds the lock, without touching the file's content.
pub fn acquire(data: &Path) -> anyhow::Result<ServerLock> {
    let _ = data;
    todo!("lockfile::acquire")
}

impl ServerLock {
    pub fn record_port(&mut self, port: u16) -> anyhow::Result<()> {
        let _ = port;
        todo!("ServerLock::record_port")
    }
}
