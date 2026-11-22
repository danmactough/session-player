//! Whether a server runs on this machine's data folder, read from `server.lock` and `server.token`.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalProbe {
    /// `token` is None when `server.token` can't be read.
    Live { port: u16, token: Option<String> },
    NotLive,
}

/// Tests the lock with a shared try-lock, so a probe never blocks a server start. While the lock is
/// held with port 0 (the server is binding), rereads it for up to 500 ms; never Live with port 0.
pub fn probe_local(data: &Path) -> LocalProbe {
    let _ = data;
    todo!("probe_local")
}
