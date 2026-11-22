//! Paired devices (`devices.json`) and open pairing offers. The server crate hashes tokens and
//! secrets before they reach here, so this file only ever holds SHA-256 hex.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::{Ctx, DeviceId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    /// Lowercase hex SHA-256 of the device's token.
    pub token_sha256: String,
    pub paired_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// Lowercase hex SHA-256 of the offer's secret.
    pub secret_sha256: String,
    pub expires_at: u64,
}

#[derive(Debug, Default)]
pub struct Offers {
    pub open: Vec<Offer>,
    /// Wrong secrets presented since the offers were last wiped.
    pub wrong: usize,
}

pub(super) struct DeviceStore {
    file: PathBuf,
    devices: Mutex<Vec<Device>>,
}

impl DeviceStore {
    pub(super) fn open(data: &Path) -> anyhow::Result<DeviceStore> {
        let file = data.join("devices.json");
        let devices = match std::fs::read_to_string(&file) {
            Ok(text) => serde_json::from_str(&text).with_context(|| format!("Couldn't read {}", file.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e).with_context(|| format!("Couldn't read {}", file.display())),
        };
        Ok(DeviceStore { file, devices: Mutex::new(devices) })
    }
}

/// Writes `devices` to a temp file beside `file` and renames it over, so a reader never sees half a
/// file and the old one is never edited in place.
fn write(file: &Path, devices: &[Device]) -> anyhow::Result<()> {
    use std::io::Write;
    let temp = file.with_file_name(format!("devices.json.{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut out = options.open(&temp)?;
        out.write_all(&serde_json::to_vec_pretty(devices)?)?;
        out.sync_all()?;
        std::fs::rename(&temp, file)?;
        anyhow::Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.with_context(|| format!("Couldn't write {}", file.display()))
}

impl Ctx {
    pub fn devices(&self) -> Vec<Device> {
        self.devices.devices.lock().unwrap().clone()
    }

    pub fn add_device(&self, device: Device) -> anyhow::Result<()> {
        let mut devices = self.devices.devices.lock().unwrap();
        let mut next = devices.clone();
        next.push(device);
        write(&self.devices.file, &next)?;
        *devices = next;
        Ok(())
    }

    /// Whether a device had that id.
    pub fn remove_device(&self, id: &DeviceId) -> anyhow::Result<bool> {
        let mut devices = self.devices.devices.lock().unwrap();
        if !devices.iter().any(|d| d.id == *id) {
            return Ok(false);
        }
        let next: Vec<Device> = devices.iter().filter(|d| d.id != *id).cloned().collect();
        write(&self.devices.file, &next)?;
        *devices = next;
        Ok(true)
    }

    /// The open pairing offers. Expired ones stay until the caller drops them.
    pub fn offers(&self) -> MutexGuard<'_, Offers> {
        self.offers.lock().unwrap()
    }

    /// Set by the server while its external listener is open; `hello` reports it.
    pub fn set_external_open(&self, open: bool) {
        self.external_open.store(open, std::sync::atomic::Ordering::SeqCst);
    }
}
