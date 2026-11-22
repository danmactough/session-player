//! `server.lock`: one server per data folder. Taken with `File::try_lock` before binding, with port 0
//! in it, and rewritten with the bound port afterwards. `probe_local` (client crate) reads it.

use std::fs::{File, TryLockError};
use std::io::{Seek, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};

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
    file: File,
}

/// A probe holds a shared lock for a moment while it checks; a start retries this long before
/// deciding another server holds the lock.
const PROBE_GRACE: Duration = Duration::from_millis(300);

/// Fails when another server holds the lock, without touching the file's content.
pub fn acquire(data: &Path) -> anyhow::Result<ServerLock> {
    std::fs::create_dir_all(data).with_context(|| format!("Couldn't create {}", data.display()))?;
    let path = data.join("server.lock");
    let file = File::options().read(true).write(true).create(true).truncate(false).open(&path).with_context(|| format!("Couldn't open {}", path.display()))?;
    let deadline = Instant::now() + PROBE_GRACE;
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Err(TryLockError::WouldBlock) => bail!("Another server is already running on {}", data.display()),
            Err(TryLockError::Error(e)) => return Err(e).with_context(|| format!("Couldn't lock {}", path.display())),
        }
    }
    let mut lock = ServerLock { file };
    lock.record_port(0)?;
    Ok(lock)
}

impl ServerLock {
    pub fn record_port(&mut self, port: u16) -> anyhow::Result<()> {
        let content = serde_json::to_vec(&LockContent { pid: std::process::id(), port })?;
        self.file.set_len(0)?;
        self.file.rewind()?;
        self.file.write_all(&content)?;
        Ok(())
    }
}
