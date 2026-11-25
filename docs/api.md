# HTTP API

`sp serve` runs the Session Player server. The desktop app, the web UI in a browser and `sp`
itself all reach the same operations through it. This is the reference for that API, version
`1.0` (`API_VERSION` 1, `API_MINOR` 0 in `src-tauri/core/src/ops/wire.rs`).

Every operation is `POST /api/v1/<op>` with a JSON body and a JSON answer. There is no REST
layer. Besides the operations there are three other routes: the event stream, images, and the
web UI's own files.

| Route | Method | Purpose |
|---|---|---|
| `/api/v1/<op>` | POST | Run an operation |
| `/api/v1/events` | GET | Server-sent events: job progress and index passes |
| `/api/v1/image/{generation}/{event}/{image}` | GET, HEAD | An image from a loaded session |
| anything not under `/api/` | GET | The web UI (no token needed) |

## Listeners and ports

The server opens up to two listeners:

| Listener | Address | Default port | Protocol |
|---|---|---|---|
| Loopback | `127.0.0.1` | 47800 | plain HTTP |
| External | `--bind` | 47801 | HTTPS (or plain HTTP, see [TLS](#tls)) |

- The loopback listener is always open.
- The external listener opens only when the server was started with `--bind` **and** either a device
  is paired or a pairing offer is open. It closes about a second after neither is true. Without
  `--bind` the server never listens off loopback.
- The two never share a port: a wildcard bind on port P conflicts with `127.0.0.1:P`.
- `hello` reports `loopback_only: true` unless the external listener is open right now.

## Authentication

Every request under `/api/` needs a credential except `POST /api/v1/connect`, which redeems a
pairing secret. A request without one, or with a wrong one, gets the same 401 body whether the
operation exists or not:

```json
{ "code": "unauthenticated", "message": "A valid token is required." }
```

There are three kinds of credential.

### Local token (`server.token`)

```
Authorization: Bearer <contents of server.token>
```

- The server writes a new random token to `server.token` in the data folder (mode 0600) every
  time it starts. Anything that can read that file is the user, so it gets full access.
- It is accepted on the **loopback listener only**. Sent to the external listener it is refused
  like any wrong token.
- Requests with it run as device `"local"`.

### Device tokens

```
Authorization: Bearer <device token>
```

- Each paired device gets its own random token from `connect`. The server keeps only its
  SHA-256 hash, in `devices.json`.
- Accepted on both listeners. Every device has full access except to the
  [local-only operations](#server-extensions); there are no roles.
- `unpair` revokes a device. Its token stops working on the next request, and its open event
  streams end within a second.

### Browser cookie

A browser can't keep a bearer token away from page scripts, so it pairs with `"browser": true`
and gets its device token as a cookie instead:

```
Set-Cookie: sp_session=<device token>; HttpOnly; SameSite=Strict; Path=/      (+ "; Secure" over TLS)
```

- The `connect` answer then has `"token": null`.
- A cookie-authenticated request other than GET must also carry an `x-sp-csrf` header (any value)
  **and** an `Origin` equal to the listener's own origin. Otherwise it gets 403
  `forbidden_origin`. A bearer token needs neither.

### Host and Origin checks

These run on every request, before authentication, including requests for unknown paths and the
web UI's files. A refused request learns nothing about what exists.

1. **Host.** The loopback listener answers `Host: 127.0.0.1:{port}` or `localhost:{port}`. The
   external listener answers `{advertise}:{port}`. HTTP/2 requests are checked by `:authority`.
   Anything else gets 403 `forbidden_origin`. This blocks DNS rebinding.
2. **Origin.** When present, it must be one of:
   - the listener's own origin (`http://127.0.0.1:{port}`, `http://localhost:{port}`, or
     `{scheme}://{advertise}:{port}`)
   - `tauri://localhost` or `http://tauri.localhost` (the desktop app's webview)

   Anything else gets 403.

CORS: an allowed Origin is echoed in `Access-Control-Allow-Origin` with `Vary: Origin`. The
server never answers `*` and never sends `Access-Control-Allow-Credentials`. `OPTIONS` (preflight)
answers 204 with `Access-Control-Allow-Methods: GET, POST, OPTIONS`, `Access-Control-Allow-Headers:
authorization, content-type, last-event-id, x-sp-csrf` and `Access-Control-Max-Age: 600`.

## Operations

```
POST /api/v1/<op>
Content-Type: application/json

{ ...request... }
```

- The body is required JSON, even for operations that take nothing (send `{}`). Fields marked
  optional below can be left out. The JSON body limit is 16 MiB.
- A successful answer is 200 with the response as JSON. An operation that returns nothing answers
  `null`.
- Field names on the wire are `snake_case`, as in the Rust structs.
- Any method other than POST on an operation's path gets 405 `method_not_allowed`.
- `hello`, `job` and `cancel_job` answer on the server's request threads. Every other operation
  runs on a blocking thread, so a slow one doesn't hold up the others.

The table in `src-tauri/core/src/ops/mod.rs` (`operations!`) is the source of truth for the core
operations. The request structs are in `src-tauri/core/src/ops/*.rs`; response types not spelled
out here are in `src-tauri/core/src/{model,index,annotations,reports,bundle}.rs`.

### Server and jobs

| Operation | Request | Response |
|---|---|---|
| `hello` | `{}` | `{ api_version, api_minor, server_version, home, loopback_only }` |
| `job` | `{ id }` | `Job` |
| `cancel_job` | `{ id }` | `null` |

- `home` is the server's home folder, for showing paths as `~/…`.
- Clients check `api_version` equals theirs and `api_minor` is at least what they need.

Long work runs as a **job** owned by the server. A client that disconnects never cancels a job;
only `cancel_job` (or the kind-specific cancel operations) does. Jobs live in memory and are lost
when the server stops; at most 100 finished jobs are kept.

```ts
type Job = {
  id: string;                 // "j<server start ms>-<n>", unique across server runs
  kind: "index" | "generate" | "analyze" | "report";
  state: "running" | "finished" | "failed" | "cancelled";
  key: string | null;         // a generate job's session key
  progress: { done: number; total: number } | null;
  started_at: number;         // ms since 1970
  ended_at: number | null;
  error: string | null;
};
```

- `job` and `cancel_job` answer 404 `not_found` for an unknown id.
- An index job can't be cancelled: 409 `conflict`.
- Cancelling a generate, analyze or report job stops its `claude` process. The job ends
  `cancelled` after publishing its done event.

### Session folders and the index

| Operation | Request | Response |
|---|---|---|
| `get_roots` | `{}` | `[{ path, exists, sessions }]` |
| `suggested_roots` | `{}` | `string[]`: agent session folders under the server's home that exist and aren't added |
| `add_root` | `{ path }` | `{ path, exists, sessions }` |
| `remove_root` | `{ path }` | `null` |
| `reindex` | `{ full?: bool }` | job id (string) |
| `library_errors` | `{}` | `[{ path, error }]`: files that failed to index |

- `add_root` takes a path typed on the server's machine. There is no file browser over HTTP.
- `reindex` coalesces: while a pass runs, more calls return the running job's id and leave at most
  one follow-up pass, a full one if any call asked for `full`.
- The server watches the session folders and indexes new and changed files on its own within
  about a second, so clients rarely need `reindex`.

### Sessions

| Operation | Request | Response |
|---|---|---|
| `list_sessions` | `{ repo?: string, since?: string }` | `SessionRow[]` |
| `search` | `{ query }` | `[{ path, snippet, rank }]` |
| `load_session` | `{ path, branch?: string }` | `Event[]` |

- `repo` is a repository name as the index names projects. `since` is read like `sp --since`:
  a date (`2026-11-01`), a date and time (`2026-11-01T09:30`), or an age (`7d`, `12h`, `2w`).
- `SessionRow` is in `src-tauri/core/src/model.rs`. `path` identifies the session in every other
  operation.
- `branch` is a pi branch's leaf entry.
- Images in `load_session`'s events are not inlined. Each is replaced by a URL
  `/api/v1/image/{generation}/{event}/{image}` (see [Images](#images)).

### Notes and analysis

| Operation | Request | Response |
|---|---|---|
| `session_notes` | `{ key, path, branch?: string }` | `{ meta: SessionMeta \| null, annotations: Annotation[], revision: string \| null }` |
| `save_meta` | `{ meta: SessionMeta }` | `SessionMeta \| null` |
| `save_annotation` | `{ annotation: Annotation }` | `Annotation` |
| `delete_annotation` | `{ id }` | `null` |
| `note_summaries` | `{}` | `Summary[]`: per-session outcome, components and annotation counts |
| `generate_notes` | `{ key, path, branch?: string, focus?: string }` | job id |
| `cancel_generate` | `{ key }` | `null` |
| `generating` | `{}` | `string[]`: session keys with a generate job running |
| `default_prompts` | `{}` | `{ analyze, report }`: the built-in focus texts |
| `analyze_sessions` | `{ paths: string[], force?: bool, focus?: string }` | job id |

- `key` is a session's key (its source and id); `path` is its file, as `list_sessions` gives it.
- `generate_notes` answers 409 while a generate job for the same key runs.
- `analyze_sessions` analyzes three sessions at a time, skips analyzed ones unless `force`, and
  answers 409 while another analyze job runs. The job ends `failed` if any session failed.
- `focus` left out uses the focus text saved in the UI's settings, else the built-in one.

### Reports

| Operation | Request | Response |
|---|---|---|
| `generate_report` | `{ paths: string[], model?: string, focus?: string, analyze_first?: bool }` | job id |
| `cancel_report` | `{ job }` | `null` |
| `list_reports` | `{}` | `[{ id, created_at, title, model, sessions, actions }]`, newest first |
| `get_report` | `{ id }` | `Report \| null` |
| `report_annotations` | `{ id }` | `[{ annotation, current_version }]`: the annotations a report cites |
| `delete_report` | `{ id }` | `null` |
| `action_responses` | `{ id }` | `ActionResponse[]` |
| `set_action_response` | `{ input: { report_id, action_index, status?, note?, user_impact? } }` | `ActionResponse \| null` |
| `action_verdicts` | `{ id }` | `[{ action_index, verdict, reasoning, report_id, report_title, created_at }]` |

- `model` defaults to `sonnet`.
- `analyze_first` analyzes the sessions without an analysis first; otherwise they are left out.
- `generate_report` answers 422 with no sessions and 409 while another report job runs.
- `status` is `done`, `in_progress`, `declined` or `considering`. `user_impact` is `helped` or
  `did_not_help`.

### Settings and logging

| Operation | Request | Response |
|---|---|---|
| `get_setting` | `{ key }` | `string \| null` |
| `set_setting` | `{ key, value }` | `null` |
| `log_frontend` | `{ level, message }` | `null` |

- Keys are the UI's own (stored as `ui.<key>` in the database, so the server's internal settings
  can't be reached). An empty key, one over 200 characters, or one with a control character is 422.
- `log_frontend` appends a line to `server.log`. The message is capped and control characters
  (newlines included) are escaped.

### Export and import

| Operation | Request | Response |
|---|---|---|
| `export_preview` | `{ path, branch?: string, subagents?: string[], request: number }` | `{ token, summary, subagents: [{ path, title, included }] }` |
| `export_session` | `{ token, exported_by }` | **raw**: the `.spsession` file |
| `export_cancel` | `{}` | `null` |
| `import_bundle` | **raw**: the `.spsession` file | `ImportReport` |
| `remove_imported` | `{ path }` | `null` |
| `export_project` | `{ repo, since?: string }` | **raw**: a sync stream |

Exporting is two steps, so what the user reviewed is exactly what leaves the machine:

1. `export_preview` builds the bundle and keeps it in a slot for the calling device. `request`
   increases with every preview; only the newest is kept. `subagents` left out includes all.
2. `export_session` with the preview's `token` answers the file as
   `Content-Type: application/octet-stream` with `Content-Disposition: attachment;
   filename="<title>.spsession"`. A stale token is 404; an empty `exported_by` is 422.

`import_bundle` takes the file itself as the body (`Content-Type: application/octet-stream`),
not JSON. The query string carries its option:

```
POST /api/v1/import_bundle?on_conflict=replace
```

- Without `on_conflict`, an import with conflicts writes nothing and lists them in `conflicts`.
  `replace` overwrites the library's copies; `keep` keeps them.
- `ImportReport` is `{ imported, unchanged, kept, conflicts: [{ id, existing_title,
  incoming_title }], empty, ... }` (`src-tauri/core/src/bundle.rs`).
- Uploads over 512 MiB get 413 `too_large` and leave nothing behind. The upload is staged in an
  owner-only (0600) file in the data folder while it is imported.
- The sessions are indexed before the answer, so they are listed when it returns.

`export_project` streams a repository's sessions, analysis and reports in the sync format
(`docs/sync-format.md`). `pull` on another server reads it. A cut stream has no end record, so the
reader stores nothing from it.

- A repository with no session indexed on the server gets 404 `not_found` before anything streams.
- An export that fails part way writes why to `server.log`.

### Server extensions

The server crate adds these to the core set (`src-tauri/server/src/pairing.rs`), and the client
crate adds `pull` (`src-tauri/client/src/pull.rs`).

| Operation | Request | Response | Auth |
|---|---|---|---|
| `pair` | `{}` | `{ pairing, expires_at }` | local token only |
| `connect` | `{ secret, name, browser?: bool }` | `{ device, token }` | **none** |
| `devices` | `{}` | `[{ id, name, paired_at }]` | local token only |
| `unpair` | `{ id }` | `null` (404 for an unknown id) | local token only |
| `pull` | `{ server, project, since?: string, dry_run?: bool }` | `PullResponse` | local token only |
| `service` | `{ action }` | | in process only |

- **Local token only** means the [local token](#local-token-servertoken) on the loopback listener (or a call
  in process). A paired device or browser gets the same 403 `forbidden_origin` as any other
  refused request: a device can't mint, list or revoke devices, or make the server pull from
  other servers with its stored credentials.
- `connect`'s request body may be at most 64 KiB; a larger one gets 413 `too_large`.

- `pair` creates a one-time offer and answers its [pairing string](#pairing-strings). It needs a
  running server: in process it answers 409. At most 5 offers are open at once (409 beyond that).
  With `--bind` given, it waits up to 2 seconds for the external listener and puts its real
  port in the string.
- `connect` redeems an offer's secret and registers a device under `name`. Every failure (wrong,
  expired, already used, or wiped secret) is the same 401. Five wrong secrets wipe every open offer.
- `pull` copies a repository's sessions from a server this server paired with (`server` is its
  name in this server's `servers.json`) into `project`, a checkout on this server's machine.
  `PullResponse` is `{ exporter, dry_run, files: [{ path, change, is_session }], sessions,
  analyses, annotations, responses, reports_added }`, where each count is `{ added, replaced,
  kept_local, unchanged }`.
- `service` (install, uninstall, start, stop, status of the per-user service) is not reachable
  over HTTP: the server answers 404 `unknown_operation`. `sp service` runs it in process.

## Errors

Every error is JSON:

```json
{ "code": "conflict", "message": "Another report is being written" }
```

| `code` | Status | When |
|---|---|---|
| `unauthenticated` | 401 | Missing or wrong credential, or a failed `connect` |
| `forbidden_origin` | 403 | Host or Origin not allowed; cookie request without `x-sp-csrf` or from another origin; a local-only operation called by a paired device or browser |
| `unknown_operation` | 404 | No operation has that name (only after authentication) |
| `not_found` | 404 | Unknown job, report, device, image or path under `/api/` |
| `method_not_allowed` | 405 | Wrong method for an operation, the event stream or an image |
| `conflict` | 409 | The work is already running, or the request clashes with server state |
| `too_large` | 413 | Upload or JSON body over the limit |
| `bad_request` | 422 | Body isn't valid JSON or doesn't match the request type; invalid value |
| `internal` | 500 | Anything else; the message says what failed |

The one exception is a non-numeric image path, which answers 400 with code `bad_request`.

## Events

```
GET /api/v1/events
Last-Event-ID: <id>        (optional)
```

A [server-sent events](https://html.spec.whatwg.org/multipage/server-sent-events.html) stream
(`Content-Type: text/event-stream`). The stream starts with a `:` comment line and sends
`: keepalive` every 3 seconds when nothing else happens. Each event is:

```
id: 1763800000123
data: {"id":1763800000123,"at":1763800000456,"event":{"name":"index-progress","payload":{"done":40,"total":200}}}
```

- `id` is the event id; `at` is when it happened (ms since 1970).
- Without `Last-Event-ID` the stream starts from now.
- With `Last-Event-ID`, the server first replays every newer event it still holds, then forwards
  new ones, with no gap and no duplicate.
- Event ids start at the server's start time in ms and increase by one. An id from before a
  restart, or one older than the server's buffer of recent events (1000 by default), is answered
  with a `snapshot` event instead of a replay. A client reading too slowly also gets a `snapshot`.
- A reader keeps the id of the last event it handled and drops an event whose id is at or below
  it, except a `snapshot`: a `snapshot` is always accepted and its id becomes the last id. A
  server restarted with its clock behind the old one answers an old id with a `snapshot` whose id
  is lower, and its next events follow on from that id.
- The stream of a device that is unpaired ends within a second.

| `name` | `payload` |
|---|---|
| `index-started` | none |
| `index-progress` | `{ done, total }` |
| `index-done` | `{ scanned, updated, removed, failed, elapsed_ms }` |
| `index-error` | message (string) |
| `generate-progress` | `{ key, done, total }` |
| `generate-done` | `{ key, error, cancelled, rejected }` |
| `analyze-progress` | `{ job, done, total, key?, title?, summary?, error? }` |
| `analyze-done` | `{ job, analyzed, skipped, failed, cancelled }` |
| `report-progress` | `{ job, done, total }` |
| `report-done` | `{ job, report, error, cancelled }` |
| `snapshot` | `{ running: Job[] }`: the jobs running now, in place of the events missed |

Events report progress; they are not how a client learns a job finished. Poll `job` for that. A
job a client followed that is missing from a `snapshot`'s `running` list, or that `job` answers
404 for, ended while the client wasn't listening (for example, the server restarted).

## Images

```
GET /api/v1/image/{generation}/{event}/{image}
```

- The path comes from `load_session`'s events. All three parts are whole numbers.
- Authenticated like any other request (bearer token or cookie). An `<img>` tag can't send a
  bearer token, so the desktop app fetches images and shows them as blob URLs.
- 200 with the image's own `Content-Type`, `Cache-Control: private, max-age=3600` and an `ETag`.
  `If-None-Match` with that ETag answers 304.
- The server keeps the images of the 4 most recently loaded sessions. An older `generation`, or
  an index out of range, is 404 `not_found`. A non-numeric part is 400.

## Pairing strings

`sp pair` (or the `pair` operation) prints a string like `sp1.eyJhZGRyZXNzIjoi…`: `sp1.`
followed by base64url (no padding) JSON:

```json
{ "address": "studio.example.ts.net", "port": 47801, "fingerprint": "3fa1…(64 hex)", "secret": "…" }
```

- `address` is `--advertise` (else the `--bind` address). Without
  `--bind` it is `127.0.0.1` and `port` is the loopback port, so the string only works on
  the same machine (as for a browser on it).
- `fingerprint` is the SHA-256 of the external listener's certificate (DER), lowercase hex. It is
  `null` when the listener serves plain HTTP.
- `secret` is 32 random bytes, base64url. It works once, within 5 minutes.

A client redeems it by sending `{ secret, name }` to `POST /api/v1/connect` at
`https://{address}:{port}` (or `http://` when `fingerprint` is null), and keeps the device token it
gets back. `sp connect` saves it in `servers.json`.

## TLS

The external listener serves HTTPS in one of three modes, set by `sp serve` flags:

| Flags | Certificate |
|---|---|
| none (default) | Generated into `tls/` in the data folder |
| `--tls-cert <pem path> --tls-key <pem path>` | Yours |
| `--allow-insecure-http` | None: plain HTTP. Tokens cross the network unencrypted |

- `--allow-insecure-http` prints a warning at start. It can't be combined with `--tls-cert`.

- Clients pin the certificate by the fingerprint in the pairing string. They accept exactly that
  certificate and still verify the handshake signature; they don't consult certificate
  authorities or check the host name.
- The generated certificate keeps its fingerprint across restarts. It is regenerated when its set
  of names changes (a new `--advertise` or `--bind`), and every device must then pair again.
- A browser has no pinning, so on a self-signed certificate it shows a warning the user must
  accept once.
- TLS uses rustls with the ring crypto provider.

## `sp serve` flags

Every flag is optional. With none, the server listens on `127.0.0.1:47800` and nowhere else.
`sp service install` takes the same flags and runs the service with them.

| Flag | Default | Meaning |
|---|---|---|
| `--port <port>` | 47800 | Loopback listener port. Moves `--external-port` to the next port unless that is given |
| `--external-port <port>` | 47801 | External listener port; must differ from `--port` |
| `--bind <addr>` | none | Address to listen on off loopback; without it the server never does |
| `--advertise <host>` | none | Host name or address other machines reach the server at. Goes into pairing strings and the certificate, and is the only `Host` the external listener answers. Without it the bind address is used; with a wildcard bind (`0.0.0.0` or `::`) it is required, and `sp serve` refuses to start without it. An IPv6 address appears in brackets (`[fd00::1]:47801`) in `Host`, origins and pairing strings |
| `--tls-cert <file>`, `--tls-key <file>` | generated | See [TLS](#tls) |
| `--allow-insecure-http` | off | See [TLS](#tls) |

`sp serve --exit-on-stdin-eof` also stops when its standard input closes, cancelling running jobs.
The Mac app runs its sidecar this way. `sp service install` doesn't take it.

`--data-dir <folder>`, which every `sp` command takes, moves the data folder. `sp service`
commands take `--label <name>`, the launchd label or systemd unit name; the default is
`com.danmactough.sessionplayer.server` on a Mac and `session-player` on Linux.

## Files in the data folder

The data folder is `~/Library/Application Support/com.danmactough.sessionplayer` on a Mac and
`~/.local/share/com.danmactough.sessionplayer` on Linux, unless `sp` is given `--data-dir`.

| File | Contents |
|---|---|
| `server.lock` | `{ "pid", "port" }` of the running server. Locked while it runs, so one server per data folder |
| `server.token` | The local token, mode 0600, new on every start |
| `devices.json` | Paired devices: id, name, pairing time and the SHA-256 of each token. Mode 0600 |
| `tls/` | The generated certificate and key |
| `servers.json` | Servers this machine paired with (`sp connect`, the app): address, port, pinned fingerprint and device token. Mode 0600 |

`server.log` is in the logs folder beside `analyze.log` (`~/Library/Logs/com.danmactough.sessionplayer`
on a Mac, `logs/` under the data folder on Linux). The service sends the server's output there;
`sp serve` in a terminal prints to the terminal instead.

A local client finds the server by reading `server.lock` for the port and `server.token` for the
token, then calls `hello`.

## Example

```bash
DATA="$HOME/Library/Application Support/com.danmactough.sessionplayer"
PORT=$(jq -r .port "$DATA/server.lock")
TOKEN=$(cat "$DATA/server.token")

curl -s -X POST "http://127.0.0.1:$PORT/api/v1/list_sessions" \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"since":"7d"}' | jq '.[].title'

curl -sN "http://127.0.0.1:$PORT/api/v1/events" -H "Authorization: Bearer $TOKEN"
```
