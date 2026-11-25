// `sp service` on Linux: a systemd user unit that runs `sp serve` with a PATH that finds `claude`,
// logs to server.log beside analyze.log, and is enabled on install. launchctl and systemctl run
// behind `ServiceManager`, so these tests use a fake that records each command. The `service`
// operation is never reachable over HTTP.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use std::path::Path;

use common::{CtxParams, bearer, loopback_config, open_ctx, post_json, start};
use session_player_core::ops::{Access, spec_of};
use session_player_server::service::{
    CommandResult, Platform, SYSTEMD_UNIT, ServiceAction, ServiceManager, ServiceParams, ServicePathParams, UnitParams, run_service, service_path,
    systemd_unit, unit_file,
};
use tempdir::TempDir;

/// Records every command and answers each with success and no output.
#[derive(Default)]
struct FakeManager {
    commands: Vec<(String, Vec<String>)>,
}

impl ServiceManager for FakeManager {
    fn run(&mut self, program: &str, args: &[String]) -> anyhow::Result<CommandResult> {
        self.commands.push((program.to_string(), args.to_vec()));
        Ok(CommandResult { success: true, stdout: String::new() })
    }
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|a| a.to_string()).collect()
}

fn unit<'a>(sp: &'a Path, logs: &'a Path, args: &'a [&'a str]) -> UnitParams<'a> {
    UnitParams { sp, logs, path: "/usr/bin:/home/someone/.local/bin", label: None, args }
}

/// The values of every `key=` line.
fn values<'a>(text: &'a str, key: &str) -> Vec<&'a str> {
    text.lines().filter_map(|l| l.trim().strip_prefix(key)).collect()
}

#[test]
fn the_systemd_unit_runs_sp_serve_with_path_set_and_logs_to_server_log() {
    let sp = Path::new("/opt/session player/bin/sp");
    let logs = Path::new("/home/someone/.local/share/sp/logs");

    let text = systemd_unit(&unit(sp, logs, &["--data-dir", "/home/someone/sp-data", "serve", "--bind", "0.0.0.0"]));

    let exec = values(&text, "ExecStart=");
    assert_eq!(exec.len(), 1, "{text}");
    assert!(exec[0].contains("/opt/session player/bin/sp"), "{text}");
    assert!(exec[0].trim_end().ends_with(r#""--data-dir" "/home/someone/sp-data" "serve" "--bind" "0.0.0.0""#), "{text}");
    let environment = values(&text, "Environment=").join("\n");
    assert!(environment.contains("PATH=/usr/bin:/home/someone/.local/bin"), "{text}");
    let log = "/home/someone/.local/share/sp/logs/server.log";
    assert_eq!(values(&text, "StandardOutput="), vec![format!("append:{log}").as_str()], "{text}");
    assert_eq!(values(&text, "StandardError="), vec![format!("append:{log}").as_str()], "{text}");
    assert_eq!(values(&text, "WantedBy="), vec!["default.target"], "{text}");
    assert!(!values(&text, "Restart=").is_empty(), "the server restarts after a crash: {text}");

    let plain = systemd_unit(&unit(sp, logs, &["serve"]));
    assert!(values(&plain, "ExecStart=")[0].trim_end().ends_with(r#""/opt/session player/bin/sp" "serve""#), "{plain}");
}

#[test]
fn the_service_path_keeps_the_current_path_and_adds_where_claude_installs() {
    let home = Path::new("/home/someone");

    let path = service_path(ServicePathParams { current: Some("/usr/local/bin:/usr/bin:/home/someone/.local/bin"), home });
    let entries: Vec<&str> = path.split(':').collect();

    assert_eq!(&entries[..3], ["/usr/local/bin", "/usr/bin", "/home/someone/.local/bin"]);
    assert!(entries.contains(&"/home/someone/.claude/local"), "{path}");
    assert_eq!(entries.iter().filter(|e| **e == "/home/someone/.local/bin").count(), 1, "{path}");

    let bare = service_path(ServicePathParams { current: None, home });
    let bare_entries: Vec<&str> = bare.split(':').collect();
    for wanted in ["/home/someone/.local/bin", "/home/someone/.claude/local", "/usr/bin", "/bin"] {
        assert!(bare_entries.contains(&wanted), "{wanted} missing from {bare}");
    }
}

#[test]
fn installing_on_systemd_writes_the_user_unit_and_enables_it() {
    let dir = TempDir::new("server-service-systemd");
    let home = dir.join("home");
    let logs = dir.join("logs");
    let sp = dir.join("bin/sp");
    let mut manager = FakeManager::default();

    let report = run_service(ServiceParams {
        action: ServiceAction::Install,
        platform: Platform::Systemd,
        home: &home,
        uid: 1000,
        unit: unit(&sp, &logs, &["serve"]),
        manager: &mut manager,
    })
    .expect("install");

    let file = home.join(".config/systemd/user").join(SYSTEMD_UNIT);
    assert_eq!(unit_file(Platform::Systemd, &home, None), file);
    assert_eq!(report.unit_file, file);
    assert!(report.installed);
    let written = std::fs::read_to_string(&file).expect("the unit file");
    assert_eq!(written, systemd_unit(&unit(&sp, &logs, &["serve"])));
    assert_eq!(
        manager.commands,
        vec![
            ("systemctl".to_string(), args(&["--user", "daemon-reload"])),
            ("systemctl".to_string(), args(&["--user", "enable", "--now", SYSTEMD_UNIT])),
        ]
    );
}

#[test]
fn the_service_operation_is_registered_but_never_reachable_over_http() {
    let dir = TempDir::new("server-service-http");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());

    let spec = spec_of(&ctx, "service");
    let answer = post_json(server.addrs().loopback, "/api/v1/service", &[("Authorization", &bearer(&ctx.paths().data))], r#"{"action":"status"}"#);

    assert_eq!(spec.map(|s| s.access), Some(Access::NotOverHttp));
    assert_eq!(answer.status, 404, "{}", String::from_utf8_lossy(&answer.body));
    assert_eq!(answer.code(), "unknown_operation");
    server.shutdown();
}
