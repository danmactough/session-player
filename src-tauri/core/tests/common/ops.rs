// Shared by the operation tests with `#[path = "common/ops.rs"] mod common;`: a Ctx on a temp
// folder with a fixed clock, session files to index, a fake `claude`, and job and event helpers.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use session_player_core::model::SessionRow;
use session_player_core::ops::{
    self, Ctx, EventsSince, JOB, Job, JobId, JobRequest, JobState, LIST_SESSIONS, ListSessionsRequest, OpenParams, Paths, PushedEvent, REINDEX,
    ReindexRequest,
};
use session_player_core::test_support::{annotations, generate, index};

pub const CLOCK: u64 = 7_000_000;

pub fn paths(dir: &Path) -> Paths {
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    Paths { db: data.join("index.sqlite"), library: data.join("library"), logs: data.join("logs"), data }
}

pub struct Open<'a> {
    pub dir: &'a Path,
    pub claude: Option<generate::Claude>,
    pub upload_cap: Option<u64>,
    /// Registers `<home>/.claude/projects` as a session folder.
    pub register_claude_root: bool,
}

/// A Ctx on `dir/data` with home `dir/home`.
pub fn open(params: Open) -> Arc<Ctx> {
    let home = params.dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let paths = paths(params.dir);
    if params.register_claude_root {
        let root = home.join(".claude/projects");
        std::fs::create_dir_all(&root).unwrap();
        let conn = index::open(&paths.db).unwrap();
        index::add_root(&conn, &root.to_string_lossy()).unwrap();
    }
    Ctx::open(OpenParams {
        paths,
        home,
        version: "9.9.9".to_string(),
        claude: params.claude,
        clock: Some(Arc::new(|| CLOCK)),
        event_capacity: None,
        upload_cap: params.upload_cap,
        extensions: vec![],
    })
    .expect("open the Ctx")
}

/// A Ctx whose home's Claude Code folder is a session folder.
pub fn open_simple(dir: &Path, claude: Option<generate::Claude>) -> Arc<Ctx> {
    open(Open { dir, claude, upload_cap: None, register_claude_root: true })
}

pub fn home(dir: &Path) -> PathBuf {
    dir.join("home")
}

/// A checkout of a repository: a folder with a `.git` folder, named like the repository.
pub fn checkout(base: &Path, name: &str) -> PathBuf {
    let project = base.join("code").join(name);
    std::fs::create_dir_all(project.join(".git")).unwrap();
    project
}

pub struct SessionFile<'a> {
    pub home: &'a Path,
    pub cwd: &'a Path,
    pub id: &'a str,
    pub prompt: &'a str,
    /// (media type, base64 data) of images in the prompt.
    pub images: &'a [(&'a str, &'a str)],
    /// Image URLs that aren't data URLs, sent in the prompt too.
    pub linked_images: &'a [&'a str],
}

/// Writes a one-turn Claude Code session and returns its path.
pub fn session_file(s: SessionFile) -> PathBuf {
    let folder: String = s.cwd.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let folder = s.home.join(".claude/projects").join(folder);
    std::fs::create_dir_all(&folder).unwrap();
    let content = if s.images.is_empty() && s.linked_images.is_empty() {
        json!(s.prompt)
    } else {
        let mut parts = vec![json!({ "type": "text", "text": s.prompt })];
        for (mime, data) in s.images {
            parts.push(json!({ "type": "image", "source": { "type": "base64", "media_type": mime, "data": data } }));
        }
        for url in s.linked_images {
            parts.push(json!({ "type": "image", "image_url": url }));
        }
        json!(parts)
    };
    let user = json!({ "type": "user", "sessionId": s.id, "cwd": s.cwd, "timestamp": "2026-11-01T10:00:00Z",
                       "message": { "role": "user", "content": content } });
    let assistant = json!({ "type": "assistant", "sessionId": s.id, "timestamp": "2026-11-01T10:00:30Z",
                            "message": { "id": "m1", "model": "claude-opus-5-5", "role": "assistant",
                                         "content": [{ "type": "text", "text": "Done." }] } });
    let path = folder.join(format!("{}.jsonl", s.id));
    std::fs::write(&path, format!("{user}\n{assistant}\n")).unwrap();
    path
}

/// A plain session under the home's Claude Code folder, held in `home/code/app`.
pub fn session(dir: &Path, id: &str, prompt: &str) -> PathBuf {
    let home = home(dir);
    let cwd = home.join("code/app");
    session_file(SessionFile { home: &home, cwd: &cwd, id, prompt, images: &[], linked_images: &[] })
}

pub fn job(ctx: &Arc<Ctx>, id: &JobId) -> Job {
    ops::call(ctx, &JOB, &JobRequest { id: id.clone() }).expect("job")
}

pub fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits for the job to leave Running, then returns it with the events published by then.
pub fn wait_done(ctx: &Arc<Ctx>, id: &JobId) -> (Job, Vec<PushedEvent>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let job = job(ctx, id);
        if job.state != JobState::Running {
            return (job, events(ctx));
        }
        assert!(Instant::now() < deadline, "{id:?} still running: {job:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn reindex(ctx: &Arc<Ctx>) {
    let id = ops::call(ctx, &REINDEX, &ReindexRequest { full: false }).expect("reindex");
    let (job, _) = wait_done(ctx, &id);
    assert_eq!(job.state, JobState::Finished, "{job:?}");
}

/// Every event published since the Ctx opened.
pub fn events(ctx: &Ctx) -> Vec<PushedEvent> {
    ctx.events_since(EventsSince { after: Some(CLOCK - 1), wait: Duration::ZERO }).into_iter().map(|e| e.event).collect()
}

pub fn rows(ctx: &Arc<Ctx>) -> Vec<SessionRow> {
    ops::call(ctx, &LIST_SESSIONS, &ListSessionsRequest::default()).expect("list_sessions")
}

pub fn row(ctx: &Arc<Ctx>, native_id: &str) -> SessionRow {
    rows(ctx).into_iter().find(|r| r.native_id == native_id).unwrap_or_else(|| panic!("no session {native_id} listed"))
}

pub fn key(row: &SessionRow) -> String {
    annotations::session_key(row)
}

/// Saves an analysis for the session as Analyze would, so it counts as analyzed.
pub fn mark_analyzed(ctx: &Ctx, key: &str) {
    let conn = index::open(&ctx.paths().db).unwrap();
    let meta = annotations::SessionMeta {
        key: key.to_string(),
        outcome: annotations::Outcome::Succeeded,
        outcome_note: Some("Finished what was asked".to_string()),
        components: vec!["src".to_string()],
        generated: vec!["outcome".to_string(), "components".to_string()],
        generated_by: Some(annotations::Generation { model: "claude-haiku-5-5".to_string(), at: 1, revision: "r".to_string() }),
        ..Default::default()
    };
    annotations::store_meta(&conn, &meta).unwrap();
}

pub fn is_analyzed(ctx: &Ctx, key: &str) -> bool {
    let conn = index::open(&ctx.paths().db).unwrap();
    annotations::meta(&conn, key).unwrap().is_some_and(|m| m.generated_by.is_some())
}

/// Whether a process is running (a reaped or zombie process is not).
pub fn alive(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat.rsplit_once(") ").is_some_and(|(_, rest)| !rest.starts_with('Z')),
        Err(_) => false,
    }
}

/// The error the fake `claude` fails with: two lines, of which only the first should be shown.
pub const FAILURE_FIRST_LINE: &str = "claude failed (exit status: 1): API Error: 529 overloaded";

/// A stand-in `claude`. It records each call's model and process id, waits while `hold` is on,
/// fails when `fail` is on or the input contains FAIL-ME, and otherwise answers as Analyze
/// (haiku) or a report (any other model) expects.
pub struct FakeClaude {
    pub dir: PathBuf,
}

impl FakeClaude {
    pub fn new(dir: &Path) -> FakeClaude {
        use std::os::unix::fs::PermissionsExt;
        let dir = dir.join("fake-claude");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("analysis.json"), reply(analysis())).unwrap();
        std::fs::write(dir.join("report.json"), reply(report())).unwrap();
        let script = dir.join("claude");
        std::fs::write(
            &script,
            format!(
                r#"#!/bin/sh
dir='{dir}'
model=''
prev=''
for a in "$@"; do
  [ "$prev" = --model ] && model="$a"
  prev="$a"
done
input=$(cat)
echo "$model" >> "$dir/models"
echo $$ >> "$dir/pids"
while [ -f "$dir/hold" ]; do sleep 0.05; done
case "$input" in *FAIL-ME*) touch "$dir/failing-$$";; esac
if [ -f "$dir/fail" ] || [ -f "$dir/failing-$$" ]; then
  echo 'API Error: 529 overloaded' >&2
  echo 'retry later' >&2
  exit 1
fi
if [ "$model" = haiku ]; then cat "$dir/analysis.json"; else cat "$dir/report.json"; fi
"#,
                dir = dir.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        FakeClaude { dir }
    }

    pub fn claude(&self) -> generate::Claude {
        generate::Claude { bin: self.dir.join("claude"), env: vec![], api_key_helper: None, log: None }
    }

    /// Every call waits until `release`.
    pub fn hold(&self) {
        std::fs::write(self.dir.join("hold"), "").unwrap();
    }

    pub fn release(&self) {
        let _ = std::fs::remove_file(self.dir.join("hold"));
    }

    /// Every call fails.
    pub fn fail(&self) {
        std::fs::write(self.dir.join("fail"), "").unwrap();
    }

    /// The process id of each call so far.
    pub fn pids(&self) -> Vec<u32> {
        self.lines("pids").iter().map(|l| l.parse().unwrap()).collect()
    }

    /// The `--model` of each call so far.
    pub fn models(&self) -> Vec<String> {
        self.lines("models")
    }

    fn lines(&self, name: &str) -> Vec<String> {
        std::fs::read_to_string(self.dir.join(name)).unwrap_or_default().lines().map(str::to_string).collect()
    }

    /// Waits until `n` calls have started.
    pub fn wait_for_calls(&self, n: usize) {
        wait_until(&format!("{n} claude calls started"), || self.pids().len() >= n);
    }
}

fn reply(structured: serde_json::Value) -> String {
    json!({
        "type": "result", "is_error": false, "result": structured.to_string(),
        "structured_output": structured,
        "modelUsage": { "claude-haiku-5-5": { "outputTokens": 10 } }
    })
    .to_string()
}

fn analysis() -> serde_json::Value {
    json!({
        "outcome": "succeeded",
        "outcome_note": "Did what was asked",
        "components": ["src"],
        "annotations": [
            { "kind": "key_decision", "anchors": ["e1"], "note": "Started from the prompt", "rationale": "", "confidence": "high" }
        ]
    })
}

fn report() -> serde_json::Value {
    json!({
        "title": "Agents finish small tasks",
        "summary": "The sessions did what was asked.",
        "takeaways": [{ "text": "Small prompts work", "refs": ["s1"] }],
        "actions": [],
        "impact": []
    })
}
