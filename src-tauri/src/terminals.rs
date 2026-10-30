//! Terminal apps that can run a session's resume command, and the AppleScript
//! that opens one of them on it.
//!
//! The command is built here from the session's fields rather than taken from
//! the webview, so the frontend can't use this to run arbitrary commands.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct TerminalApp {
    pub id: &'static str,
    pub label: &'static str,
}

/// (id, label, app bundle, where it is installed when it ships with macOS)
const TERMINALS: [(&str, &str, &str, Option<&str>); 4] = [
    ("terminal", "Terminal", "Terminal.app", Some("/System/Applications/Utilities")),
    ("iterm", "iTerm", "iTerm.app", None),
    ("ghostty", "Ghostty", "Ghostty.app", None),
    ("cmux", "cmux", "cmux.app", None),
];

pub fn installed() -> Vec<TerminalApp> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    TERMINALS
        .iter()
        .filter(|(_, _, bundle, system)| {
            let mut dirs = vec![PathBuf::from("/Applications"), home.join("Applications")];
            dirs.extend(system.map(PathBuf::from));
            dirs.iter().any(|d| d.join(bundle).is_dir())
        })
        .map(|&(id, label, _, _)| TerminalApp { id, label })
        .collect()
}

pub struct Session<'a> {
    pub source: &'a str,
    pub native_id: &'a str,
    pub path: &'a str,
    pub cwd: Option<&'a str>,
}

/// The same command the frontend copies (`resumeCommand` in lib/format.ts).
pub fn resume_command(s: &Session) -> Result<String, String> {
    let cd = s.cwd.map(|c| format!("cd {} && ", shell_quote(c))).unwrap_or_default();
    match s.source {
        "claude" => Ok(format!("{cd}claude --resume {}", shell_quote(s.native_id))),
        "codex" => Ok(format!("{cd}codex resume {}", shell_quote(s.native_id))),
        "pi" => Ok(format!("{cd}pi --session {}", shell_quote(s.path))),
        other => Err(format!("unknown agent: {other}")),
    }
}

fn shell_quote(s: &str) -> String {
    let plain = !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"@%+=:,./-_".contains(&b));
    if plain { s.to_string() } else { format!("'{}'", s.replace('\'', r"'\''")) }
}

/// An AppleScript string literal.
fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', r"\\").replace('"', "\\\""))
}

/// Opens a new window (a new workspace in cmux) in `app` and runs `command`.
/// If the app isn't running, it uses the window the app opens at launch
/// instead of adding a second one.
pub fn script(app: &str, command: &str, cwd: Option<&str>) -> Result<String, String> {
    let cmd = quoted(command);
    // Waits up to 5 s for the window a freshly launched app opens.
    let wait = "repeat 50 times\n      if (count of windows) > 0 then exit repeat\n      delay 0.1\n    end repeat";
    let s = match app {
        "terminal" => format!(
            r#"set wasRunning to application "Terminal" is running
tell application "Terminal"
  if not wasRunning then
    launch
    {wait}
  end if
  if wasRunning or (count of windows) = 0 then
    do script {cmd}
  else
    do script {cmd} in front window
  end if
  activate
end tell"#
        ),
        "iterm" => format!(
            r#"set wasRunning to application "iTerm" is running
tell application "iTerm"
  activate
  if not wasRunning then
    {wait}
  end if
  if wasRunning or (count of windows) = 0 then
    set w to (create window with default profile)
  else
    set w to current window
  end if
  tell current session of w to write text {cmd}
end tell"#
        ),
        "ghostty" => {
            let dir = cwd.map(|c| format!("\n  set initial working directory of cfg to {}", quoted(c))).unwrap_or_default();
            // "initial input" is typed into the new shell; `input text` would
            // paste, and a pasted newline doesn't run the command.
            format!(
                r#"set wasRunning to application "Ghostty" is running
tell application "Ghostty"
  activate
  if not wasRunning then
    {wait}
  end if
  if wasRunning or (count of windows) = 0 then
    set cfg to new surface configuration{dir}
    set initial input of cfg to {cmd} & return
    new window with configuration cfg
  else
    set t to focused terminal of selected tab of front window
    input text {cmd} to t
    send key "enter" to t
  end if
end tell"#
            )
        }
        "cmux" => {
            // Ghostty's `text:` action types the text as if from the keyboard
            // (it reads escapes like a Zig string, hence the doubled
            // backslashes), so the trailing \r runs the command.
            let action = quoted(&format!(r"text:{}\r", command.replace('\\', r"\\")));
            format!(
                r#"set wasRunning to application "cmux" is running
tell application "cmux"
  activate
  if not wasRunning then
    {wait}
  end if
  if (count of windows) = 0 then
    set t to selected tab of (new window)
  else if wasRunning then
    set t to new tab in front window
  else
    set t to selected tab of front window
  end if
  perform action {action} on focused terminal of t
end tell"#
            )
        }
        other => return Err(format!("unknown terminal: {other}")),
    };
    Ok(s)
}

pub fn open(app: &str, session: &Session) -> Result<(), String> {
    let command = resume_command(session)?;
    let cwd = session.cwd.filter(|c| Path::new(c).is_dir());
    let out = Command::new("osascript").arg("-e").arg(script(app, &command, cwd)?).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_like_the_copied_command() {
        let s = Session { source: "pi", native_id: "x", path: "/tmp/it's here.jsonl", cwd: Some("/tmp/a b") };
        assert_eq!(resume_command(&s).unwrap(), r"cd '/tmp/a b' && pi --session '/tmp/it'\''s here.jsonl'");
        let s = Session { source: "claude", native_id: "e420ccf1-2c30", path: "", cwd: None };
        assert_eq!(resume_command(&s).unwrap(), "claude --resume e420ccf1-2c30");
    }

    #[test]
    fn escapes_applescript_strings() {
        assert_eq!(quoted(r#"say "hi" \ bye"#), r#""say \"hi\" \\ bye""#);
    }

    /// Compiles each script against the installed app's dictionary without
    /// running it (`osacompile` only reads the dictionary). Run with
    /// `cargo test -- --ignored` on a Mac that has the apps.
    #[test]
    #[ignore]
    fn scripts_compile() {
        let compiled = std::env::temp_dir().join("session-player-terminal.scpt");
        for app in installed() {
            let s = script(app.id, r#"cd '/tmp/a b' && echo "hi""#, Some("/tmp")).unwrap();
            let out = Command::new("osacompile").arg("-o").arg(&compiled).args(["-e", &s]).output().unwrap();
            assert!(out.status.success(), "{}: {}", app.id, String::from_utf8_lossy(&out.stderr));
        }
    }
}
