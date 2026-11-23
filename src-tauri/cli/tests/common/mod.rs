// Shared by the CLI's integration tests with `mod common;`. Each test drives the real `sp` binary
// as one or two "machines": a home folder, a data folder and a folder of stand-in programs put
// first on PATH (a fake `claude`, and `ssh` or `systemctl` where a test needs them).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::json;
use session_player_client::{Client, LocalProbe, PairedServer, ServersStore, probe_local};
use session_player_core::ops::{HELLO, HelloRequest, JOB, Job, JobId, JobRequest, JobState};
use session_player_core::{index, reports};
use session_player_server::{ServerConfig, TlsMode};

pub const SP: &str = env!("CARGO_BIN_EXE_sp");

/// One machine: `{root}/home`, `{root}/data`, `{root}/bin`.
pub struct Machine {
    pub root: PathBuf,
    pub home: PathBuf,
    pub data: PathBuf,
    pub bin: PathBuf,
}

impl Machine {
    pub fn new(base: &Path, name: &str) -> Machine {
        let root = base.join(name);
        let machine = Machine { home: root.join("home"), data: root.join("data"), bin: root.join("bin"), root };
        // Present before the first sp run, so it is registered as a session folder.
        std::fs::create_dir_all(machine.home.join(".claude/projects")).unwrap();
        std::fs::create_dir_all(&machine.data).unwrap();
        std::fs::create_dir_all(&machine.bin).unwrap();
        machine
    }

    /// sp with this machine's environment and nothing inherited but TMPDIR.
    pub fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(SP);
        command
            .args(args)
            .env_clear()
            .env("HOME", &self.home)
            .env("SESSION_PLAYER_DATA_DIR", &self.data)
            .env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()))
            .current_dir(&self.root)
            .stdin(Stdio::null());
        if let Some(tmp) = std::env::var_os("TMPDIR") {
            command.env("TMPDIR", tmp);
        }
        command
    }

    pub fn sp(&self, args: &[&str]) -> Output {
        self.command(args).output().expect("run sp")
    }

    /// A checkout of the repository `name`: a folder with an empty `.git` folder.
    pub fn checkout(&self, name: &str) -> PathBuf {
        let path = self.root.join("code").join(name);
        std::fs::create_dir_all(path.join(".git")).unwrap();
        path
    }

    /// A one-turn Claude Code session started at 10:00 UTC on `day` in `checkout`.
    pub fn session(&self, checkout: &Path, id: &str, day: &str) -> PathBuf {
        claude_session(&self.home, checkout, id, day)
    }

    /// A saved report with `actions` recommended actions, written straight to the database.
    pub fn save_report(&self, id: &str, title: &str, actions: usize) {
        let actions: Vec<serde_json::Value> = (1..=actions)
            .map(|n| json!({ "title": format!("{title} action {n}"), "category": "codebase", "priority": "high", "change": "Change it", "evidence": [] }))
            .collect();
        let report: reports::Report = serde_json::from_value(json!({
            "id": id, "created_at": 1_790_000_000_000i64, "title": title, "model": "sonnet", "prompt_version": 1,
            "sessions": [], "stats": { "sessions": 0, "outcomes": {}, "kinds": {}, "annotations": 0 },
            "summary": format!("{title} summary"), "takeaways": [], "actions": actions, "impact": [], "rejected": 0
        }))
        .unwrap();
        let conn = index::open(&self.data.join("index.sqlite")).unwrap();
        reports::save_report(&conn, &report).unwrap();
    }

    pub fn report_ids(&self) -> Vec<String> {
        let conn = index::open(&self.data.join("index.sqlite")).unwrap();
        reports::reports(&conn).unwrap().into_iter().map(|r| r.id).collect()
    }

    pub fn action_responses(&self, report: &str) -> Vec<reports::ActionResponse> {
        let conn = index::open(&self.data.join("index.sqlite")).unwrap();
        reports::action_responses(&conn, report).unwrap()
    }

    /// Puts the fake `claude` first on PATH.
    pub fn fake_claude(&self) -> FakeClaude {
        FakeClaude::new(&self.bin)
    }

    /// A stand-in program on PATH that records its arguments to `{bin}/{name}.calls` and prints
    /// `stdout` and exits with `code`.
    pub fn stub(&self, name: &str, stdout: &str, code: i32) {
        use std::os::unix::fs::PermissionsExt;
        let script = self.bin.join(name);
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho \"$*\" >> '{calls}'\nprintf '%s' '{stdout}'\nexit {code}\n", calls = self.bin.join(format!("{name}.calls")).display()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    pub fn stub_calls(&self, name: &str) -> Vec<String> {
        std::fs::read_to_string(self.bin.join(format!("{name}.calls"))).unwrap_or_default().lines().map(str::to_string).collect()
    }

    pub fn servers(&self) -> Vec<PairedServer> {
        ServersStore::new(&self.data).list().unwrap()
    }

    /// Starts `sp serve` with `config` written to server.json and waits until it answers hello.
    pub fn serve(&self, config: &ServerConfig) -> Server {
        self.serve_with(config, &[])
    }

    /// `sp serve {args}` with `config` written to server.json, once it answers hello.
    pub fn serve_with(&self, config: &ServerConfig, args: &[&str]) -> Server {
        std::fs::write(self.data.join("server.json"), serde_json::to_vec_pretty(config).unwrap()).unwrap();
        let stdout = std::fs::File::create(self.root.join("serve-stdout.log")).unwrap();
        let stderr = std::fs::File::create(self.root.join("serve-stderr.log")).unwrap();
        let mut command_args = vec!["serve"];
        command_args.extend_from_slice(args);
        let child = self.command(&command_args).stdout(stdout).stderr(stderr).spawn().expect("start sp serve");
        let mut server = Server { child: Some(child), root: self.root.clone(), data: self.data.clone() };
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = server.child.as_mut().unwrap().try_wait().unwrap() {
                server.child = None;
                panic!("sp serve exited with {status} before answering\n{}", server_logs(&self.root));
            }
            if server.client().is_some_and(|c| c.call(&HELLO, &HelloRequest {}).is_ok()) {
                return server;
            }
            assert!(Instant::now() < deadline, "sp serve didn't answer hello in 30 s\n{}", server_logs(&self.root));
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// `sp serve` listening on loopback only.
    pub fn serve_local(&self) -> Server {
        self.serve(&loopback_config())
    }

    /// `sp serve` that opens a self-signed TLS listener on 127.0.0.1 once a pairing is offered.
    pub fn serve_external(&self) -> Server {
        self.serve(&external_config())
    }
}

pub fn loopback_config() -> ServerConfig {
    ServerConfig { port: 0, external_port: 0, external_bind: None, advertise: None, tls: TlsMode::SelfSigned }
}

pub fn external_config() -> ServerConfig {
    ServerConfig { port: 0, external_port: 0, external_bind: Some("127.0.0.1".parse().unwrap()), advertise: Some("127.0.0.1".into()), tls: TlsMode::SelfSigned }
}

/// A running `sp serve`. Dropping it sends SIGTERM and kills it if it hasn't exited in 5 s.
pub struct Server {
    child: Option<Child>,
    root: PathBuf,
    data: PathBuf,
}

impl Server {
    /// What `sp serve` has written to stderr so far.
    pub fn stderr(&self) -> String {
        std::fs::read_to_string(self.root.join("serve-stderr.log")).unwrap_or_default()
    }

    pub fn pid(&self) -> u32 {
        self.child.as_ref().expect("the server runs").id()
    }

    /// The local client, as the server's own data folder allows.
    pub fn client(&self) -> Option<Client> {
        match probe_local(&self.data) {
            LocalProbe::Live { port, token: Some(token) } => Client::local(port, &token).ok(),
            _ => None,
        }
    }

    pub fn job(&self, id: &JobId) -> Job {
        self.client().expect("the server is live").call(&JOB, &JobRequest { id: id.clone() }).expect("job")
    }

    pub fn wait_for_state(&self, id: &JobId, state: JobState) -> Job {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let job = self.job(id);
            if job.state == state {
                return job;
            }
            assert!(Instant::now() < deadline, "job {} is {:?}, not {state:?}\n{}", id.0, job.state, server_logs(&self.root));
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// SIGTERM, then the exit status (None when it had to be killed after 10 s).
    pub fn terminate(&mut self) -> Option<ExitStatus> {
        let mut child = self.child.take()?;
        signal(child.id(), "TERM");
        let status = wait_exit(&mut child, Duration::from_secs(10));
        if status.is_none() {
            let _ = child.kill();
            let _ = child.wait();
        }
        status
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            signal(child.id(), "TERM");
            if wait_exit(&mut child, Duration::from_secs(5)).is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

/// Sends a signal to one process. Never to a group: see cycle-rules.md, "Killing processes".
pub fn signal(pid: u32, name: &str) {
    let status = Command::new("/bin/sh").args(["-c", r#"kill -s "$1" "$2""#, "sh", name, &pid.to_string()]).status().unwrap();
    assert!(status.success(), "kill -s {name} {pid} failed");
}

pub fn wait_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Panics with both streams unless sp exited 0.
pub fn ok(output: &Output, what: &str) -> String {
    assert_eq!(output.status.code(), Some(0), "{what} failed\nstdout: {}\nstderr: {}", stdout(output), stderr(output));
    stdout(output)
}

pub fn pairing_string(output: &Output) -> String {
    stdout(output)
        .split_whitespace()
        .find(|word| word.starts_with("sp1."))
        .unwrap_or_else(|| panic!("no pairing string in: {}", stdout(output)))
        .to_string()
}

/// Pairs `client` with the server running on `server`'s data folder, saved as `name`.
pub fn pair(server: &Machine, client: &Machine, name: &str) {
    let pairing = pairing_string(&server.sp(&["pair"]));
    ok(&client.sp(&["connect", &pairing, "--name", name]), "sp connect");
}

/// Every log under `root` (sp serve's output, server.log, analyze.log), because a failure on the
/// server is only explained there and the folder is gone when the test ends.
pub fn server_logs(root: &Path) -> String {
    fn walk(folder: &Path, out: &mut String) {
        let Ok(entries) = std::fs::read_dir(folder) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "log") {
                out.push_str(&format!("--- {}\n{}\n", path.display(), std::fs::read_to_string(&path).unwrap_or_default()));
            }
        }
    }
    let mut out = String::new();
    walk(root, &mut out);
    out
}

/// Writes a one-turn Claude Code session in `checkout` under `home`'s Claude Code folder.
pub fn claude_session(home: &Path, checkout: &Path, id: &str, day: &str) -> PathBuf {
    let folder: String = checkout.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let folder = home.join(".claude/projects").join(folder);
    std::fs::create_dir_all(&folder).unwrap();
    let user = json!({ "type": "user", "sessionId": id, "cwd": checkout, "timestamp": format!("{day}T10:00:00Z"),
                       "message": { "role": "user", "content": format!("Prompt of {id}") } });
    let assistant = json!({ "type": "assistant", "sessionId": id, "timestamp": format!("{day}T10:00:30Z"),
                            "message": { "id": "m1", "model": "claude-opus-5-5", "role": "assistant",
                                         "content": [{ "type": "text", "text": "Done." }] } });
    let path = folder.join(format!("{id}.jsonl"));
    std::fs::write(&path, format!("{user}\n{assistant}\n")).unwrap();
    path
}

/// Finds files named `name` anywhere under `folder`.
pub fn find_files(folder: &Path, name: &str) -> Vec<PathBuf> {
    fn walk(folder: &Path, name: &str, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(folder) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, name, out);
            } else if path.file_name().is_some_and(|n| n == name) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(folder, name, &mut out);
    out
}

/// A stand-in `claude` (as in core/tests/common/ops.rs). It records each call's model and process
/// id, waits while `hold` is on, and answers as Analyze (haiku) or a report (any other model)
/// expects. Dropping it releases the hold, so no held call outlives the test.
pub struct FakeClaude {
    pub dir: PathBuf,
}

impl FakeClaude {
    pub fn new(bin: &Path) -> FakeClaude {
        use std::os::unix::fs::PermissionsExt;
        let dir = bin.join("fake-claude");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("analysis.json"), reply(analysis())).unwrap();
        std::fs::write(dir.join("report.json"), reply(report())).unwrap();
        let script = bin.join("claude");
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
if [ -f "$dir/fail" ] && printf '%s' "$input" | grep -qF "$(cat "$dir/fail")"; then
  echo 'claude broke' >&2
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

    /// Every call waits until `release`.
    pub fn hold(&self) {
        std::fs::write(self.dir.join("hold"), "").unwrap();
    }

    pub fn release(&self) {
        let _ = std::fs::remove_file(self.dir.join("hold"));
    }

    /// Calls whose input contains `text` exit 1 with "claude broke" on stderr.
    pub fn fail_on(&self, text: &str) {
        std::fs::write(self.dir.join("fail"), text).unwrap();
    }

    /// The `--model` of each call so far.
    pub fn models(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("models")).unwrap_or_default().lines().map(str::to_string).collect()
    }

    pub fn wait_for_calls(&self, n: usize) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.models().len() < n {
            assert!(Instant::now() < deadline, "timed out waiting for {n} claude calls");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for FakeClaude {
    fn drop(&mut self) {
        self.release();
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

pub const FAKE_REPORT_TITLE: &str = "Agents finish small tasks";

fn report() -> serde_json::Value {
    json!({
        "title": FAKE_REPORT_TITLE,
        "summary": "The sessions did what was asked.",
        "takeaways": [{ "text": "Small prompts work", "refs": ["s1"] }],
        "actions": [],
        "impact": []
    })
}
