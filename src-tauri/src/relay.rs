//! The relay: the page reaches a paired remote server through the shell, which holds that server's
//! token and pinned certificate. The page names a server from `servers.json` and an operation; the
//! shell makes the call and hands back the server's whole answer as one frame:
//!
//! ```text
//! [4 bytes: head length, big-endian u32][head: JSON {"status": u16, "headers": [[name, value], ...]}][body: raw bytes]
//! ```
//!
//! The token never reaches the page, and a name that isn't in `servers.json` reaches nothing.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use session_player_client::{Client, PairedServer, RawAnswer, RawCall};
use session_player_core::ops::{RawData, ServerEvent};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseHead {
    pub status: u16,
    /// Lowercase names, in the order received.
    pub headers: Vec<(String, String)>,
}

/// The paired server named exactly `name`.
pub fn resolve_server(name: &str, paired: &[PairedServer]) -> Result<PairedServer, String> {
    paired
        .iter()
        .find(|s| s.name == name)
        .cloned()
        .ok_or_else(|| format!("No paired server is named {name:?}. Pair with it first (sp connect, or Connect in the app)."))
}

/// An operation name is 1 to 64 characters of a-z and `_`, so it can't change the request path.
pub fn validate_op(op: &str) -> Result<(), String> {
    if (1..=64).contains(&op.len()) && op.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
        Ok(())
    } else {
        Err(format!("{op:?} isn't an operation name: 1 to 64 characters of a-z and _."))
    }
}

pub fn frame_response(head: &ResponseHead, body: &[u8]) -> Vec<u8> {
    let head = serde_json::to_vec(head).expect("a response head serializes");
    let mut framed = Vec::with_capacity(4 + head.len() + body.len());
    framed.extend_from_slice(&(head.len() as u32).to_be_bytes());
    framed.extend_from_slice(&head);
    framed.extend_from_slice(body);
    framed
}

/// The head and body of a frame. A damaged frame is an error.
pub fn parse_framed(bytes: &[u8]) -> Result<(ResponseHead, &[u8]), String> {
    let damaged = |why: &str| format!("The relayed response is damaged: {why}");
    let length: [u8; 4] = bytes.get(..4).and_then(|b| b.try_into().ok()).ok_or_else(|| damaged("no head length"))?;
    let end = 4usize.checked_add(u32::from_be_bytes(length) as usize).ok_or_else(|| damaged("the head length overflows"))?;
    let head = bytes.get(4..end).ok_or_else(|| damaged("the head is cut short"))?;
    let head: ResponseHead = serde_json::from_slice(head).map_err(|e| damaged(&e.to_string()))?;
    Ok((head, &bytes[end..]))
}

/// The headers that may cross the relay: never credentials (Authorization, Cookie, Set-Cookie) or
/// a Host, which belong to one side's own connection.
pub fn relay_headers(incoming: &[(String, String)]) -> Vec<(String, String)> {
    const DROPPED: [&str; 4] = ["authorization", "cookie", "set-cookie", "host"];
    incoming.iter().filter(|(name, _)| !DROPPED.iter().any(|d| name.eq_ignore_ascii_case(d))).cloned().collect()
}

/// A URL-encoded query string (`a=1&b=x%20y`, `+` is a space) as name-value pairs.
pub fn parse_query(query: &str) -> Result<Vec<(String, String)>, String> {
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (name, value) = part.split_once('=').unwrap_or((part, ""));
            Ok((percent_decode(&name.replace('+', " "))?, percent_decode(&value.replace('+', " "))?))
        })
        .collect()
}

/// Decodes `%XX` escapes. Not `+`: a header value the page encoded with encodeURIComponent.
pub fn percent_decode(text: &str) -> Result<String, String> {
    let bad = || format!("{text:?} has a broken %-escape");
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).ok_or_else(bad)?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| bad())?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| format!("{text:?} isn't UTF-8 once decoded"))
}

/// What the page's `relay_request` headers say. `server` arrives percent-encoded (header values are
/// ASCII), `query` URL-encoded and still encoded here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestHeaders {
    pub server: String,
    pub op: String,
    pub query: String,
    pub content_type: String,
}

/// Reads the `server`, `op`, `query` and `content-type` headers (names in any case). `query` may be
/// missing; `content-type` defaults to JSON.
pub fn request_headers(headers: &[(String, String)]) -> Result<RequestHeaders, String> {
    let get = |name: &str| headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone());
    let server = percent_decode(&get("server").ok_or("The relay request has no server header")?)?;
    let op = get("op").ok_or("The relay request has no op header")?;
    let query = get("query").unwrap_or_default();
    let content_type = get("content-type").unwrap_or_else(|| "application/json".to_string());
    Ok(RequestHeaders { server, op, query, content_type })
}

/// One Client per paired server, by name, so relayed calls reuse its kept-alive connections. An
/// entry is rebuilt when `servers.json` holds different details for that name (`sp connect` in a
/// terminal re-paired it); pairing or removing a server in the app forgets it at once.
#[derive(Default)]
pub struct Clients {
    by_name: Mutex<HashMap<String, (PairedServer, Arc<Client>)>>,
}

impl Clients {
    /// The Client for `server`, made on first use.
    pub fn for_server(&self, server: &PairedServer) -> Result<Arc<Client>, String> {
        let mut by_name = self.by_name.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((saved, client)) = by_name.get(&server.name)
            && saved == server
        {
            return Ok(client.clone());
        }
        let client = Arc::new(Client::paired(server).map_err(|e| format!("{e:#}"))?);
        by_name.insert(server.name.clone(), (server.clone(), client.clone()));
        Ok(client)
    }

    /// Drops the Client kept for `name`, and with it its connections.
    pub fn forget(&self, name: &str) {
        self.by_name.lock().unwrap_or_else(PoisonError::into_inner).remove(name);
    }

    /// The Client kept for `name`, if any.
    pub fn cached(&self, name: &str) -> Option<Arc<Client>> {
        self.by_name.lock().unwrap_or_else(PoisonError::into_inner).get(name).map(|(_, client)| client.clone())
    }
}

/// The open relayed event streams, each with the flag that ends it.
pub struct Streams {
    open: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    next: AtomicU64,
}

impl Default for Streams {
    fn default() -> Streams {
        Streams { open: Mutex::new(HashMap::new()), next: AtomicU64::new(1) }
    }
}

impl Streams {
    /// A new stream's id and its stop flag.
    pub fn open(&self) -> (u64, Arc<AtomicBool>) {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let stop = Arc::new(AtomicBool::new(false));
        self.table().insert(id, stop.clone());
        (id, stop)
    }

    /// Ends the stream `id`, if it is still open.
    pub fn close(&self, id: u64) {
        if let Some(stop) = self.table().remove(&id) {
            stop.store(true, Ordering::SeqCst);
        }
    }

    /// Ends every open stream. The shell calls this when the page reloads, since a reloaded page
    /// can no longer close the streams the old one opened.
    pub fn close_all(&self) {
        for (_, stop) in self.table().drain() {
            stop.store(true, Ordering::SeqCst);
        }
    }

    /// Forgets `id` once its thread has returned.
    pub fn ended(&self, id: u64) {
        self.table().remove(&id);
    }

    pub fn len(&self) -> usize {
        self.table().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn table(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Arc<AtomicBool>>> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub struct RelayCall<'a> {
    pub clients: &'a Clients,
    /// What `servers.json` lists.
    pub paired: &'a [PairedServer],
    pub server: &'a str,
    pub op: &'a str,
    /// URL-encoded, as the page sent it; empty for none.
    pub query: &'a str,
    pub content_type: &'a str,
    /// Sent unchanged, JSON or an upload.
    pub body: Vec<u8>,
}

/// `POST /api/v1/{op}` on the named server. Any status comes back as a frame; only a refused
/// request (unknown server, bad op name, damaged query) or a failure to reach the server is an Err,
/// and those fail before any network I/O except the last.
pub fn relay_call(call: RelayCall) -> Result<Vec<u8>, String> {
    let server = resolve_server(call.server, call.paired)?;
    validate_op(call.op)?;
    let query = parse_query(call.query)?;
    let client = call.clients.for_server(&server)?;
    let raw = RawCall { name: call.op.to_string(), body: RawData::Bytes(call.body), content_type: call.content_type.to_string(), query };
    let answer = client.call_raw_answer(raw).map_err(|e| e.message)?;
    Ok(frame_answer(answer))
}

pub struct RelayImage<'a> {
    pub clients: &'a Clients,
    pub paired: &'a [PairedServer],
    pub server: &'a str,
    /// `/api/v1/image/{generation}/{event}/{image}`, as `load_session` returned it.
    pub path: &'a str,
}

/// `GET` of an image URL on the named server, framed like `relay_call`. Only the image route.
pub fn relay_image(image: RelayImage) -> Result<Vec<u8>, String> {
    let server = resolve_server(image.server, image.paired)?;
    let ids: Vec<&str> = image.path.strip_prefix("/api/v1/image/").map(|rest| rest.split('/').collect()).unwrap_or_default();
    if ids.len() != 3 || !ids.iter().all(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())) {
        return Err(format!("{:?} isn't an image URL from load_session.", image.path));
    }
    let client = image.clients.for_server(&server)?;
    let answer = client.get(image.path).map_err(|e| e.message)?;
    Ok(frame_answer(answer))
}

fn frame_answer(answer: RawAnswer) -> Vec<u8> {
    frame_response(&ResponseHead { status: answer.status, headers: relay_headers(&answer.headers) }, &answer.body)
}

pub struct StreamParams<'a> {
    pub server: PairedServer,
    /// The last event id the page has seen; None starts from now.
    pub last_id: Option<u64>,
    /// Set to end the stream.
    pub stop: &'a AtomicBool,
    /// How long to wait before reconnecting after the stream breaks.
    pub retry: Duration,
    /// Returns false when the page is gone, which ends the stream.
    pub on_event: &'a mut dyn FnMut(ServerEvent) -> bool,
}

/// Forwards the server's events until `stop` is set or `on_event` returns false. When the stream
/// breaks (the server restarted, the network dropped), reconnects after `retry` from the last id
/// it forwarded, so the page sees no gap: the server answers a too-old id with a Snapshot.
/// How often a stream waiting for events checks whether it should stop.
const POLL: Duration = Duration::from_millis(250);

pub fn run_stream(params: StreamParams) {
    let StreamParams { server, mut last_id, stop, retry, on_event } = params;
    let stopped = || stop.load(Ordering::SeqCst);
    while !stopped() {
        // A new Client per connection: it is cheap, and a broken one holds nothing worth keeping.
        if let Some(mut events) = Client::paired(&server).ok().and_then(|client| client.events(last_id).ok()) {
            while !stopped() {
                let asked = Instant::now();
                match events.next_within(POLL) {
                    Some(Ok(event)) => {
                        let id = event.id;
                        if !on_event(event) {
                            return;
                        }
                        last_id = Some(id);
                    }
                    Some(Err(_)) => break,
                    // A wait that ran out takes the whole POLL; an ended stream answers None at once.
                    None if asked.elapsed() < POLL / 2 => break,
                    None => {}
                }
            }
        }
        let resume = Instant::now() + retry;
        while !stopped() && Instant::now() < resume {
            std::thread::sleep(Duration::from_millis(20).min(retry));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use session_player_core::ops::DeviceId;

    fn server(name: &str) -> PairedServer {
        PairedServer { name: name.to_string(), address: "10.0.0.2".to_string(), port: 47801, fingerprint: Some("ab".repeat(32)), token: "secret".to_string(), device: DeviceId("d1".to_string()) }
    }

    fn pair(key: &str, value: &str) -> (String, String) {
        (key.to_string(), value.to_string())
    }

    #[test]
    fn resolve_server_returns_the_exact_name_match_and_points_at_sp_connect_for_an_unknown_name() {
        let paired = [server("work"), server("Work laptop")];
        assert_eq!(resolve_server("Work laptop", &paired).unwrap().name, "Work laptop");
        assert_eq!(resolve_server("work", &paired).unwrap().name, "work");
        for unknown in ["WORK", "work ", "wor", ""] {
            let e = resolve_server(unknown, &paired).unwrap_err();
            assert!(e.contains("sp connect"), "{unknown:?}: {e}");
        }
        assert!(resolve_server("work", &[]).is_err());
    }

    #[test]
    fn validate_op_accepts_1_to_64_characters_of_lowercase_letters_and_underscores_only() {
        for good in ["hello", "list_sessions", "a", &"a".repeat(64)] {
            assert!(validate_op(good).is_ok(), "{good}");
        }
        for bad in ["", &"a".repeat(65), "Hello", "list-sessions", "../hello", "a/b", "op1", "hello?x=1", "hé"] {
            assert!(validate_op(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_frame_round_trips_its_head_and_raw_body() {
        let head = ResponseHead { status: 422, headers: vec![pair("content-type", "application/json"), pair("x-two", "a"), pair("x-two", "b")] };
        let body = b"\x00\xff{\"code\":\"bad_request\"}";
        let framed = frame_response(&head, body);
        let (parsed, rest) = parse_framed(&framed).unwrap();
        assert_eq!(parsed, head);
        assert_eq!(rest, body);

        let empty = frame_response(&ResponseHead { status: 204, headers: Vec::new() }, b"");
        let (parsed, rest) = parse_framed(&empty).unwrap();
        assert_eq!(parsed.status, 204);
        assert!(rest.is_empty());
    }

    #[test]
    fn parse_framed_returns_an_error_instead_of_panicking_on_a_damaged_frame() {
        let good = frame_response(&ResponseHead { status: 200, headers: Vec::new() }, b"body");
        let mut huge = good.clone();
        huge[..4].copy_from_slice(&u32::MAX.to_be_bytes());
        let mut not_json = good.clone();
        not_json[4] = b'x';
        let damaged: Vec<&[u8]> = vec![b"", b"\x00", b"\x00\x00\x00", &good[..6], &huge, &not_json, b"\x00\x00\x00\x02{}"];
        for bytes in damaged {
            assert!(parse_framed(bytes).is_err(), "{bytes:?}");
        }
    }

    #[test]
    fn relay_headers_drops_credentials_and_host_and_keeps_the_rest() {
        let incoming = vec![
            pair("Authorization", "Bearer stolen"),
            pair("COOKIE", "a=b"),
            pair("set-cookie", "sp_session=x"),
            pair("Host", "evil.example"),
            pair("authorization", "Basic abc"),
            pair("Content-Type", "application/json"),
            pair("Last-Event-ID", "42"),
            pair("etag", "\"1\""),
        ];
        assert_eq!(relay_headers(&incoming), vec![pair("Content-Type", "application/json"), pair("Last-Event-ID", "42"), pair("etag", "\"1\"")]);
    }

    #[test]
    fn parse_query_decodes_url_encoded_pairs() {
        assert_eq!(parse_query("").unwrap(), Vec::<(String, String)>::new());
        assert_eq!(parse_query("a=1&b=x%20y+z&c=&d").unwrap(), vec![pair("a", "1"), pair("b", "x y z"), pair("c", ""), pair("d", "")]);
        assert_eq!(parse_query("name=caf%C3%A9").unwrap(), vec![pair("name", "café")]);
        for bad in ["a=%", "a=%zz", "a=%ff"] {
            assert!(parse_query(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn percent_decode_reads_what_encode_uri_component_wrote() {
        assert_eq!(percent_decode("Dan%E2%80%99s%20Mac").unwrap(), "Dan’s Mac");
        assert_eq!(percent_decode("a+b").unwrap(), "a+b");
        assert_eq!(percent_decode("plain").unwrap(), "plain");
        assert!(percent_decode("%4").is_err());
    }

    #[test]
    fn request_headers_decodes_the_server_name_and_defaults_the_content_type() {
        let headers = [pair("Server", "Dan%E2%80%99s%20Mac"), pair("op", "import_bundle"), pair("query", "on_conflict=replace"), pair("Content-Type", "application/octet-stream")];
        assert_eq!(
            request_headers(&headers).unwrap(),
            RequestHeaders { server: "Dan’s Mac".to_string(), op: "import_bundle".to_string(), query: "on_conflict=replace".to_string(), content_type: "application/octet-stream".to_string() }
        );
        let minimal = request_headers(&[pair("server", "work"), pair("op", "hello")]).unwrap();
        assert_eq!((minimal.query.as_str(), minimal.content_type.as_str()), ("", "application/json"));
        assert!(request_headers(&[pair("op", "hello")]).unwrap_err().contains("server"));
        assert!(request_headers(&[pair("server", "work")]).unwrap_err().contains("op"));
        assert!(request_headers(&[pair("server", "%zz"), pair("op", "hello")]).is_err());
    }

    #[test]
    fn relay_image_refuses_any_path_but_the_image_route_before_any_network_io() {
        // Port 9 on a TEST-NET address: if a request were sent it would fail as unreachable instead.
        let paired = [PairedServer { address: "192.0.2.1".to_string(), port: 9, ..server("work") }];
        for path in ["/api/v1/hello", "/api/v1/image/1/2", "/api/v1/image/1/2/x", "/api/v1/image/1/2/3/4", "/api/v1/image/../hello", "/api/v1/image/1/2/3?x=1", "http://evil/api/v1/image/1/2/3"] {
            let e = relay_image(RelayImage { clients: &Clients::default(), paired: &paired, server: "work", path }).unwrap_err();
            assert!(!e.contains("Couldn't reach"), "{path}: {e}");
        }
    }

    #[test]
    fn clients_keeps_one_client_per_server_name_until_it_is_forgotten() {
        let clients = Clients::default();
        let work = server("work");

        let first = clients.for_server(&work).unwrap();
        let again = clients.for_server(&work).unwrap();
        let home = clients.for_server(&server("home")).unwrap();

        assert!(Arc::ptr_eq(&first, &again), "the same server reuses its Client");
        assert!(!Arc::ptr_eq(&first, &home), "another server has its own");
        assert!(Arc::ptr_eq(&clients.cached("work").unwrap(), &first));
        clients.forget("work");
        assert!(clients.cached("work").is_none());
        assert!(clients.cached("home").is_some(), "forgetting one server keeps the others");
        let rebuilt = clients.for_server(&work).unwrap();
        assert!(!Arc::ptr_eq(&first, &rebuilt), "a forgotten server gets a new Client");
        clients.forget("nobody");
    }

    #[test]
    fn clients_rebuilds_a_servers_client_when_its_saved_details_change() {
        let clients = Clients::default();
        let first = clients.for_server(&server("work")).unwrap();

        let repaired = PairedServer { token: "new secret".to_string(), ..server("work") };
        let second = clients.for_server(&repaired).unwrap();
        let moved = PairedServer { port: 47802, ..repaired.clone() };
        let third = clients.for_server(&moved).unwrap();

        assert!(!Arc::ptr_eq(&first, &second), "a new token gets a new Client");
        assert!(!Arc::ptr_eq(&second, &third), "a new port gets a new Client");
        assert!(Arc::ptr_eq(&clients.for_server(&moved).unwrap(), &third));
    }

    #[test]
    fn streams_close_sets_the_streams_stop_flag_and_forgets_it() {
        let streams = Streams::default();
        let (a, stop_a) = streams.open();
        let (b, stop_b) = streams.open();
        assert_ne!(a, b);
        assert_eq!(streams.len(), 2);

        streams.close(a);
        assert!(stop_a.load(Ordering::SeqCst));
        assert!(!stop_b.load(Ordering::SeqCst));
        assert_eq!(streams.len(), 1);
        streams.close(a);
        streams.ended(b);
        assert!(streams.is_empty());
        assert!(!stop_b.load(Ordering::SeqCst), "a stream whose thread returned isn't stopped again");
    }

    #[test]
    fn streams_close_all_stops_every_open_stream_and_empties_the_table() {
        let streams = Streams::default();
        let flags: Vec<Arc<AtomicBool>> = (0..3).map(|_| streams.open().1).collect();

        streams.close_all();

        assert!(streams.is_empty());
        assert!(flags.iter().all(|f| f.load(Ordering::SeqCst)));
        let (_, fresh) = streams.open();
        assert!(!fresh.load(Ordering::SeqCst), "a stream the reloaded page opens runs");
        assert_eq!(streams.len(), 1);
    }
}
