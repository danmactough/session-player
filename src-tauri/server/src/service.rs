//! `sp service install|uninstall|start|stop|status`: a per-user LaunchAgent (macOS) or systemd user
//! unit (Linux) running `sp serve`. Rendering is pure; launchctl and systemctl run behind
//! `ServiceManager`, so tests use a fake. The `service` operation is never reachable over HTTP.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use session_player_core::ops::{Access, Body, Ctx, DeviceId, Extension, Input, OpError, OpSpec, Output};

/// The LaunchAgent's label; its plist is `~/Library/LaunchAgents/{LAUNCHD_LABEL}.plist`.
pub const LAUNCHD_LABEL: &str = "com.danmactough.sessionplayer.server";

/// The systemd user unit, at `~/.config/systemd/user/{SYSTEMD_UNIT}`.
pub const SYSTEMD_UNIT: &str = "session-player.service";

const SYSTEMD_LABEL: &str = "session-player";

/// The name the service manager knows the service by: the launchd label, or the systemd unit name
/// without `.service`. `sp service --label` replaces the default, so a second server can run beside
/// the first.
pub fn service_label(platform: Platform, requested: Option<&str>) -> String {
    match (requested, platform) {
        (Some(label), _) => label.to_string(),
        (None, Platform::Launchd) => LAUNCHD_LABEL.to_string(),
        (None, Platform::Systemd) => SYSTEMD_LABEL.to_string(),
    }
}

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
    /// See `service_label`.
    #[serde(default)]
    pub label: Option<String>,
    /// The server's data folder, when it isn't the default one.
    #[serde(default)]
    pub data_dir: Option<PathBuf>,
    /// The `sp serve` flags to install the service with.
    #[serde(default)]
    pub serve_args: Vec<String>,
}

/// What the unit file says.
pub struct UnitParams<'a> {
    /// The absolute path of the `sp` binary.
    pub sp: &'a Path,
    /// The logs folder (where `analyze.log` is); the server logs to `server.log` in it.
    pub logs: &'a Path,
    /// PATH for the server, so it finds `claude` (see `service_path`).
    pub path: &'a str,
    /// See `service_label`.
    pub label: Option<&'a str>,
    /// What follows `sp` on the command line the service runs, such as `serve --port 47900`.
    pub args: &'a [&'a str],
}

pub struct ServicePathParams<'a> {
    /// The installing process's PATH.
    pub current: Option<&'a str>,
    pub home: &'a Path,
}

/// The current PATH, then `~/.local/bin` and `~/.claude/local` (where Claude Code installs
/// `claude`) when missing from it, without duplicates.
pub fn service_path(params: ServicePathParams) -> String {
    let current = params.current.unwrap_or("/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin");
    let mut entries: Vec<String> = Vec::new();
    let extra = [params.home.join(".local/bin"), params.home.join(".claude/local")];
    for entry in current.split(':').map(str::to_string).chain(extra.iter().map(|p| p.to_string_lossy().into_owned())) {
        if !entry.is_empty() && !entries.contains(&entry) {
            entries.push(entry);
        }
    }
    entries.join(":")
}

/// A value systemd reads literally: `%` starts a specifier and `$` a variable.
fn systemd_literal(text: &str) -> String {
    text.replace('%', "%%").replace('$', "$$")
}

/// A double-quoted systemd word.
fn systemd_quoted(text: &str) -> String {
    format!("\"{}\"", systemd_literal(&text.replace('\\', "\\\\").replace('"', "\\\"")))
}

/// The systemd user unit's text.
pub fn systemd_unit(params: &UnitParams) -> String {
    let log = systemd_literal(&params.logs.join("server.log").to_string_lossy());
    let mut text = String::from("[Unit]\nDescription=Session Player server\n\n[Service]\n");
    let command: Vec<String> = std::iter::once(params.sp.to_string_lossy().into_owned()).chain(params.args.iter().map(|a| a.to_string())).map(|word| systemd_quoted(&word)).collect();
    text.push_str(&format!("ExecStart={}\n", command.join(" ")));
    text.push_str(&format!("Environment={}\n", systemd_quoted(&format!("PATH={}", params.path))));
    text.push_str(&format!("StandardOutput=append:{log}\nStandardError=append:{log}\n"));
    text.push_str("Restart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n");
    text
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// The LaunchAgent plist's text, with every value XML-escaped.
pub fn launchd_plist(params: &UnitParams) -> String {
    let log = xml(&params.logs.join("server.log").to_string_lossy());
    let environment = format!("\t\t<key>PATH</key>\n\t\t<string>{}</string>\n", xml(params.path));
    let arguments: String = std::iter::once(params.sp.to_string_lossy().into_owned()).chain(params.args.iter().map(|a| a.to_string())).map(|word| format!("\t\t<string>{}</string>\n", xml(&word))).collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
{arguments}	</array>
	<key>EnvironmentVariables</key>
	<dict>
{environment}	</dict>
	<key>StandardOutPath</key>
	<string>{log}</string>
	<key>StandardErrorPath</key>
	<string>{log}</string>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<dict>
		<key>SuccessfulExit</key>
		<false/>
	</dict>
</dict>
</plist>
"#,
        label = xml(&service_label(Platform::Launchd, params.label)),
    )
}

/// Where the unit file lives under `home`.
pub fn unit_file(platform: Platform, home: &Path, label: Option<&str>) -> PathBuf {
    let label = service_label(platform, label);
    match platform {
        Platform::Launchd => home.join("Library/LaunchAgents").join(format!("{label}.plist")),
        Platform::Systemd => home.join(".config/systemd/user").join(format!("{label}.service")),
    }
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
    fn run(&mut self, program: &str, args: &[String]) -> anyhow::Result<CommandResult> {
        let output = std::process::Command::new(program).args(args).output().with_context(|| format!("Couldn't run {program}"))?;
        // Both tools explain a failure on stderr, so it goes with stdout into the error message.
        let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.status.success() {
            stdout.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        Ok(CommandResult { success: output.status.success(), stdout })
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

fn run(manager: &mut dyn ServiceManager, program: &str, args: &[&str]) -> anyhow::Result<CommandResult> {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    manager.run(program, &args)
}

/// Runs the command and fails with its output when it fails.
fn must(manager: &mut dyn ServiceManager, program: &str, args: &[&str]) -> anyhow::Result<()> {
    let result = run(manager, program, args)?;
    if !result.success {
        bail!("`{program} {}` failed: {}", args.join(" "), result.stdout.trim());
    }
    Ok(())
}

/// launchd: install writes the plist, then `bootstrap gui/{uid}` and `kickstart`. systemd: install
/// writes the unit, then `daemon-reload` and `enable --now`. Status reads the unit file and asks
/// the manager whether the server runs.
pub fn run_service(params: ServiceParams) -> anyhow::Result<ServiceReport> {
    let ServiceParams { action, platform, home, uid, unit, manager } = params;
    let file = unit_file(platform, home, unit.label);
    let file_text = file.to_string_lossy().into_owned();
    let domain = format!("gui/{uid}");
    let label = service_label(platform, unit.label);
    let target = format!("{domain}/{label}");
    let unit_name = format!("{label}.service");
    let report = |installed: bool, running: bool| ServiceReport { installed, running, unit_file: file.clone() };
    if matches!(action, ServiceAction::Start) && !file.is_file() {
        bail!("The server isn't installed as a service. Run `sp service install` first.");
    }
    match (action, platform) {
        (ServiceAction::Install, _) => {
            let text = match platform {
                Platform::Launchd => launchd_plist(&unit),
                Platform::Systemd => systemd_unit(&unit),
            };
            std::fs::create_dir_all(unit.logs).with_context(|| format!("Couldn't create {}", unit.logs.display()))?;
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir).with_context(|| format!("Couldn't create {}", dir.display()))?;
            }
            std::fs::write(&file, text).with_context(|| format!("Couldn't write {file_text}"))?;
            match platform {
                Platform::Systemd => {
                    must(manager, "systemctl", &["--user", "daemon-reload"])?;
                    must(manager, "systemctl", &["--user", "enable", "--now", &unit_name])?;
                }
                Platform::Launchd => {
                    // A reinstall (after an update) replaces an agent that is already loaded; when
                    // none is, bootout fails and that is fine.
                    run(manager, "launchctl", &["bootout", &target])?;
                    must(manager, "launchctl", &["bootstrap", &domain, &file_text])?;
                    must(manager, "launchctl", &["kickstart", "-k", &target])?;
                }
            }
            Ok(report(true, true))
        }
        (ServiceAction::Uninstall, Platform::Systemd) => {
            // Disabling a unit that isn't loaded fails; the file still goes.
            run(manager, "systemctl", &["--user", "disable", "--now", &unit_name])?;
            remove(&file)?;
            must(manager, "systemctl", &["--user", "daemon-reload"])?;
            Ok(report(false, false))
        }
        (ServiceAction::Uninstall, Platform::Launchd) => {
            run(manager, "launchctl", &["bootout", &target])?;
            remove(&file)?;
            Ok(report(false, false))
        }
        (ServiceAction::Start, Platform::Systemd) => {
            must(manager, "systemctl", &["--user", "start", &unit_name])?;
            Ok(report(true, true))
        }
        (ServiceAction::Start, Platform::Launchd) => {
            // Bootstrap fails when the agent is already loaded, and kickstart then starts it.
            run(manager, "launchctl", &["bootstrap", &domain, &file_text])?;
            must(manager, "launchctl", &["kickstart", &target])?;
            Ok(report(true, true))
        }
        (ServiceAction::Stop, Platform::Systemd) => {
            must(manager, "systemctl", &["--user", "stop", &unit_name])?;
            Ok(report(file.is_file(), false))
        }
        (ServiceAction::Stop, Platform::Launchd) => {
            // The plist stays, so the agent loads again at the next login. Bootout fails when the
            // agent isn't loaded, which is already stopped.
            run(manager, "launchctl", &["bootout", &target])?;
            Ok(report(file.is_file(), false))
        }
        (ServiceAction::Status, Platform::Systemd) => {
            let result = run(manager, "systemctl", &["--user", "is-active", &unit_name])?;
            Ok(report(file.is_file(), result.success && result.stdout.trim() == "active"))
        }
        (ServiceAction::Status, Platform::Launchd) => {
            let result = run(manager, "launchctl", &["print", &target])?;
            Ok(report(file.is_file(), result.success && result.stdout.contains("state = running")))
        }
    }
}

fn remove(file: &Path) -> anyhow::Result<()> {
    match std::fs::remove_file(file) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e).with_context(|| format!("Couldn't remove {}", file.display())),
        _ => Ok(()),
    }
}

/// The real user's service, for the `sp` binary running this operation.
fn service(ctx: &Arc<Ctx>, _device: &DeviceId, input: Input) -> Result<Output, OpError> {
    let Input::Json(value) = input else { return Err(OpError::BadRequest("service takes a JSON body".to_string())) };
    let req: ServiceRequest = serde_json::from_value(value).map_err(|e| OpError::BadRequest(e.to_string()))?;
    let platform = if cfg!(target_os = "macos") {
        Platform::Launchd
    } else if cfg!(target_os = "linux") {
        Platform::Systemd
    } else {
        return Err(OpError::BadRequest("sp service works on macOS and Linux only".to_string()));
    };
    let internal = |e: anyhow::Error| OpError::Internal(format!("{e:#}"));
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| OpError::Internal("HOME isn't set".to_string()))?;
    let sp = std::env::current_exe().context("Couldn't find the sp binary").map_err(internal)?;
    let uid = if platform == Platform::Launchd { current_uid().map_err(internal)? } else { 0 };
    let current = std::env::var("PATH").ok();
    let path = service_path(ServicePathParams { current: current.as_deref(), home: &home });
    let data_dir = req.data_dir.as_ref().map(|d| d.to_string_lossy().into_owned());
    let mut args: Vec<&str> = Vec::new();
    if let Some(data) = &data_dir {
        args.extend(["--data-dir", data]);
    }
    args.push("serve");
    args.extend(req.serve_args.iter().map(String::as_str));
    let report = run_service(ServiceParams {
        action: req.action,
        platform,
        home: &home,
        uid,
        unit: UnitParams { sp: &sp, logs: &ctx.paths().logs, path: &path, label: req.label.as_deref(), args: &args },
        manager: &mut SystemManager,
    })
    .map_err(internal)?;
    serde_json::to_value(report).map(Output::Json).map_err(|e| OpError::Internal(e.to_string()))
}

fn current_uid() -> anyhow::Result<u32> {
    let output = std::process::Command::new("id").arg("-u").output().context("Couldn't run id -u")?;
    String::from_utf8_lossy(&output.stdout).trim().parse().context("id -u didn't print a number")
}
