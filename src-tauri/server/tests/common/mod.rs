// Shared by the server's integration tests: a Ctx over a temp folder, a real `serve`, the local
// client, and a raw HTTP/1.1 socket for what the client hides (headers, Host, Origin, cookies).
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use session_player_client::{Client, ClientParams, PairingString};
use session_player_core::index;
use session_player_core::model::SessionRow;
use session_player_core::ops::{self, Ctx, Extension, JOB, JobRequest, JobState, LIST_SESSIONS, ListSessionsRequest, OpenParams, Paths, REINDEX, ReindexRequest};
use session_player_server::{SERVER_EXTENSIONS, ServeParams, ServerConfig, ServerHandle, TlsMode, serve};

pub const CLOCK: u64 = 7_000_000;
pub const VERSION: &str = "9.9.9";

/// A clock a test moves forward.
#[derive(Clone)]
pub struct TestClock(pub Arc<AtomicU64>);

impl TestClock {
    pub fn new() -> TestClock {
        TestClock(Arc::new(AtomicU64::new(CLOCK)))
    }

    pub fn advance(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

pub struct CtxParams<'a> {
    pub dir: &'a Path,
    pub clock: Option<TestClock>,
    pub event_capacity: Option<usize>,
    /// Copy three session fixtures under an indexed root.
    pub fixtures: bool,
}

pub fn data_of(dir: &Path) -> PathBuf {
    dir.join("data")
}

pub fn open_ctx(params: CtxParams) -> Arc<Ctx> {
    open_ctx_with(params, OpenWith { upload_cap: None, extensions: SERVER_EXTENSIONS.to_vec() })
}

/// What `open_ctx` doesn't vary.
pub struct OpenWith {
    pub upload_cap: Option<u64>,
    pub extensions: Vec<Extension>,
}

pub fn open_ctx_with(params: CtxParams, with: OpenWith) -> Arc<Ctx> {
    let data = data_of(params.dir);
    std::fs::create_dir_all(&data).unwrap();
    let home = params.dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let paths = Paths { db: data.join("index.sqlite"), library: data.join("library"), logs: data.join("logs"), data: data.clone() };
    if params.fixtures {
        let root = home.join(".claude/projects/fixtures");
        std::fs::create_dir_all(&root).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/fixtures");
        for name in ["claude.jsonl", "codex.jsonl", "pi.jsonl"] {
            std::fs::copy(fixtures.join(name), root.join(name)).unwrap();
        }
        let conn = index::open(&paths.db).unwrap();
        index::add_root(&conn, &home.join(".claude/projects").to_string_lossy()).unwrap();
    }
    let clock = params.clock.unwrap_or_else(TestClock::new);
    Ctx::open(OpenParams {
        paths,
        home,
        version: VERSION.to_string(),
        claude: None,
        clock: Some(Arc::new(move || clock.0.load(Ordering::SeqCst))),
        event_capacity: params.event_capacity,
        upload_cap: with.upload_cap,
        extensions: with.extensions,
    })
    .expect("open the Ctx")
}

/// Loopback only, any free port.
pub fn loopback_config() -> ServerConfig {
    ServerConfig { port: 0, external_port: 0, external_bind: None, advertise: None, tls: TlsMode::SelfSigned }
}

/// A self-signed TLS external listener on 127.0.0.1, advertised as 127.0.0.1.
pub fn external_config() -> ServerConfig {
    ServerConfig {
        port: 0,
        external_port: 0,
        external_bind: Some("127.0.0.1".parse().unwrap()),
        advertise: Some("127.0.0.1".to_string()),
        tls: TlsMode::SelfSigned,
    }
}

pub fn start(ctx: &Arc<Ctx>, config: ServerConfig) -> ServerHandle {
    let data = ctx.paths().data.clone();
    serve(ServeParams { ctx: ctx.clone(), data, config }).expect("start the server")
}

pub fn read_token(data: &Path) -> String {
    std::fs::read_to_string(data.join("server.token")).expect("read server.token").trim().to_string()
}

pub fn local_client(handle: &ServerHandle, data: &Path) -> Client {
    Client::local(handle.addrs().loopback.port(), &read_token(data)).expect("a local client")
}

/// A client for the listener a pairing string points at, holding `token`.
pub fn client_for(pairing: &PairingString, token: Option<String>) -> Client {
    let scheme = if pairing.fingerprint.is_some() { "https" } else { "http" };
    Client::new(ClientParams { url: format!("{scheme}://{}:{}", pairing.address, pairing.port), token, fingerprint: pairing.fingerprint.clone() })
        .expect("a client")
}

/// A port nothing listens on right now.
pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

pub fn wait_until(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn accepts_connections(addr: SocketAddr) -> bool {
    TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}

pub struct RawRequest<'a> {
    pub method: &'a str,
    pub path: &'a str,
    /// None sends `Host: {addr}`.
    pub host: Option<&'a str>,
    pub headers: &'a [(&'a str, &'a str)],
    pub body: &'a [u8],
}

#[derive(Debug)]
pub struct RawResponse {
    pub status: u16,
    /// Lowercase names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RawResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    pub fn headers_named(&self, name: &str) -> Vec<&str> {
        self.headers.iter().filter(|(n, _)| n == name).map(|(_, v)| v.as_str()).collect()
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("body is not JSON ({e}): {}", String::from_utf8_lossy(&self.body)))
    }

    pub fn code(&self) -> String {
        self.json()["code"].as_str().unwrap_or_default().to_string()
    }
}

fn send(addr: SocketAddr, req: &RawRequest) -> TcpStream {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let host = req.host.map(str::to_string).unwrap_or_else(|| addr.to_string());
    let mut head = format!("{} {} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Length: {}\r\n", req.method, req.path, req.body.len());
    for (name, value) in req.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(req.body).unwrap();
    stream
}

fn parse_head(head: &str) -> (u16, Vec<(String, String)>) {
    let mut lines = head.split("\r\n");
    let status = lines.next().and_then(|l| l.split(' ').nth(1)).and_then(|s| s.parse().ok()).expect("a status line");
    let headers = lines
        .filter(|l| !l.is_empty())
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    (status, headers)
}

fn dechunk(mut body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(end) = body.windows(2).position(|w| w == b"\r\n") else { return out };
        let size = usize::from_str_radix(std::str::from_utf8(&body[..end]).unwrap().split(';').next().unwrap().trim(), 16).unwrap();
        if size == 0 {
            return out;
        }
        out.extend_from_slice(&body[end + 2..end + 2 + size]);
        body = &body[end + 2 + size + 2..];
    }
}

/// Sends one request and reads the whole response (the server closes after it).
pub fn raw(addr: SocketAddr, req: RawRequest) -> RawResponse {
    let mut stream = send(addr, &req);
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).expect("read the response");
    let split = bytes.windows(4).position(|w| w == b"\r\n\r\n").expect("a response head");
    let (status, headers) = parse_head(&String::from_utf8_lossy(&bytes[..split]));
    let rest = &bytes[split + 4..];
    let chunked = headers.iter().any(|(n, v)| n == "transfer-encoding" && v.contains("chunked"));
    let body = if chunked { dechunk(rest) } else { rest.to_vec() };
    RawResponse { status, headers, body }
}

/// Sends one request and reads only the response head, for a response that doesn't end (SSE).
pub fn raw_head(addr: SocketAddr, req: RawRequest) -> RawResponse {
    let mut stream = send(addr, &req);
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        let n = stream.read(&mut byte).expect("read the response head");
        assert!(n == 1, "the connection closed before the head ended: {}", String::from_utf8_lossy(&bytes));
        bytes.push(byte[0]);
    }
    let (status, headers) = parse_head(&String::from_utf8_lossy(&bytes[..bytes.len() - 4]));
    RawResponse { status, headers, body: Vec::new() }
}

pub fn post_json(addr: SocketAddr, path: &str, headers: &[(&str, &str)], body: &str) -> RawResponse {
    let mut all = vec![("Content-Type", "application/json")];
    all.extend_from_slice(headers);
    raw(addr, RawRequest { method: "POST", path, host: None, headers: &all, body: body.as_bytes() })
}

pub fn bearer(data: &Path) -> String {
    format!("Bearer {}", read_token(data))
}

/// The Claude Code session folder `open_ctx` registers as a root (with `fixtures: true`).
pub fn claude_root(dir: &Path) -> PathBuf {
    dir.join("home/.claude/projects")
}

/// A checkout of a repository: a folder with a `.git` folder, named like the repository.
pub fn checkout(home: &Path, name: &str) -> PathBuf {
    let project = home.join("code").join(name);
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
}

/// Writes a one-turn Claude Code session under `home/.claude/projects` and returns its path.
pub fn session_file(s: SessionFile) -> PathBuf {
    let folder: String = s.cwd.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let folder = s.home.join(".claude/projects").join(folder);
    std::fs::create_dir_all(&folder).unwrap();
    let content = if s.images.is_empty() {
        serde_json::json!(s.prompt)
    } else {
        let mut parts = vec![serde_json::json!({ "type": "text", "text": s.prompt })];
        for (mime, data) in s.images {
            parts.push(serde_json::json!({ "type": "image", "source": { "type": "base64", "media_type": mime, "data": data } }));
        }
        serde_json::json!(parts)
    };
    let user = serde_json::json!({ "type": "user", "sessionId": s.id, "cwd": s.cwd, "timestamp": "2026-11-01T10:00:00Z",
                       "message": { "role": "user", "content": content } });
    let assistant = serde_json::json!({ "type": "assistant", "sessionId": s.id, "timestamp": "2026-11-01T10:00:30Z",
                            "message": { "id": "m1", "model": "claude-opus-5-5", "role": "assistant",
                                         "content": [{ "type": "text", "text": "Done." }] } });
    let path = folder.join(format!("{}.jsonl", s.id));
    std::fs::write(&path, format!("{user}\n{assistant}\n")).unwrap();
    path
}

/// Runs an incremental index pass in process and waits for it.
pub fn reindex(ctx: &Arc<Ctx>) {
    let id = ops::call(ctx, &REINDEX, &ReindexRequest { full: false }).expect("reindex");
    wait_until("the index pass ends", Duration::from_secs(60), || ops::call(ctx, &JOB, &JobRequest { id: id.clone() }).expect("job").state != JobState::Running);
    let job = ops::call(ctx, &JOB, &JobRequest { id }).expect("job");
    assert_eq!(job.state, JobState::Finished, "{job:?}");
}

pub fn rows(ctx: &Arc<Ctx>) -> Vec<SessionRow> {
    ops::call(ctx, &LIST_SESSIONS, &ListSessionsRequest::default()).expect("list_sessions")
}

pub fn row(ctx: &Arc<Ctx>, native_id: &str) -> SessionRow {
    rows(ctx).into_iter().find(|r| r.native_id == native_id).unwrap_or_else(|| panic!("no session {native_id} listed"))
}

/// Text that compresses poorly.
pub fn noise(len: usize) -> String {
    let mut state: u64 = 0x2545_f491_4f6c_dd1d;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            char::from(b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 "[(state % 63) as usize])
        })
        .collect()
}

/// Every file under `dir`, except SQLite's own journal files.
pub fn files_under(dir: &Path) -> std::collections::BTreeSet<PathBuf> {
    let mut found = std::collections::BTreeSet::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(folder) = todo.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path);
            } else if !path.to_string_lossy().ends_with("-wal") && !path.to_string_lossy().ends_with("-shm") {
                found.insert(path);
            }
        }
    }
    found
}
