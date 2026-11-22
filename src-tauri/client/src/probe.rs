//! Whether a server runs on this machine's data folder, read from `server.lock` and `server.token`.

use std::fs::{File, TryLockError};
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalProbe {
    /// `token` is None when `server.token` can't be read.
    Live { port: u16, token: Option<String> },
    NotLive,
}

/// Tests the lock with a shared try-lock, so a probe never blocks a server start. While the lock is
/// held with port 0 (the server is binding), rereads it for up to 500 ms; never Live with port 0.
pub fn probe_local(data: &Path) -> LocalProbe {
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        match read_live_port(data) {
            Held::NotHeld => return LocalProbe::NotLive,
            Held::Port(port) => {
                let token = std::fs::read_to_string(data.join("server.token")).ok().map(|t| t.trim().to_string());
                return LocalProbe::Live { port, token };
            }
            Held::Starting if Instant::now() >= deadline => return LocalProbe::NotLive,
            Held::Starting => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

#[derive(Deserialize)]
struct LockContent {
    port: u16,
}

enum Held {
    NotHeld,
    /// Held, with no port in it yet (or a partly written file).
    Starting,
    Port(u16),
}

fn read_live_port(data: &Path) -> Held {
    let Ok(mut file) = File::open(data.join("server.lock")) else { return Held::NotHeld };
    match file.try_lock_shared() {
        // Nobody holds it; the shared lock is released when `file` drops.
        Ok(()) => return Held::NotHeld,
        Err(TryLockError::WouldBlock) => {}
        Err(TryLockError::Error(_)) => return Held::NotHeld,
    }
    let mut text = String::new();
    if file.read_to_string(&mut text).is_err() {
        return Held::Starting;
    }
    match serde_json::from_str::<LockContent>(&text) {
        Ok(content) if content.port != 0 => Held::Port(content.port),
        _ => Held::Starting,
    }
}
