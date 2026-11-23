//! `servers.json`: the servers this machine paired with. 0600, written atomically (temp file +
//! rename). An unparsable file is an error, never silently replaced.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use crate::PairedServer;

pub struct ServersStore {
    file: PathBuf,
}

impl ServersStore {
    /// The store at `{dir}/servers.json`. Opening reads nothing.
    pub fn new(dir: &Path) -> ServersStore {
        ServersStore { file: dir.join("servers.json") }
    }

    /// Empty when the file doesn't exist.
    pub fn list(&self) -> anyhow::Result<Vec<PairedServer>> {
        match std::fs::read_to_string(&self.file) {
            Ok(text) => serde_json::from_str(&text).with_context(|| format!("Couldn't read {}; fix or remove it", self.file.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e).with_context(|| format!("Couldn't read {}", self.file.display())),
        }
    }

    pub fn get(&self, name: &str) -> anyhow::Result<Option<PairedServer>> {
        Ok(self.list()?.into_iter().find(|s| s.name == name))
    }

    /// Adds `server`, or replaces the one with its name in place. Refuses to replace a pinned
    /// fingerprint with none.
    pub fn save(&self, server: PairedServer) -> anyhow::Result<()> {
        let mut servers = self.list()?;
        match servers.iter_mut().find(|s| s.name == server.name) {
            Some(existing) => {
                if existing.fingerprint.is_some() && server.fingerprint.is_none() {
                    bail!("{} is pinned to a certificate; a pairing without one can't replace it. Remove it first.", server.name);
                }
                *existing = server;
            }
            None => servers.push(server),
        }
        self.write(&servers)
    }

    /// Whether a server had that name.
    pub fn remove(&self, name: &str) -> anyhow::Result<bool> {
        let mut servers = self.list()?;
        let before = servers.len();
        servers.retain(|s| s.name != name);
        if servers.len() == before {
            return Ok(false);
        }
        self.write(&servers)?;
        Ok(true)
    }

    fn write(&self, servers: &[PairedServer]) -> anyhow::Result<()> {
        use std::io::Write;
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("Couldn't create {}", dir.display()))?;
        }
        let temp = self.file.with_extension(format!("json.{}.tmp", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let result = (|| {
            let mut out = options.open(&temp)?;
            out.write_all(&serde_json::to_vec_pretty(servers)?)?;
            out.sync_all()?;
            std::fs::rename(&temp, &self.file)?;
            anyhow::Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result.with_context(|| format!("Couldn't write {}", self.file.display()))
    }
}
