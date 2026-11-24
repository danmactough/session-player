// `sp service` on macOS: a per-user LaunchAgent whose plist escapes every value, runs `sp serve`
// at login with a PATH that finds `claude`, logs to server.log beside analyze.log, and is
// bootstrapped into the user's gui domain and then kickstarted on install.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;

use std::path::Path;

use session_player_server::service::{
    CommandResult, LAUNCHD_LABEL, Platform, ServiceAction, ServiceManager, ServiceParams, UnitParams, launchd_plist, run_service, unit_file,
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

/// The text between `<string>` tags that follow `<key>{key}</key>`.
fn string_after_key<'a>(plist: &'a str, key: &str) -> &'a str {
    let at = plist.find(&format!("<key>{key}</key>")).unwrap_or_else(|| panic!("no {key} in {plist}"));
    let rest = &plist[at..];
    let start = rest.find("<string>").expect("a string value") + "<string>".len();
    let end = rest.find("</string>").expect("a closed string");
    &rest[start..end]
}

#[test]
fn the_launch_agent_plist_escapes_every_value_and_runs_sp_serve_with_path_set() {
    let sp = Path::new("/Users/a&b/<odd> \"apps\"/sp");
    let logs = Path::new("/Users/a&b/Library/Logs/sp");
    let data = Path::new("/Users/a&b/sp data");

    let data = data.to_string_lossy();
    let plist = launchd_plist(&UnitParams { sp, logs, path: "/usr/bin:/Users/a&b/.local/bin", label: None, args: &["--data-dir", &data, "serve", "--port", "47900"] });

    assert!(plist.starts_with("<?xml"), "{plist}");
    assert!(plist.contains("<!DOCTYPE plist"), "{plist}");
    assert_eq!(string_after_key(&plist, "Label"), LAUNCHD_LABEL);
    let program_arguments = &plist[plist.find("<key>ProgramArguments</key>").expect("ProgramArguments")..];
    let array = &program_arguments[..program_arguments.find("</array>").expect("an array")];
    assert!(array.contains("<string>/Users/a&amp;b/&lt;odd&gt; &quot;apps&quot;/sp</string>"), "{plist}");
    assert!(array.contains("<string>--data-dir</string>\n\t\t<string>/Users/a&amp;b/sp data</string>\n\t\t<string>serve</string>\n\t\t<string>--port</string>\n\t\t<string>47900</string>"), "{plist}");
    assert_eq!(string_after_key(&plist, "PATH"), "/usr/bin:/Users/a&amp;b/.local/bin");
    assert_eq!(string_after_key(&plist, "StandardOutPath"), "/Users/a&amp;b/Library/Logs/sp/server.log");
    assert_eq!(string_after_key(&plist, "StandardErrorPath"), "/Users/a&amp;b/Library/Logs/sp/server.log");
    assert!(plist.contains("<key>RunAtLoad</key>") && plist.contains("<true/>"), "{plist}");
    assert!(plist.contains("<key>KeepAlive</key>"), "{plist}");
    let unescaped = plist.replace("&amp;", "").replace("&lt;", "").replace("&gt;", "").replace("&quot;", "");
    assert!(!unescaped.contains('&'), "an unescaped & in {plist}");

    let named = launchd_plist(&UnitParams { sp, logs, path: "/usr/bin", label: Some("dev.sp.server"), args: &["serve"] });
    assert_eq!(string_after_key(&named, "Label"), "dev.sp.server");
}

#[test]
fn installing_on_launchd_writes_an_escaped_launch_agent_and_bootstraps_then_kickstarts_it() {
    let dir = TempDir::new("server-service-launchd");
    let home = dir.join("home");
    let logs = dir.join("logs & more");
    let sp = dir.join("bin/sp");
    let unit = || UnitParams { sp: &sp, logs: &logs, path: "/usr/bin", label: None, args: &["serve"] };
    let mut manager = FakeManager::default();

    let report = run_service(ServiceParams { action: ServiceAction::Install, platform: Platform::Launchd, home: &home, uid: 501, unit: unit(), manager: &mut manager })
        .expect("install");

    let file = home.join("Library/LaunchAgents").join(format!("{LAUNCHD_LABEL}.plist"));
    assert_eq!(unit_file(Platform::Launchd, &home, None), file);
    assert_eq!(report.unit_file, file);
    assert!(report.installed);
    assert_eq!(std::fs::read_to_string(&file).expect("the plist"), launchd_plist(&unit()));
    assert!(manager.commands.iter().all(|(program, _)| program == "launchctl"), "{:?}", manager.commands);
    let tail: Vec<Vec<String>> = manager.commands.iter().rev().take(2).rev().map(|(_, a)| a.clone()).collect();
    let target = format!("gui/501/{LAUNCHD_LABEL}");
    assert_eq!(
        tail,
        vec![
            vec!["bootstrap".to_string(), "gui/501".to_string(), file.to_string_lossy().into_owned()],
            vec!["kickstart".to_string(), "-k".to_string(), target],
        ],
        "{:?}",
        manager.commands
    );
}

#[test]
fn a_labelled_launch_agent_has_its_own_plist_and_target() {
    let dir = TempDir::new("server-service-launchd-label");
    let home = dir.join("home");
    let logs = dir.join("logs");
    let sp = dir.join("bin/sp");
    let mut manager = FakeManager::default();
    let unit = UnitParams { sp: &sp, logs: &logs, path: "/usr/bin", label: Some("dev.sp.server"), args: &["serve", "--port", "47900"] };

    let report = run_service(ServiceParams { action: ServiceAction::Install, platform: Platform::Launchd, home: &home, uid: 501, unit, manager: &mut manager }).expect("install");

    let file = home.join("Library/LaunchAgents/dev.sp.server.plist");
    assert_eq!(report.unit_file, file);
    assert!(std::fs::read_to_string(&file).expect("the plist").contains("<string>dev.sp.server</string>"));
    assert!(manager.commands.iter().any(|(_, a)| a.last().is_some_and(|t| t == "gui/501/dev.sp.server")), "{:?}", manager.commands);
    assert!(!home.join(format!("Library/LaunchAgents/{LAUNCHD_LABEL}.plist")).exists());
}
