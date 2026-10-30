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

pub fn targets(source: &str, native_id: &str, path: &str) -> Vec<OpenTarget> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Vec::new() };
    let path = Path::new(path);
    let mut out = Vec::new();
    // Claude and Codex find a session by id in their own session folder, so
    // any session from there opens in their app.
    if source == "claude" && path.starts_with(home.join(".claude/projects")) && installed(&home, &["Claude.app"]) {
        // A session the app already ran opens directly; any other is imported
        // first. The resume link takes only a CLI session id (a UUID).
        let url = match claude_desktop_session(&home, native_id) {
            Some(id) => Some(format!("claude://code/continue?session={id}")),
            None if is_uuid(native_id) => Some(format!("claude://resume?session={native_id}")),
            None => None,
        };
        if let Some(url) = url {
            out.push(OpenTarget { id: "claude", label: "Open in Claude", url });
        }
    }
    // The Codex desktop app ships inside ChatGPT.app.
    if source == "codex" && path.starts_with(home.join(".codex")) && installed(&home, &["ChatGPT.app"]) {
        out.push(OpenTarget { id: "chatgpt", label: "Open in ChatGPT", url: format!("codex://threads/{native_id}") });
    }
    if installed(&home, &["T3 Code (Beta).app", "T3 Code.app"]) {
        if let Some(url) = t3_thread_url(&home, native_id) {
            out.push(OpenTarget { id: "t3", label: "Open in T3 Code", url });
        }
    }
    out
}

fn is_uuid(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.iter().map(|g| g.len()).eq([8, 4, 4, 4, 12])
        && groups.iter().all(|g| g.bytes().all(|b| b.is_ascii_hexdigit()))
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

    #[test]
    fn recognizes_cli_session_ids() {
        assert!(super::is_uuid("e420ccf1-2c30-4cad-ba98-05b1ae4dfb90"));
        assert!(!super::is_uuid("agent-a1b2c3d4"));
        assert!(!super::is_uuid("e420ccf1-2c30-4cad-ba98-05b1ae4dfb9z"));
    }
}
