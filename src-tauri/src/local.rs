//! The server on this machine: whether it runs, and installing or starting it as a per-user
//! service. The service is managed by running the `sp` bundled in the app (`sp service ...`), so
//! the unit it writes starts that same `sp`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::Serialize;
use session_player_client::{LocalProbe, probe_local};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LocalServerState {
    /// No server runs and none is installed as a service.
    NotInstalled,
    /// Installed as a service, not running.
    Installed,
    Running { port: u16 },
}

/// What `sp service status` printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceFacts {
    pub installed: bool,
    pub running: bool,
    pub unit_file: PathBuf,
}

/// Where the page reaches the local server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Connection {
    /// `http://127.0.0.1:{port}`
    pub url: String,
    pub token: String,
}

/// Reads `sp service`'s two lines: `Installed: <unit file>` or `Not installed: <unit file>`, then
/// `Running` or `Not running`.
pub fn parse_service_status(text: &str) -> Option<ServiceFacts> {
    let mut lines = text.lines();
    let first = lines.next()?;
    let (installed, unit_file) = if let Some(file) = first.strip_prefix("Installed: ") {
        (true, file)
    } else {
        (false, first.strip_prefix("Not installed: ")?)
    };
    let running = match lines.next()?.trim() {
        "Running" => true,
        "Not running" => false,
        _ => return None,
    };
    Some(ServiceFacts { installed, running, unit_file: PathBuf::from(unit_file) })
}

/// A live lock is Running whatever the service says (it may be a `sp serve` in a terminal).
pub fn local_state(probe: &LocalProbe, service: Option<&ServiceFacts>) -> LocalServerState {
    match (probe, service) {
        (LocalProbe::Live { port, .. }, _) => LocalServerState::Running { port: *port },
        (LocalProbe::NotLive, Some(facts)) if facts.installed => LocalServerState::Installed,
        (LocalProbe::NotLive, _) => LocalServerState::NotInstalled,
    }
}

pub fn local_connection(probe: &LocalProbe) -> Result<Connection, String> {
    match probe {
        LocalProbe::Live { port, token: Some(token) } => Ok(Connection { url: format!("http://127.0.0.1:{port}"), token: token.clone() }),
        LocalProbe::Live { .. } => Err("The server on this machine is running, but its token file can't be read.".to_string()),
        LocalProbe::NotLive => Err("No server is running on this machine.".to_string()),
    }
}

/// Runs `{sp} service {action}` and reads what it printed.
pub fn run_service(sp: &Path, action: &str) -> Result<ServiceFacts, String> {
    let output = Command::new(sp).args(["service", action]).output().map_err(|e| format!("Couldn't run {}: {e}", sp.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("sp service {action} failed: {}", stderr.trim()));
    }
    parse_service_status(&stdout).ok_or_else(|| format!("sp service {action} printed something unexpected: {}", stdout.trim()))
}

pub struct StateParams<'a> {
    pub data: &'a Path,
    /// The bundled `sp`; None in a build without one, where only a running server is seen.
    pub sp: Option<&'a Path>,
}

pub fn server_state(params: StateParams) -> LocalServerState {
    let probe = probe_local(params.data);
    if matches!(probe, LocalProbe::Live { .. }) {
        return local_state(&probe, None);
    }
    let service = params.sp.and_then(|sp| run_service(sp, "status").ok());
    local_state(&probe, service.as_ref())
}

pub struct StartParams<'a> {
    pub data: &'a Path,
    pub sp: &'a Path,
    /// How long to wait for the server's lock after the service manager says it started.
    pub wait: Duration,
}

/// `sp service install`, then waits until the server is live.
pub fn install(params: StartParams) -> Result<LocalServerState, String> {
    run_service(params.sp, "install")?;
    wait_live(&params)
}

/// Starts the installed, stopped service, then waits until the server is live.
pub fn start(params: StartParams) -> Result<LocalServerState, String> {
    run_service(params.sp, "start")?;
    wait_live(&params)
}

fn wait_live(params: &StartParams) -> Result<LocalServerState, String> {
    let deadline = Instant::now() + params.wait;
    loop {
        // probe_local itself waits up to 500 ms for a server that is binding.
        if let LocalProbe::Live { port, .. } = probe_local(params.data) {
            return Ok(LocalServerState::Running { port });
        }
        if Instant::now() >= deadline {
            return Err(format!("The server didn't start within {} s. Its log is server.log beside analyze.log.", params.wait.as_secs_f32()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// After an update, the service may name an `sp` the app no longer has. When the service is
/// installed and its unit file doesn't name `sp`, reinstalls it. Returns whether it did.
pub fn refresh_after_update(sp: &Path) -> Result<bool, String> {
    let facts = run_service(sp, "status")?;
    if !facts.installed {
        return Ok(false);
    }
    // The unit file escapes the path (XML in a plist, quotes in a systemd unit), but an app path
    // rarely holds a character that changes; when one does, this reinstalls at every launch, which
    // is harmless.
    let unit = std::fs::read_to_string(&facts.unit_file).unwrap_or_default();
    if unit.contains(&*sp.to_string_lossy()) {
        return Ok(false);
    }
    run_service(sp, "install")?;
    Ok(true)
}
