//! `servers.json`: the servers this machine paired with. 0600, written atomically (temp file +
//! rename). An unparsable file is an error, never silently replaced.

use std::path::{Path, PathBuf};

use crate::PairedServer;

pub struct ServersStore {
    _todo: PathBuf,
}

impl ServersStore {
    /// The store at `{dir}/servers.json`. Opening reads nothing.
    pub fn new(dir: &Path) -> ServersStore {
        let _ = dir;
        todo!("ServersStore::new")
    }

    /// Empty when the file doesn't exist.
    pub fn list(&self) -> anyhow::Result<Vec<PairedServer>> {
        todo!("ServersStore::list")
    }

    pub fn get(&self, name: &str) -> anyhow::Result<Option<PairedServer>> {
        let _ = name;
        todo!("ServersStore::get")
    }

    /// Adds `server`, or replaces the one with its name in place. Refuses to replace a pinned
    /// fingerprint with none.
    pub fn save(&self, server: PairedServer) -> anyhow::Result<()> {
        let _ = server;
        todo!("ServersStore::save")
    }

    /// Whether a server had that name.
    pub fn remove(&self, name: &str) -> anyhow::Result<bool> {
        let _ = name;
        todo!("ServersStore::remove")
    }
}
