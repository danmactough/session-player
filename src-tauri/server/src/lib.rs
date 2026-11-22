//! The Session Player server: `POST /api/v1/<operation>` and one event stream over the operations in
//! session-player-core. See .plans/server-api/ for the design.

pub mod assets;
pub mod auth;
pub mod listener;
pub mod lockfile;
pub mod pairing;
pub mod service;
pub mod stream;
pub mod tls;
mod watcher;

use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use http_body_util::channel::Channel;
use ring::rand::SecureRandom;
use serde::{Deserialize, Serialize};
use session_player_core::ops::{
    Access, ApiError, Body, Ctx, DeviceId, DispatchParams, ErrorCode, EventsSince, Input, OpError, ImageRef, OpSpec, Output, RawBody, RawData, ServerEvent, dispatch,
    spec_of,
};
use tokio::sync::{Notify, broadcast};

use auth::{CheckParams, HostParams, Listener, Refusal, error_response, forbidden, unauthenticated};
pub use pairing::SERVER_EXTENSIONS;

/// The loopback listener's port unless `server.json` says otherwise. The external listener
/// defaults to the next port, because a wildcard bind on a port conflicts with 127.0.0.1 on it.
pub const DEFAULT_PORT: u16 = 47800;

/// What `server.json` holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// The loopback listener (plain HTTP on 127.0.0.1). 0 binds any free port (tests only).
    pub port: u16,
    /// The external listener. Never equal to `port`; 0 binds any free port (tests only).
    pub external_port: u16,
    /// None: the server never listens off loopback.
    pub external_bind: Option<IpAddr>,
    /// The host name or address paired devices reach the server at. It goes into pairing
    /// strings and the generated certificate.
    pub advertise: Option<String>,
    pub tls: TlsMode,
}

impl Default for ServerConfig {
    fn default() -> ServerConfig {
        ServerConfig { port: DEFAULT_PORT, external_port: DEFAULT_PORT + 1, external_bind: None, advertise: None, tls: TlsMode::SelfSigned }
    }
}

/// How the external listener serves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum TlsMode {
    /// A certificate generated into `tls/`, pinned by fingerprint.
    SelfSigned,
    /// The user's own certificate and key (PEM).
    OwnCert { cert: PathBuf, key: PathBuf },
    /// Plain HTTP off loopback, only with `--allow-insecure-http`.
    Plain,
}

pub struct ServeParams {
    /// Opened with `SERVER_EXTENSIONS` among its extensions.
    pub ctx: Arc<Ctx>,
    /// The folder holding server.lock, server.token, devices.json and tls/.
    pub data: PathBuf,
    pub config: ServerConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundAddrs {
    pub loopback: SocketAddr,
    /// The external listener's address while it is open.
    pub external: Option<SocketAddr>,
}

pub struct ServerHandle {
    loopback: SocketAddr,
    shared: Arc<Shared>,
    running: Option<Running>,
}

struct Running {
    runtime: tokio::runtime::Runtime,
    pump: std::thread::JoinHandle<()>,
    stop: Arc<AtomicBool>,
    watcher: Option<watcher::Watcher>,
    lock: lockfile::ServerLock,
}

impl ServerHandle {
    pub fn addrs(&self) -> BoundAddrs {
        BoundAddrs { loopback: self.loopback, external: self.shared.external() }
    }

    /// Stops both listeners and releases server.lock before returning.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        let Some(running) = self.running.take() else { return };
        running.stop.store(true, Ordering::SeqCst);
        if let Some(watcher) = running.watcher {
            watcher.stop();
        }
        // Dropping the runtime's tasks closes both listeners and every open connection.
        running.runtime.shutdown_timeout(Duration::from_secs(2));
        let _ = running.pump.join();
        RUNNING.lock().unwrap().retain(|s| !Arc::ptr_eq(s, &self.shared));
        *self.shared.external.lock().unwrap() = None;
        self.shared.ctx.set_external_open(false);
        drop(running.lock);
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// What the request handlers, the listener controller and the pairing operations share.
pub(crate) struct Shared {
    pub(crate) ctx: Arc<Ctx>,
    pub(crate) config: ServerConfig,
    /// The configured advertise host, else the external bind address.
    pub(crate) advertise: Option<String>,
    pub(crate) loopback: SocketAddr,
    external: Mutex<Option<SocketAddr>>,
    /// The external listener's certificate fingerprint; None when it serves plain HTTP.
    pub(crate) fingerprint: Option<String>,
    tls: Option<Arc<rustls::ServerConfig>>,
    /// Wakes the listener controller after a pair, connect or unpair.
    pub(crate) nudge: Notify,
    local_token: [u8; 32],
    events: broadcast::Sender<ServerEvent>,
}

impl Shared {
    pub(crate) fn external(&self) -> Option<SocketAddr> {
        *self.external.lock().unwrap()
    }
}

/// The servers running in this process. Extension handlers get only the Ctx, so the pairing
/// operations find their server here.
static RUNNING: Mutex<Vec<Arc<Shared>>> = Mutex::new(Vec::new());

pub(crate) fn running(ctx: &Arc<Ctx>) -> Option<Arc<Shared>> {
    RUNNING.lock().unwrap().iter().find(|s| Arc::ptr_eq(&s.ctx, ctx)).cloned()
}

fn random_bytes(n: usize) -> anyhow::Result<Vec<u8>> {
    let mut bytes = vec![0u8; n];
    ring::rand::SystemRandom::new().fill(&mut bytes).map_err(|_| anyhow::anyhow!("The system's random number generator failed"))?;
    Ok(bytes)
}

/// `n` random bytes as base64url without padding.
pub(crate) fn random_token(n: usize) -> anyhow::Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(random_bytes(n)?))
}

pub(crate) fn random_hex(n: usize) -> anyhow::Result<String> {
    Ok(auth::hex(&random_bytes(n)?))
}

/// Owner-only, replaced by a rename so a reader never sees a partial file.
fn write_private(file: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let temp = file.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = options.open(&temp).and_then(|mut f| f.write_all(bytes)).and_then(|()| std::fs::rename(&temp, file));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.with_context(|| format!("Couldn't write {}", file.display()))
}

/// Takes server.lock, writes a new server.token, binds the loopback listener and starts the
/// external listener controller on the server's own tokio runtime.
pub fn serve(params: ServeParams) -> anyhow::Result<ServerHandle> {
    tls::install_provider();
    let ServeParams { ctx, data, config } = params;
    let mut lock = lockfile::acquire(&data)?;
    let token = random_token(32)?;
    write_private(&data.join("server.token"), token.as_bytes())?;
    let loopback_listener = std::net::TcpListener::bind(("127.0.0.1", config.port)).with_context(|| format!("Couldn't listen on 127.0.0.1:{}", config.port))?;
    let loopback = loopback_listener.local_addr()?;
    lock.record_port(loopback.port())?;

    let advertise = config.advertise.clone().or_else(|| config.external_bind.map(|ip| ip.to_string()));
    let (tls, fingerprint) = match (config.external_bind, &config.tls) {
        (None, _) | (Some(_), TlsMode::Plain) => (None, None),
        (Some(bind), TlsMode::SelfSigned) => {
            let identity = tls::self_signed(tls::IdentityParams { dir: &data.join("tls"), advertise: advertise.as_deref().unwrap_or_default(), bind: Some(bind) })?;
            (Some(Arc::new(tls::server_config(vec![identity.cert_der], identity.key_der)?)), Some(identity.fingerprint))
        }
        (Some(_), TlsMode::OwnCert { cert, key }) => {
            let (chain, key, fingerprint) = tls::own_cert(cert, key)?;
            (Some(Arc::new(tls::server_config(chain, key)?)), Some(fingerprint))
        }
    };

    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("sp-server").enable_all().build()?;
    let (events, _) = broadcast::channel(256);
    let shared = Arc::new(Shared {
        ctx: ctx.clone(),
        config,
        advertise,
        loopback,
        external: Mutex::new(None),
        fingerprint,
        tls,
        nudge: Notify::new(),
        local_token: auth::hash_token(&token),
        events: events.clone(),
    });

    let stop = Arc::new(AtomicBool::new(false));
    let pump = start_pump(&ctx, events, stop.clone());
    // A server that can't watch still serves; new sessions then wait for a reindex.
    let watcher = watcher::start(&ctx).inspect_err(|e| eprintln!("Couldn't watch the session folders: {e:#}")).ok();

    {
        let _entered = runtime.enter();
        let info = ListenerInfo {
            kind: Listener::Loopback,
            port: loopback.port(),
            own_origins: vec![format!("http://127.0.0.1:{}", loopback.port()), format!("http://localhost:{}", loopback.port())],
            tls: false,
        };
        let server = axum_server::from_tcp(loopback_listener)?;
        runtime.spawn(server.serve(router(shared.clone(), info).into_make_service()));
        runtime.spawn(control_external(shared.clone()));
    }
    RUNNING.lock().unwrap().push(shared.clone());
    Ok(ServerHandle { loopback, shared, running: Some(Running { runtime, pump, stop, watcher, lock }) })
}

/// The one thread that reads the core event ring and hands each event to every connected stream.
fn start_pump(ctx: &Arc<Ctx>, events: broadcast::Sender<ServerEvent>, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    let ctx = ctx.clone();
    let mut cursor = newest_event(&ctx).id;
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            for event in ctx.events_since(EventsSince { after: Some(cursor), wait: Duration::from_millis(100) }) {
                cursor = event.id;
                let _ = events.send(event);
            }
        }
    })
}

/// A Snapshot carrying the newest event id.
fn newest_event(ctx: &Ctx) -> ServerEvent {
    ctx.events_since(EventsSince { after: Some(u64::MAX), wait: Duration::ZERO }).remove(0)
}

/// Opens the external listener while it is wanted and closes it a grace period after it stops
/// being wanted.
async fn control_external(shared: Arc<Shared>) {
    let Some(bind) = shared.config.external_bind else { return };
    let mut open: Option<(axum_server::Handle<SocketAddr>, tokio::task::JoinHandle<std::io::Result<()>>)> = None;
    let mut last_wanted = Instant::now();
    let mut reported = false;
    loop {
        let now = shared.ctx.now();
        let open_offers = shared.ctx.offers().open.iter().filter(|o| o.expires_at > now).count();
        let wanted = listener::wanted(&listener::Wanted { external_bind: true, devices: shared.ctx.devices().len(), open_offers });
        if wanted {
            last_wanted = Instant::now();
            if open.is_none() {
                match open_external(&shared, bind) {
                    Ok((addr, handle, task)) => {
                        *shared.external.lock().unwrap() = Some(addr);
                        shared.ctx.set_external_open(true);
                        open = Some((handle, task));
                        reported = false;
                    }
                    Err(e) if !reported => {
                        eprintln!("Couldn't open the external listener: {e:#}");
                        reported = true;
                    }
                    Err(_) => {}
                }
            }
        } else if last_wanted.elapsed() >= listener::CLOSE_GRACE
            && let Some((handle, task)) = open.take()
        {
            handle.shutdown();
            task.abort();
            // Waiting for the task drops its listening socket, so the port is free again before
            // `addrs` says the listener is closed.
            let _ = task.await;
            *shared.external.lock().unwrap() = None;
            shared.ctx.set_external_open(false);
        }
        tokio::select! {
            () = tokio::time::sleep(listener::POLL) => {}
            () = shared.nudge.notified() => {}
        }
    }
}

type ExternalTask = (SocketAddr, axum_server::Handle<SocketAddr>, tokio::task::JoinHandle<std::io::Result<()>>);

fn open_external(shared: &Arc<Shared>, bind: IpAddr) -> anyhow::Result<ExternalTask> {
    let listener = std::net::TcpListener::bind((bind, shared.config.external_port))?;
    let addr = listener.local_addr()?;
    let scheme = if shared.tls.is_some() { "https" } else { "http" };
    let advertise = shared.advertise.clone().unwrap_or_default();
    let info = ListenerInfo { kind: Listener::External, port: addr.port(), own_origins: vec![format!("{scheme}://{advertise}:{}", addr.port())], tls: shared.tls.is_some() };
    let app = router(shared.clone(), info).into_make_service();
    let handle = axum_server::Handle::new();
    let task = match &shared.tls {
        Some(config) => {
            let config = axum_server::tls_rustls::RustlsConfig::from_config(config.clone());
            tokio::spawn(axum_server::tls_rustls::from_tcp_rustls(listener, config)?.handle(handle.clone()).serve(app))
        }
        None => tokio::spawn(axum_server::from_tcp(listener)?.handle(handle.clone()).serve(app)),
    };
    Ok((addr, handle, task))
}

struct ListenerInfo {
    kind: Listener,
    /// The port this listener bound.
    port: u16,
    own_origins: Vec<String>,
    tls: bool,
}

#[derive(Clone)]
struct AppState {
    shared: Arc<Shared>,
    listener: Arc<ListenerInfo>,
}

/// Every request goes through `handle`, so the Host, Origin and token checks run before anything
/// learns which path was asked for (axum's own routing would answer 405 for some paths first).
fn router(shared: Arc<Shared>, info: ListenerInfo) -> Router {
    Router::new().fallback(handle).with_state(AppState { shared, listener: Arc::new(info) })
}

const JSON_LIMIT: usize = 16 * 1024 * 1024;
const KEEPALIVE: Duration = Duration::from_secs(3);

fn header<'a>(headers: &'a HeaderMap, name: header::HeaderName) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

async fn handle(State(state): State<AppState>, req: Request) -> Response {
    let origin = header(req.headers(), header::ORIGIN).map(str::to_string);
    let mut response = answer(&state, req, origin.as_deref()).await;
    if let Some(origin) = origin
        && auth::origin_allowed(&origin, &state.listener.own_origins)
        && let Ok(value) = HeaderValue::from_str(&origin)
    {
        response.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
        response.headers_mut().insert(header::VARY, HeaderValue::from_static("Origin"));
    }
    response
}

async fn answer(state: &AppState, req: Request, origin: Option<&str>) -> Response {
    let info = &state.listener;
    let shared = &state.shared;
    // HTTP/2 carries the host in :authority instead of a Host header.
    let host = header(req.headers(), header::HOST).or_else(|| req.uri().authority().map(|a| a.as_str()));
    if !auth::host_allowed(HostParams { listener: info.kind, host, port: info.port, advertise: shared.advertise.as_deref() }) {
        return forbidden();
    }
    if origin.is_some_and(|o| !auth::origin_allowed(o, &info.own_origins)) {
        return forbidden();
    }
    if req.method() == Method::OPTIONS {
        return preflight();
    }
    let path = req.uri().path().to_string();
    if path != "/api" && !path.starts_with("/api/") {
        return assets::response(req.method(), &path);
    }
    let anonymous = path == "/api/v1/connect" && req.method() == Method::POST;
    let device = if anonymous {
        DeviceId("anonymous".to_string())
    } else {
        let devices: Vec<(DeviceId, [u8; 32])> = shared.ctx.devices().into_iter().filter_map(|d| auth::from_hex(&d.token_sha256).map(|h| (d.id, h))).collect();
        let headers = req.headers();
        let checked = auth::check(CheckParams {
            listener: info.kind,
            method: req.method().as_str(),
            authorization: header(headers, header::AUTHORIZATION),
            cookie: header(headers, header::COOKIE),
            csrf: headers.get(auth::CSRF_HEADER).and_then(|v| v.to_str().ok()),
            origin,
            own_origins: &info.own_origins,
            local_token: &shared.local_token,
            devices: &devices,
        });
        match checked {
            Ok(device) => device,
            Err(Refusal::Unauthenticated) => return unauthenticated(),
            Err(Refusal::Forbidden) => return forbidden(),
        }
    };
    if let Some(ids) = path.strip_prefix("/api/v1/image/") {
        if req.method() != Method::GET && req.method() != Method::HEAD {
            return error_response(ErrorCode::MethodNotAllowed, "Images take GET.");
        }
        return image(&shared.ctx, ids, req.headers());
    }
    if path == "/api/v1/events" {
        if req.method() != Method::GET {
            return error_response(ErrorCode::MethodNotAllowed, "The event stream takes GET.");
        }
        let last_id = header(req.headers(), header::HeaderName::from_static("last-event-id")).and_then(|v| v.trim().parse().ok());
        return event_stream(shared.clone(), last_id);
    }
    let Some(name) = path.strip_prefix("/api/v1/").filter(|n| !n.is_empty() && !n.contains('/')) else {
        return error_response(ErrorCode::NotFound, "Nothing is here.");
    };
    let Some(spec) = spec_of(&shared.ctx, name).filter(|s| s.access != Access::NotOverHttp) else {
        return error_response(ErrorCode::UnknownOperation, &format!("No operation is named {name}."));
    };
    if req.method() != Method::POST {
        return error_response(ErrorCode::MethodNotAllowed, "Operations take POST.");
    }
    run_operation(state, spec, device, req).await
}

/// `{generation}/{event}/{image}` from a `load_session` image URL.
fn image(ctx: &Ctx, ids: &str, headers: &HeaderMap) -> Response {
    let not_found = || error_response(ErrorCode::NotFound, "No such image.");
    let [generation, event, image] = ids.split('/').collect::<Vec<_>>()[..] else { return not_found() };
    let (Ok(generation), Ok(event), Ok(image)) = (generation.parse(), event.parse(), image.parse()) else {
        let error = ApiError { code: ErrorCode::BadRequest, message: "An image URL ends in three whole numbers.".to_string() };
        return (StatusCode::BAD_REQUEST, axum::Json(error)).into_response();
    };
    let Ok(found) = ctx.image(ImageRef { generation, event, image }) else { return not_found() };
    let digest = ring::digest::digest(&ring::digest::SHA256, &found.bytes);
    let etag = format!("\"{}\"", auth::hex(&digest.as_ref()[..16]));
    let matched = header(headers, header::IF_NONE_MATCH).is_some_and(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*"));
    let builder = Response::builder().header(header::ETAG, &etag).header(header::CACHE_CONTROL, "private, max-age=3600");
    let built = if matched {
        builder.status(StatusCode::NOT_MODIFIED).body(axum::body::Body::empty())
    } else {
        builder.status(StatusCode::OK).header(header::CONTENT_TYPE, found.content_type).body(axum::body::Body::from(found.bytes))
    };
    built.unwrap_or_else(|e| error_response(ErrorCode::Internal, &e.to_string()))
}

fn preflight() -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    let headers = response.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, OPTIONS"));
    headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("authorization, content-type, last-event-id, x-sp-csrf"));
    headers.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
    response
}

async fn run_operation(state: &AppState, spec: OpSpec, device: DeviceId, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let mut browser = false;
    let input = match spec.body {
        Body::Json => {
            let Ok(bytes) = axum::body::to_bytes(body, JSON_LIMIT).await else {
                return error_response(ErrorCode::TooLarge, "The request body is too large.");
            };
            let value: serde_json::Value = match serde_json::from_slice(&bytes) {
                Ok(value) => value,
                Err(e) => return error_response(ErrorCode::BadRequest, &format!("The body isn't valid JSON: {e}")),
            };
            browser = spec.name == "connect" && value.get("browser") == Some(&serde_json::Value::Bool(true));
            Input::Json(value)
        }
        Body::Raw => {
            let query = Query::<Vec<(String, String)>>::try_from_uri(&parts.uri).map(|q| q.0).unwrap_or_default();
            let content_type = header(&parts.headers, header::CONTENT_TYPE).map(str::to_string);
            Input::Raw { body: Box::new(stream::body_reader(body, None)), content_type, query }
        }
    };
    let ctx = state.shared.ctx.clone();
    let params = DispatchParams { name: spec.name.to_string(), device, input };
    let result = if spec.inline {
        dispatch(&ctx, params)
    } else {
        tokio::task::spawn_blocking(move || dispatch(&ctx, params)).await.unwrap_or_else(|e| Err(OpError::Internal(e.to_string())))
    };
    match result {
        Ok(Output::Json(mut value)) => {
            let cookie = if browser { value.get_mut("token").map(serde_json::Value::take) } else { None };
            let mut response = axum::Json(value).into_response();
            if let Some(serde_json::Value::String(token)) = cookie {
                let secure = if state.listener.tls { "; Secure" } else { "" };
                let cookie = format!("{}={token}; HttpOnly; SameSite=Strict; Path=/{secure}", auth::COOKIE);
                if let Ok(value) = HeaderValue::from_str(&cookie) {
                    response.headers_mut().insert(header::SET_COOKIE, value);
                }
            }
            response
        }
        Ok(Output::Raw(raw)) => raw_response(raw),
        Err(e) => op_error(e),
    }
}

fn raw_response(raw: RawBody) -> Response {
    let body = match raw.data {
        RawData::Bytes(bytes) => axum::body::Body::from(bytes),
        RawData::Stream(reader) => stream::reader_body(reader),
    };
    let mut builder = Response::builder().status(StatusCode::OK).header(header::CONTENT_TYPE, raw.content_type);
    if let Some(name) = raw.filename {
        builder = builder.header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", safe_filename(&name)));
    }
    builder.body(body).unwrap_or_else(|e| error_response(ErrorCode::Internal, &e.to_string()))
}

/// Printable ASCII without quotes, backslashes or slashes, so the name fits a quoted
/// Content-Disposition filename in every browser. A name with nothing left but its extension
/// becomes `export.{extension}`.
fn safe_filename(name: &str) -> String {
    let safe: String = name.chars().map(|c| if c == ' ' || (c.is_ascii_graphic() && !matches!(c, '"' | '\\' | '/')) { c } else { '_' }).collect();
    let (stem, extension) = safe.rsplit_once('.').unwrap_or((&safe, "spsession"));
    if stem.chars().any(|c| c.is_ascii_alphanumeric()) { safe.clone() } else { format!("export.{extension}") }
}

fn op_error(e: OpError) -> Response {
    match e {
        OpError::Unauthenticated => unauthenticated(),
        OpError::UnknownOperation(name) => error_response(ErrorCode::UnknownOperation, &format!("No operation is named {name}.")),
        OpError::BadRequest(m) => error_response(ErrorCode::BadRequest, &m),
        OpError::NotFound(m) => error_response(ErrorCode::NotFound, &m),
        OpError::Conflict(m) => error_response(ErrorCode::Conflict, &m),
        OpError::TooLarge => error_response(ErrorCode::TooLarge, "The upload is larger than the server accepts."),
        OpError::Internal(m) => error_response(ErrorCode::Internal, &m),
    }
}

fn sse_frame(event: &ServerEvent) -> Bytes {
    let data = serde_json::to_string(event).unwrap_or_default();
    Bytes::from(format!("id: {}\ndata: {data}\n\n", event.id))
}

/// Subscribes to the pump before reading the ring, so every event is either replayed or forwarded;
/// ids at or below the last one sent are dropped, so none arrives twice.
fn event_stream(shared: Arc<Shared>, last_id: Option<u64>) -> Response {
    let mut rx = shared.events.subscribe();
    let ctx = shared.ctx.clone();
    let (replay, mut last) = match last_id {
        Some(id) => {
            let replay = ctx.events_since(EventsSince { after: Some(id), wait: Duration::ZERO });
            let last = replay.last().map_or(id, |e| e.id);
            (replay, last)
        }
        None => (Vec::new(), newest_event(&ctx).id),
    };
    let (mut sender, body) = Channel::<Bytes, Infallible>::new(16);
    tokio::spawn(async move {
        if sender.send_data(Bytes::from_static(b":\n\n")).await.is_err() {
            return;
        }
        for event in replay {
            if sender.send_data(sse_frame(&event)).await.is_err() {
                return;
            }
        }
        loop {
            let event = match tokio::time::timeout(KEEPALIVE, rx.recv()).await {
                Err(_) => {
                    if sender.send_data(Bytes::from_static(b": keepalive\n\n")).await.is_err() {
                        return;
                    }
                    continue;
                }
                Ok(Ok(event)) => event,
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => newest_event(&ctx),
                Ok(Err(broadcast::error::RecvError::Closed)) => return,
            };
            if event.id <= last {
                continue;
            }
            last = event.id;
            if sender.send_data(sse_frame(&event)).await.is_err() {
                return;
            }
        }
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(axum::body::Body::new(body))
        .unwrap_or_else(|e| error_response(ErrorCode::Internal, &e.to_string()))
}
