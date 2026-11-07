//! A plain-text log of Analyze and report runs, for finding out why one
//! failed: each run's start and end, and each call to `claude` with its exit
//! status and, when it fails, what it wrote to stderr and stdout.

use std::io::Write;
use std::path::Path;

/// The log is moved to `<name>.1` when it passes this size, replacing the
/// older one, so it never holds more than about twice this.
const MAX_BYTES: u64 = 2_000_000;
/// The most of claude's stderr or stdout kept for one failed call.
pub const MAX_OUTPUT: usize = 8_000;
/// The most of a model's reply kept for one call. A report or an analysis
/// is usually a few thousand characters.
pub const MAX_REPLY: usize = 40_000;

/// Appends `text` to the log at `path` with a timestamp, creating the log and
/// its folder as needed. Further lines of `text` are indented under the
/// first. A log that can't be written is skipped: it must never stop a run.
pub fn append(path: &Path, text: &str) {
    let _ = try_append(path, text);
}

fn try_append(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let mut old = path.as_os_str().to_owned();
        old.push(".1");
        std::fs::rename(path, old)?;
    }
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let body = text.trim_end().replace('\n', "\n    ");
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{stamp}  {body}")
}

/// `text` cut to its last `max` characters (the end of an error output says
/// the most), marked when anything was cut.
pub fn tail(text: &str, max: usize) -> String {
    let text = text.trim();
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let kept: String = text.chars().skip(count - max).collect();
    format!("…{kept}")
}

/// `text` cut to its first `max` characters, marked when anything was cut.
pub fn head(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}…")
}
