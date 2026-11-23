// `sp service uninstall|start|stop|status` on both platforms, through a fake ServiceManager.
// systemd: disable --now and remove the unit; start; stop; is-active. launchd: bootout and remove
// the plist; bootstrap then kickstart; bootout (the plist stays, so it loads at the next login);
// print. Status reads "installed" from the unit file and "running" from the manager.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;

use std::path::{Path, PathBuf};

use session_player_server::service::{
    CommandResult, LAUNCHD_LABEL, Platform, SYSTEMD_UNIT, ServiceAction, ServiceManager, ServiceParams, ServiceReport, UnitParams, run_service, unit_file,
};
use tempdir::TempDir;

const UID: u32 = 501;

/// Records every command; `answer` decides what each returns.
struct FakeManager {
    commands: Vec<(String, Vec<String>)>,
    answer: fn(&[String]) -> CommandResult,
}

impl FakeManager {
    fn new(answer: fn(&[String]) -> CommandResult) -> FakeManager {
        FakeManager { commands: Vec::new(), answer }
    }

    fn args(&self) -> Vec<Vec<String>> {
        self.commands.iter().map(|(_, a)| a.clone()).collect()
    }
}

impl ServiceManager for FakeManager {
    fn run(&mut self, program: &str, args: &[String]) -> anyhow::Result<CommandResult> {
        self.commands.push((program.to_string(), args.to_vec()));
        Ok((self.answer)(args))
    }
}

fn ok(_: &[String]) -> CommandResult {
    CommandResult { success: true, stdout: String::new() }
}

fn running(args: &[String]) -> CommandResult {
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["--user", "is-active", ..] => CommandResult { success: true, stdout: "active\n".to_string() },
        ["print", ..] => CommandResult { success: true, stdout: format!("{LAUNCHD_LABEL} = {{\n\tstate = running\n\tpid = 4242\n}}\n") },
        _ => ok(args),
    }
}

fn stopped(args: &[String]) -> CommandResult {
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["--user", "is-active", ..] => CommandResult { success: false, stdout: "inactive\n".to_string() },
        ["print", ..] => CommandResult { success: false, stdout: String::new() },
        _ => ok(args),
    }
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|a| a.to_string()).collect()
}

struct Home {
    home: PathBuf,
    logs: PathBuf,
    sp: PathBuf,
}

impl Home {
    fn new(dir: &Path) -> Home {
        Home { home: dir.join("home"), logs: dir.join("logs"), sp: dir.join("bin/sp") }
    }

    fn run(&self, platform: Platform, action: ServiceAction, manager: &mut FakeManager) -> anyhow::Result<ServiceReport> {
        run_service(ServiceParams {
            action,
            platform,
            home: &self.home,
            uid: UID,
            unit: UnitParams { sp: &self.sp, logs: &self.logs, path: "/usr/bin", data_dir: None },
            manager,
        })
    }

    /// Installs with a manager that succeeds at everything.
    fn installed(&self, platform: Platform) -> PathBuf {
        self.run(platform, ServiceAction::Install, &mut FakeManager::new(ok)).expect("install");
        let file = unit_file(platform, &self.home);
        assert!(file.is_file(), "{}", file.display());
        file
    }
}

#[test]
fn uninstalling_on_systemd_disables_the_unit_and_removes_it() {
    let dir = TempDir::new("server-service-uninstall-systemd");
    let home = Home::new(&dir);
    let file = home.installed(Platform::Systemd);
    let mut manager = FakeManager::new(ok);

    let report = home.run(Platform::Systemd, ServiceAction::Uninstall, &mut manager).expect("uninstall");

    assert!(!file.exists());
    assert!(!report.installed && !report.running, "{report:?}");
    assert_eq!(manager.args(), vec![strings(&["--user", "disable", "--now", SYSTEMD_UNIT]), strings(&["--user", "daemon-reload"])]);
}

#[test]
fn uninstalling_on_launchd_boots_the_agent_out_and_removes_its_plist() {
    let dir = TempDir::new("server-service-uninstall-launchd");
    let home = Home::new(&dir);
    let file = home.installed(Platform::Launchd);
    let mut manager = FakeManager::new(ok);

    let report = home.run(Platform::Launchd, ServiceAction::Uninstall, &mut manager).expect("uninstall");

    assert!(!file.exists());
    assert!(!report.installed && !report.running, "{report:?}");
    assert_eq!(manager.args(), vec![strings(&["bootout", &format!("gui/{UID}/{LAUNCHD_LABEL}")])]);
}

#[test]
fn start_and_stop_on_systemd_ask_systemctl() {
    let dir = TempDir::new("server-service-startstop-systemd");
    let home = Home::new(&dir);
    home.installed(Platform::Systemd);

    let mut start = FakeManager::new(ok);
    home.run(Platform::Systemd, ServiceAction::Start, &mut start).expect("start");
    let mut stop = FakeManager::new(ok);
    home.run(Platform::Systemd, ServiceAction::Stop, &mut stop).expect("stop");

    assert_eq!(start.args(), vec![strings(&["--user", "start", SYSTEMD_UNIT])]);
    assert_eq!(stop.args(), vec![strings(&["--user", "stop", SYSTEMD_UNIT])]);
}

#[test]
fn start_on_launchd_bootstraps_then_kickstarts_and_stop_boots_out_but_keeps_the_plist() {
    let dir = TempDir::new("server-service-startstop-launchd");
    let home = Home::new(&dir);
    let file = home.installed(Platform::Launchd);
    let target = format!("gui/{UID}/{LAUNCHD_LABEL}");

    let mut start = FakeManager::new(ok);
    home.run(Platform::Launchd, ServiceAction::Start, &mut start).expect("start");
    let mut stop = FakeManager::new(ok);
    home.run(Platform::Launchd, ServiceAction::Stop, &mut stop).expect("stop");

    assert_eq!(start.args(), vec![strings(&["bootstrap", &format!("gui/{UID}"), &file.to_string_lossy()]), strings(&["kickstart", &target])]);
    assert_eq!(stop.args(), vec![strings(&["bootout", &target])]);
    assert!(file.is_file(), "stop keeps the plist so the agent loads at the next login");
}

#[test]
fn starting_a_service_that_is_not_installed_fails_without_running_anything() {
    for platform in [Platform::Systemd, Platform::Launchd] {
        let dir = TempDir::new("server-service-start-missing");
        let home = Home::new(&dir);
        let mut manager = FakeManager::new(ok);

        let result = home.run(platform, ServiceAction::Start, &mut manager);

        let message = format!("{:#}", result.expect_err("start without install"));
        assert!(message.contains("sp service install"), "{platform:?}: {message}");
        assert!(manager.commands.is_empty(), "{platform:?}: {:?}", manager.commands);
    }
}

#[test]
fn status_reports_installed_from_the_unit_file_and_running_from_the_manager_on_both_platforms() {
    for platform in [Platform::Systemd, Platform::Launchd] {
        let dir = TempDir::new("server-service-status");
        let home = Home::new(&dir);

        let never = home.run(platform, ServiceAction::Status, &mut FakeManager::new(stopped)).expect("status");
        assert!(!never.installed && !never.running, "{platform:?}: {never:?}");

        home.installed(platform);
        let mut manager = FakeManager::new(running);
        let up = home.run(platform, ServiceAction::Status, &mut manager).expect("status");
        let down = home.run(platform, ServiceAction::Status, &mut FakeManager::new(stopped)).expect("status");

        assert!(up.installed && up.running, "{platform:?}: {up:?}");
        assert!(down.installed && !down.running, "{platform:?}: {down:?}");
        assert_eq!(up.unit_file, unit_file(platform, &home.home));
        let expected = match platform {
            Platform::Systemd => strings(&["--user", "is-active", SYSTEMD_UNIT]),
            Platform::Launchd => strings(&["print", &format!("gui/{UID}/{LAUNCHD_LABEL}")]),
        };
        assert_eq!(manager.args(), vec![expected], "{platform:?}");
    }
}
