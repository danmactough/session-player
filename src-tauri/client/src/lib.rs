//! Calls the Session Player server over HTTP, with certificate pinning for a remote one.
//! See .plans/server-api/ for the design.

mod pairing;
mod probe;
mod pull;
mod store;

use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::Serialize;
use serde::de::DeserializeOwned;
use session_player_core::ops::{API_MINOR, API_VERSION, ApiError, ErrorCode, HelloResponse, Op, RawBody, RawData, ServerEvent};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::runtime::Runtime;

pub use pairing::{ConnectParams, PairedServer, PairingString, connect};
pub use probe::{LocalProbe, probe_local};
pub use pull::{PULL, PullChange, PullCounts, PullFile, PullRequest, PullResponse};
pub use store::ServersStore;

pub struct ClientParams {
    /// `http://127.0.0.1:{port}` for the local server, `https://{host}:{port}` for a paired one.
    pub url: String,
    pub token: Option<String>,
    /// Lowercase hex SHA-256 of the server's certificate. Only this certificate is accepted, and
    /// the handshake signature is still verified.
    pub fingerprint: Option<String>,
}

/// A synchronous client. Its tokio runtime is shared (`Arc`) with every body and event stream it
/// returns, so those keep working after the Client is dropped.
pub struct Client {
    runtime: Arc<Runtime>,
    /// As the Host header carries it.
    host: String,
    port: u16,
    token: Option<String>,
    tls: Option<Arc<rustls::ClientConfig>>,
}

/// An operation with a raw body, by name.
pub struct RawCall {
    pub name: String,
    pub body: RawData,
    pub content_type: String,
    /// Sent as the URL query; a raw operation's options.
    pub query: Vec<(String, String)>,
}

/// A response whatever its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawAnswer {
    pub status: u16,
    /// Lowercase names, in the order received.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RawAnswer {
    /// The first header named `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

impl Client {
    pub fn new(params: ClientParams) -> anyhow::Result<Client> {
        let (tls, rest) = if let Some(rest) = params.url.strip_prefix("https://") {
            (true, rest)
        } else if let Some(rest) = params.url.strip_prefix("http://") {
            (false, rest)
        } else {
            bail!("{} isn't an http:// or https:// address", params.url);
        };
        let (host, port) = rest.trim_end_matches('/').rsplit_once(':').with_context(|| format!("{} has no port", params.url))?;
        let port: u16 = port.parse().with_context(|| format!("{} has no port", params.url))?;
        let tls = match (tls, params.fingerprint) {
            (true, Some(fingerprint)) => Some(Arc::new(pinned_config(fingerprint.to_ascii_lowercase())?)),
            (true, None) => bail!("A TLS server needs a pinned certificate fingerprint; pair with it again"),
            (false, _) => None,
        };
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).thread_name("sp-client").enable_all().build()?;
        Ok(Client { runtime: Arc::new(runtime), host: host.to_string(), port, token: params.token, tls })
    }

    /// The local server's loopback listener.
    pub fn local(port: u16, token: &str) -> anyhow::Result<Client> {
        Client::new(ClientParams { url: format!("http://127.0.0.1:{port}"), token: Some(token.to_string()), fingerprint: None })
    }

    pub fn paired(server: &PairedServer) -> anyhow::Result<Client> {
        let scheme = if server.fingerprint.is_some() { "https" } else { "http" };
        Client::new(ClientParams { url: format!("{scheme}://{}:{}", server.address, server.port), token: Some(server.token.clone()), fingerprint: server.fingerprint.clone() })
    }

    pub fn call<Req: Serialize, Res: DeserializeOwned>(&self, op: &Op<Req, Res>, req: &Req) -> Result<Res, ApiError> {
        self.call_named(op.spec.name, req)
    }

    /// A JSON operation by name: the server crate's extensions, which have no typed `Op`.
    pub fn call_named<Req: Serialize, Res: DeserializeOwned>(&self, name: &str, req: &Req) -> Result<Res, ApiError> {
        let body = serde_json::to_vec(req).map_err(|e| failure(ErrorCode::BadRequest, e))?;
        let path = format!("/api/v1/{name}");
        let answer = self.exchange(Exchange { method: Method::POST, path: &path, content_type: Some("application/json"), last_event_id: None, body })?;
        if !(200..300).contains(&answer.status) {
            return Err(api_error(answer.status, &answer.body));
        }
        serde_json::from_slice(&answer.body).map_err(|e| failure(ErrorCode::Internal, format!("The server's answer to {name} didn't parse: {e}")))
    }

    /// A non-2xx answer is the server's ApiError.
    pub fn call_raw(&self, call: RawCall) -> Result<RawBody, ApiError> {
        let (path, body) = raw_request(call)?;
        let response = self.runtime.block_on(self.send(Exchange { method: Method::POST, path: &path.0, content_type: Some(&path.1), last_event_id: None, body }))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let body = self.runtime.block_on(collect(response.into_body()))?;
            return Err(api_error(status, &body));
        }
        let header = |name| response.headers().get(name).and_then(|v: &hyper::header::HeaderValue| v.to_str().ok()).map(str::to_string);
        let content_type = header(hyper::header::CONTENT_TYPE).unwrap_or_else(|| "application/octet-stream".to_string());
        let filename = header(hyper::header::CONTENT_DISPOSITION).and_then(|d| {
            let start = d.find("filename=\"")? + "filename=\"".len();
            d[start..].split('"').next().map(str::to_string)
        });
        let reader = IncomingReader { runtime: self.runtime.clone(), body: response.into_body(), pending: Bytes::new() };
        Ok(RawBody { data: RawData::Stream(Box::new(reader)), content_type, filename })
    }

    /// Any status comes back as a RawAnswer; only a failure to reach the server is an error.
    pub fn call_raw_answer(&self, call: RawCall) -> Result<RawAnswer, ApiError> {
        let (path, body) = raw_request(call)?;
        self.exchange(Exchange { method: Method::POST, path: &path.0, content_type: Some(&path.1), last_event_id: None, body })
    }

    /// `GET {path}` with the token, such as an image URL from `load_session`.
    pub fn get(&self, path: &str) -> Result<RawAnswer, ApiError> {
        self.exchange(Exchange { method: Method::GET, path, content_type: None, last_event_id: None, body: Vec::new() })
    }

    /// `GET /api/v1/events`, resuming after `last_id` (sent as `Last-Event-ID`).
    pub fn events(&self, last_id: Option<u64>) -> Result<EventStream, ApiError> {
        let exchange = Exchange { method: Method::GET, path: "/api/v1/events", content_type: None, last_event_id: last_id, body: Vec::new() };
        let response = self.runtime.block_on(self.send(exchange))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let body = self.runtime.block_on(collect(response.into_body()))?;
            return Err(api_error(status, &body));
        }
        Ok(EventStream { runtime: self.runtime.clone(), body: response.into_body(), buf: Vec::new(), ended: false })
    }

    fn exchange(&self, exchange: Exchange) -> Result<RawAnswer, ApiError> {
        self.runtime.block_on(async {
            let response = self.send(exchange).await?;
            let status = response.status().as_u16();
            let headers = response.headers().iter().map(|(n, v)| (n.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())).collect();
            let body = collect(response.into_body()).await?;
            Ok(RawAnswer { status, headers, body })
        })
    }

    /// One request on its own connection.
    async fn send(&self, exchange: Exchange<'_>) -> Result<Response<Incoming>, ApiError> {
        let unreachable = |e: &dyn std::fmt::Display| failure(ErrorCode::Internal, format!("Couldn't reach {}:{}: {e}", self.host, self.port));
        let mut request = Request::builder().method(exchange.method).uri(exchange.path).header(hyper::header::HOST, format!("{}:{}", self.host, self.port));
        if let Some(token) = &self.token {
            request = request.header(hyper::header::AUTHORIZATION, format!("Bearer {token}"));
        }
        if let Some(content_type) = exchange.content_type {
            request = request.header(hyper::header::CONTENT_TYPE, content_type);
        }
        if let Some(id) = exchange.last_event_id {
            request = request.header("last-event-id", id.to_string());
        }
        let request = request.body(Full::new(Bytes::from(exchange.body))).map_err(|e| failure(ErrorCode::BadRequest, e))?;
        let connect_host = self.host.trim_start_matches('[').trim_end_matches(']').to_string();
        let tcp = tokio::net::TcpStream::connect((connect_host.as_str(), self.port)).await.map_err(|e| unreachable(&e))?;
        match &self.tls {
            None => over(tcp, request).await.map_err(|e| unreachable(&e)),
            Some(config) => {
                let name = ServerName::try_from(connect_host).map_err(|e| unreachable(&e))?;
                let tls = tokio_rustls::TlsConnector::from(config.clone()).connect(name, tcp).await.map_err(|e| unreachable(&e))?;
                over(tls, request).await.map_err(|e| unreachable(&e))
            }
        }
    }
}

struct Exchange<'a> {
    method: Method,
    path: &'a str,
    content_type: Option<&'a str>,
    last_event_id: Option<u64>,
    body: Vec<u8>,
}

async fn over<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(io: S, request: Request<Full<Bytes>>) -> Result<Response<Incoming>, hyper::Error> {
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(io)).await?;
    tokio::spawn(connection);
    sender.send_request(request).await
}

async fn collect(body: Incoming) -> Result<Vec<u8>, ApiError> {
    body.collect().await.map(|c| c.to_bytes().to_vec()).map_err(|e| failure(ErrorCode::Internal, format!("The server's answer broke off: {e}")))
}

fn failure(code: ErrorCode, message: impl std::fmt::Display) -> ApiError {
    ApiError { code, message: message.to_string() }
}

fn api_error(status: u16, body: &[u8]) -> ApiError {
    serde_json::from_slice(body).unwrap_or_else(|_| failure(ErrorCode::Internal, format!("The server answered {}", StatusCode::from_u16(status).map_or(status.to_string(), |s| s.to_string()))))
}

/// `((path with query, content type), body)` for a raw call.
fn raw_request(call: RawCall) -> Result<((String, String), Vec<u8>), ApiError> {
    let body = match call.body {
        RawData::Bytes(bytes) => bytes,
        RawData::Stream(mut reader) => {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).map_err(|e| failure(ErrorCode::BadRequest, format!("Couldn't read the upload: {e}")))?;
            bytes
        }
    };
    let mut path = format!("/api/v1/{}", call.name);
    for (i, (name, value)) in call.query.iter().enumerate() {
        path.push(if i == 0 { '?' } else { '&' });
        path.push_str(&percent_encode(name));
        path.push('=');
        path.push_str(&percent_encode(value));
    }
    Ok(((path, call.content_type), body))
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// A response body read from a blocking caller.
struct IncomingReader {
    runtime: Arc<Runtime>,
    body: Incoming,
    pending: Bytes,
}

impl Read for IncomingReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.pending.is_empty() {
            match self.runtime.block_on(self.body.frame()) {
                None => return Ok(0),
                Some(Err(e)) => return Err(std::io::Error::other(e)),
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        self.pending = data;
                    }
                }
            }
        }
        let n = buf.len().min(self.pending.len());
        buf[..n].copy_from_slice(&self.pending.split_to(n));
        Ok(n)
    }
}

/// Accepts exactly the certificate with this SHA-256 fingerprint, and still checks that the server
/// holds its key.
#[derive(Debug)]
struct PinVerifier {
    fingerprint: String,
    provider: Arc<CryptoProvider>,
}

fn pinned_config(fingerprint: String) -> anyhow::Result<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = PinVerifier { fingerprint, provider: provider.clone() };
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth())
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let digest = ring::digest::digest(&ring::digest::SHA256, end_entity.as_ref());
        let actual: String = digest.as_ref().iter().map(|b| format!("{b:02x}")).collect();
        if actual == self.fingerprint {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure))
        }
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// The server's events in order. Iterating blocks until the next event.
pub struct EventStream {
    runtime: Arc<Runtime>,
    body: Incoming,
    buf: Vec<u8>,
    ended: bool,
}

impl EventStream {
    /// The next event, or None when none arrives within `wait`.
    pub fn next_within(&mut self, wait: Duration) -> Option<Result<ServerEvent, ApiError>> {
        let deadline = Instant::now() + wait;
        loop {
            while let Some(end) = self.buf.windows(2).position(|w| w == b"\n\n") {
                let block: Vec<u8> = self.buf.drain(..end + 2).collect();
                let text = String::from_utf8_lossy(&block);
                let data: Vec<&str> = text.lines().filter_map(|l| l.strip_prefix("data:")).map(|d| d.strip_prefix(' ').unwrap_or(d)).collect();
                // A block without data is a comment, such as a keepalive.
                if !data.is_empty() {
                    return Some(serde_json::from_str(&data.join("\n")).map_err(|e| failure(ErrorCode::Internal, format!("An event didn't parse: {e}"))));
                }
            }
            if self.ended {
                return None;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            match self.runtime.block_on(async { tokio::time::timeout(left, self.body.frame()).await }) {
                Err(_) => return None,
                Ok(None) => self.ended = true,
                Ok(Some(Err(e))) => {
                    self.ended = true;
                    return Some(Err(failure(ErrorCode::Internal, format!("The event stream broke off: {e}"))));
                }
                Ok(Some(Ok(frame))) => {
                    if let Ok(data) = frame.into_data() {
                        self.buf.extend_from_slice(&data);
                    }
                }
            }
        }
    }
}

impl Iterator for EventStream {
    type Item = Result<ServerEvent, ApiError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(event) = self.next_within(Duration::from_secs(3600)) {
                return Some(event);
            }
            if self.ended {
                return None;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compat {
    Exact,
    /// Same `api_version`, higher `api_minor`: the server has operations this client doesn't use.
    NewerServer,
    /// Same `api_version`, lower `api_minor`: some operations this client knows may be missing.
    OlderServer,
}

/// Err (saying which side to upgrade and naming the server's version) when `api_version` differs.
pub fn check_compat(hello: &HelloResponse) -> Result<Compat, String> {
    let server = &hello.server_version;
    if hello.api_version < API_VERSION {
        return Err(format!(
            "The server runs Session Player {server} (API {}), older than this client's API {API_VERSION}. Upgrade the server.",
            hello.api_version
        ));
    }
    if hello.api_version > API_VERSION {
        return Err(format!(
            "The server runs Session Player {server} (API {}), newer than this client's API {API_VERSION}. Upgrade sp or the app on this machine.",
            hello.api_version
        ));
    }
    Ok(match hello.api_minor.cmp(&API_MINOR) {
        std::cmp::Ordering::Equal => Compat::Exact,
        std::cmp::Ordering::Greater => Compat::NewerServer,
        std::cmp::Ordering::Less => Compat::OlderServer,
    })
}
