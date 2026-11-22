//! `sp service install|uninstall|start|stop|status`: a per-user LaunchAgent (macOS) or systemd user
//! unit (Linux) running `sp serve`. Rendering is pure; launchctl and systemctl run behind
//! `ServiceManager`, so tests use a fake. The `service` operation is never reachable over HTTP.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use session_player_core::ops::{Access, Body, Ctx, DeviceId, Extension, Input, OpError, OpSpec, Output};

/// The LaunchAgent's label; its plist is `~/Library/LaunchAgents/{LAUNCHD_LABEL}.plist`.
pub const LAUNCHD_LABEL: &str = "com.danmactough.sessionplayer.server";

/// The systemd user unit, at `~/.config/systemd/user/{SYSTEMD_UNIT}`.
pub const SYSTEMD_UNIT: &str = "session-player.service";

pub const SERVICE: Extension = Extension { spec: OpSpec { name: "service", body: Body::Json, inline: false, access: Access::NotOverHttp }, handler: service };

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Launchd,
    Systemd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceAction {
    Install,
    Uninstall,
    Start,
    Stop,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceRequest {
    pub action: ServiceAction,
}

/// What the unit file says.
pub struct UnitParams<'a> {
    /// The absolute path of the `sp` binary.
    pub sp: &'a Path,
    /// The logs folder (where `analyze.log` is); the server logs to `server.log` in it.
    pub logs: &'a Path,
    /// PATH for the server, so it finds `claude` (see `service_path`).
    pub path: &'a str,
    /// Set as `SESSION_PLAYER_DATA_DIR` when the installing sp had it set.
    pub data_dir: Option<&'a Path>,
}

pub struct ServicePathParams<'a> {
    /// The installing process's PATH.
    pub current: Option<&'a str>,
    pub home: &'a Path,
}

/// The current PATH, then `~/.local/bin` and `~/.claude/local` (where Claude Code installs
/// `claude`) when missing from it, without duplicates.
pub fn service_path(_params: ServicePathParams) -> String {
    todo!("PATH for the service")
}

/// The systemd user unit's text.
pub fn systemd_unit(_params: &UnitParams) -> String {
    todo!("render the systemd user unit")
}

/// The LaunchAgent plist's text, with every value XML-escaped.
pub fn launchd_plist(_params: &UnitParams) -> String {
    todo!("render the launchd plist")
}

/// Where the unit file lives under `home`.
pub fn unit_file(_platform: Platform, _home: &Path) -> PathBuf {
    todo!("the unit file's path")
}

/// What running launchctl or systemctl gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub success: bool,
    pub stdout: String,
}

/// Runs `launchctl` or `systemctl`.
pub trait ServiceManager {
    fn run(&mut self, program: &str, args: &[String]) -> anyhow::Result<CommandResult>;
}

/// The real launchctl and systemctl.
pub struct SystemManager;

impl ServiceManager for SystemManager {
    fn run(&mut self, _program: &str, _args: &[String]) -> anyhow::Result<CommandResult> {
        todo!("run launchctl or systemctl")
    }
}

pub struct ServiceParams<'a> {
    pub action: ServiceAction,
    pub platform: Platform,
    pub home: &'a Path,
    /// launchd's `gui/{uid}` domain.
    pub uid: u32,
    pub unit: UnitParams<'a>,
    pub manager: &'a mut dyn ServiceManager,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceReport {
    pub installed: bool,
    pub running: bool,
    pub unit_file: PathBuf,
}

/// launchd: install writes the plist, then `bootstrap gui/{uid}` and `kickstart`. systemd: install
/// writes the unit, then `daemon-reload` and `enable --now`. Status reads the unit file and asks
/// the manager whether the server runs.
pub fn run_service(_params: ServiceParams) -> anyhow::Result<ServiceReport> {
    todo!("run a service action")
}

fn service(_ctx: &Arc<Ctx>, _device: &DeviceId, _input: Input) -> Result<Output, OpError> {
    todo!("the service operation")
}
