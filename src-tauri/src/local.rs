//! The server on this machine: whether it runs, running it as the app's sidecar when none does,
//! and installing or starting it as a per-user service. Both run the `sp` bundled in the app, so
//! the service's unit starts that same `sp`.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};
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
    /// `sidecar`: this app started the server, which stops when the app quits.
    Running { port: u16, sidecar: bool },
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
        (LocalProbe::Live { port, .. }, _) => LocalServerState::Running { port: *port, sidecar: false },
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

/// Runs `{sp} service {action}` and reads what it printed. `data_override` is the `--data-dir` the
/// app was started with, if any, so the service belongs to that data folder.
pub fn run_service(sp: &Path, action: &str, data_override: Option<&Path>) -> Result<ServiceFacts, String> {
    let mut command = Command::new(sp);
    command.args(["service", action]);
    if let Some(dir) = data_override {
        command.arg("--data-dir").arg(dir);
    }
    let output = command.output().map_err(|e| format!("Couldn't run {}: {e}", sp.display()))?;
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
    pub data_override: Option<&'a Path>,
}

pub fn server_state(params: StateParams) -> LocalServerState {
    let probe = probe_local(params.data);
    if matches!(probe, LocalProbe::Live { .. }) {
        return local_state(&probe, None);
    }
    let service = params.sp.and_then(|sp| run_service(sp, "status", params.data_override).ok());
    local_state(&probe, service.as_ref())
}

#[derive(Clone, Copy)]
pub struct StartParams<'a> {
    pub data: &'a Path,
    pub sp: &'a Path,
    pub data_override: Option<&'a Path>,
    /// How long to wait for the server's lock after the service manager says it started.
    pub wait: Duration,
}

/// `sp service install`, then waits until the server is live.
pub fn install(params: StartParams) -> Result<LocalServerState, String> {
    run_service(params.sp, "install", params.data_override)?;
    wait_live(&params)
}

/// Starts the installed, stopped service, then waits until the server is live.
pub fn start(params: StartParams) -> Result<LocalServerState, String> {
    run_service(params.sp, "start", params.data_override)?;
    wait_live(&params)
}

fn wait_live(params: &StartParams) -> Result<LocalServerState, String> {
    let deadline = Instant::now() + params.wait;
    loop {
        // probe_local itself waits up to 500 ms for a server that is binding.
        if let LocalProbe::Live { port, .. } = probe_local(params.data) {
            return Ok(LocalServerState::Running { port, sidecar: false });
        }
        if Instant::now() >= deadline {
            return Err(format!("The server didn't start within {} s. Its log is server.log beside analyze.log.", params.wait.as_secs_f32()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// After an update, the service may name an `sp` the app no longer has. When the service is
/// installed and its unit file doesn't name `sp`, reinstalls it. Returns whether it did.
pub fn refresh_after_update(sp: &Path, data_override: Option<&Path>) -> Result<bool, String> {
    let facts = run_service(sp, "status", data_override)?;
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
    run_service(sp, "install", data_override)?;
    Ok(true)
}

/// How long a sidecar gets to exit after its standard input closes, before it is killed.
const SIDECAR_EXIT_WAIT: Duration = Duration::from_secs(8);

pub struct SidecarParams<'a> {
    pub start: StartParams<'a>,
    /// Where the sidecar's output goes.
    pub log: &'a Path,
}

/// The app's hold on this machine's server. One lock covers starting a sidecar and changing the
/// service, so a reconnecting page can't start a sidecar that takes `server.lock` from a service
/// that is on its way up.
#[derive(Default)]
pub struct LocalServer {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    sidecar: Option<Child>,
    refreshed: bool,
    /// Set when the app is quitting, after which no sidecar starts.
    closed: bool,
}

impl Inner {
    fn sidecar_runs(&mut self) -> bool {
        self.sidecar.as_mut().is_some_and(|child| matches!(child.try_wait(), Ok(None)))
    }

    /// `refresh_after_update`, once per launch, before anything else looks for a server: a
    /// reinstall restarts the service, and a sidecar must not start in that gap.
    fn refresh_once(&mut self, params: &StartParams) {
        if std::mem::replace(&mut self.refreshed, true) {
            return;
        }
        match refresh_after_update(params.sp, params.data_override) {
            Ok(false) => {}
            Ok(true) => {
                if let Err(e) = wait_live(params) {
                    eprintln!("{e}");
                }
            }
            Err(e) => eprintln!("Couldn't refresh the server's service: {e}"),
        }
    }

    /// Closing the sidecar's standard input asks it to exit.
    fn stop_sidecar(&mut self) {
        let Some(mut child) = self.sidecar.take() else { return };
        drop(child.stdin.take());
        let deadline = Instant::now() + SIDECAR_EXIT_WAIT;
        while matches!(child.try_wait(), Ok(None)) {
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn ensure(&mut self, params: &SidecarParams) -> Result<LocalServerState, String> {
        let start = &params.start;
        if let LocalProbe::Live { port, .. } = probe_local(start.data) {
            return Ok(LocalServerState::Running { port, sidecar: self.sidecar_runs() });
        }
        if self.closed {
            return Err("Session Player is quitting.".to_string());
        }
        self.stop_sidecar();
        self.sidecar = Some(spawn_sidecar(params)?);
        let deadline = Instant::now() + start.wait;
        loop {
            if let LocalProbe::Live { port, .. } = probe_local(start.data) {
                return Ok(LocalServerState::Running { port, sidecar: self.sidecar_runs() });
            }
            if !self.sidecar_runs() {
                self.stop_sidecar();
                return Err(format!("The server stopped while starting. Its log is {}.", params.log.display()));
            }
            if Instant::now() >= deadline {
                self.stop_sidecar();
                return Err(format!("The server didn't start within {} s. Its log is {}.", start.wait.as_secs_f32(), params.log.display()));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn spawn_sidecar(params: &SidecarParams) -> Result<Child, String> {
    let log_error = |e: std::io::Error| format!("Couldn't open {}: {e}", params.log.display());
    if let Some(dir) = params.log.parent() {
        std::fs::create_dir_all(dir).map_err(log_error)?;
    }
    let log = std::fs::File::options().create(true).append(true).open(params.log).map_err(log_error)?;
    let mut command = Command::new(params.start.sp);
    if let Some(dir) = params.start.data_override {
        command.arg("--data-dir").arg(dir);
    }
    command
        .args(["serve", "--exit-on-stdin-eof"])
        // The app holds the write end until it exits. A crash or a force quit closes it too.
        .stdin(Stdio::piped())
        .stdout(log.try_clone().map_err(log_error)?)
        .stderr(log)
        .spawn()
        .map_err(|e| format!("Couldn't run {}: {e}", params.start.sp.display()))
}

impl LocalServer {
    /// Never rewrites the installed service. A dev build uses this: its `sp` differs from the
    /// one the service names, and must not take the service over.
    pub fn leaving_service_alone() -> LocalServer {
        LocalServer { inner: Mutex::new(Inner { refreshed: true, ..Inner::default() }) }
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Uses the server running on this machine; when none runs, starts one as a sidecar.
    pub fn ensure(&self, params: SidecarParams) -> Result<LocalServerState, String> {
        let mut inner = self.inner();
        inner.refresh_once(&params.start);
        inner.ensure(&params)
    }

    pub fn state(&self, params: StateParams) -> LocalServerState {
        // Locked first, so the answer doesn't describe a server that is being replaced.
        let mut inner = self.inner();
        match server_state(params) {
            LocalServerState::Running { port, .. } => LocalServerState::Running { port, sidecar: inner.sidecar_runs() },
            other => other,
        }
    }

    /// Replaces the sidecar with the per-user service, which keeps running after the app quits.
    /// An installed service is only started, which keeps the `sp serve` flags it was installed
    /// with. When that fails, the sidecar starts again.
    pub fn keep_running(&self, params: SidecarParams) -> Result<LocalServerState, String> {
        let mut inner = self.inner();
        inner.refresh_once(&params.start);
        let facts = run_service(params.start.sp, "status", params.start.data_override)?;
        inner.stop_sidecar();
        let result = if facts.installed { start(params.start) } else { install(params.start) };
        if result.is_err()
            && let Err(e) = inner.ensure(&params)
        {
            eprintln!("{e}");
        }
        result
    }

    /// Stops the sidecar as the app quits. While another call holds the lock this does nothing:
    /// the app's exit closes the sidecar's standard input anyway.
    pub fn shutdown(&self) {
        let mut inner = match self.inner.try_lock() {
            Ok(inner) => inner,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return,
        };
        inner.closed = true;
        inner.stop_sidecar();
    }
}
