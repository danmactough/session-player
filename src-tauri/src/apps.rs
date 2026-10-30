//! Desktop apps that can open a session, and the link that opens it there.
//! Each app is offered only when it is installed and already knows the
//! session, so its link lands on the session instead of an error.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct OpenTarget {
    pub id: &'static str,
    pub label: &'static str,
    pub url: String,
}

/// Schemes `open_in_app` will hand to macOS, so the frontend can't use it to
/// open arbitrary URLs.
pub const SCHEMES: [&str; 3] = ["claude", "codex", "t3code"];

pub fn targets(source: &str, native_id: &str) -> Vec<OpenTarget> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Vec::new() };
    let mut out = Vec::new();
    if source == "claude" && installed(&home, &["Claude.app"]) {
        if let Some(id) = claude_desktop_session(&home, native_id) {
            out.push(OpenTarget { id: "claude", label: "Open in Claude", url: format!("claude://code/continue?session={id}") });
        }
    }
    // The Codex desktop app ships inside ChatGPT.app.
    if source == "codex" && installed(&home, &["ChatGPT.app"]) && codex_knows(&home, native_id) {
        out.push(OpenTarget { id: "chatgpt", label: "Open in ChatGPT", url: format!("codex://threads/{native_id}") });
    }
    if installed(&home, &["T3 Code (Beta).app", "T3 Code.app"]) {
        if let Some(url) = t3_thread_url(&home, native_id) {
            out.push(OpenTarget { id: "t3", label: "Open in T3 Code", url });
        }
    }
    out
}

fn installed(home: &Path, names: &[&str]) -> bool {
    find_app(home, names).is_some()
}

fn find_app(home: &Path, names: &[&str]) -> Option<PathBuf> {
    names
        .iter()
        .flat_map(|n| [Path::new("/Applications").join(n), home.join("Applications").join(n)])
        .find(|p| p.is_dir())
}

fn read_only(path: &Path) -> Option<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()
}

/// The Claude desktop app keeps one record per Code session it ran, naming
/// the CLI session it drives (`cliSessionId`) and its own id (`local_…`),
/// which is what its `claude://code/continue` link takes.
fn claude_desktop_session(home: &Path, cli_session_id: &str) -> Option<String> {
    let base = home.join("Library/Application Support/Claude/claude-code-sessions");
    for account in std::fs::read_dir(base).ok()?.flatten() {
        for org in std::fs::read_dir(account.path()).into_iter().flatten().flatten() {
            for file in std::fs::read_dir(org.path()).into_iter().flatten().flatten() {
                let name = file.file_name();
                let name = name.to_string_lossy();
                if !(name.starts_with("local_") && name.ends_with(".json")) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(file.path()) else { continue };
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
                if v.get("cliSessionId").and_then(|x| x.as_str()) == Some(cli_session_id) {
                    return v.get("sessionId").and_then(|x| x.as_str()).map(str::to_string);
                }
            }
        }
    }
    None
}

/// Whether the Codex app has this thread in its state database.
fn codex_knows(home: &Path, thread_id: &str) -> bool {
    let Some(conn) = read_only(&home.join(".codex/state_5.sqlite")) else { return false };
    conn.query_row("SELECT 1 FROM threads WHERE id = ?1", [thread_id], |_| Ok(()))
        .optional()
        .ok()
        .flatten()
        .is_some()
}

/// T3 Code lists every agent session it runs or imported, keyed by the
/// agent's own session id (`…native-thread:<id>`), with its thread id
/// (`import:claudeAgent:<id>` for imports). Its links name the thread and the
/// local environment.
fn t3_thread_url(home: &Path, native_id: &str) -> Option<String> {
    let dir = home.join(".t3/userdata");
    let environment = std::fs::read_to_string(dir.join("environment-id")).ok()?.trim().to_string();
    let conn = read_only(&dir.join("statev2.sqlite"))?;
    let thread: String = conn
        .query_row(
            "SELECT thread_id FROM orchestration_v2_projection_provider_threads
             WHERE provider_thread_id LIKE '%native-thread:' || ?1 AND thread_id IS NOT NULL
             ORDER BY updated_at DESC LIMIT 1",
            [native_id],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten()?;
    Some(format!("t3code://threads/{}/{}", encode(&environment), encode(&thread)))
}

/// Percent-encodes a path segment, like JavaScript's encodeURIComponent.
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn encodes_like_encode_uri_component() {
        assert_eq!(super::encode("import:claudeAgent:e420-ccf1"), "import%3AclaudeAgent%3Ae420-ccf1");
    }
}
