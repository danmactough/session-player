import type { ServerEvent } from "../types";

/** A failed call, with the server's message. `String(e)` gives the message alone, as Tauri's errors did. */
export class ServerError extends Error {
  /** The server's error code (`not_found`, `unauthenticated` ...), or `network` when no answer came back. */
  code: string;
  status: number;
  constructor({ code, status, message }: { code: string; status: number; message: string }) {
    super(message);
    this.code = code;
    this.status = status;
  }
  override toString() {
    return this.message;
  }
}

export interface Download {
  blob: Blob;
  /** From Content-Disposition, when the server offered one. */
  filename: string | null;
}

export interface Transport {
  /** `POST /api/v1/<op>` with a JSON body; resolves to the JSON answer. */
  call<T>(op: string, req: unknown): Promise<T>;
  /** A raw operation's upload (`import_bundle`): the bytes as the body, its options as the URL query. */
  upload<T>(op: string, query: Record<string, string>, body: Blob): Promise<T>;
  /** A raw operation's answer (`export_session`) as a file. */
  download(op: string, req: unknown): Promise<Download>;
  /** An image URL from `load_session`, fetched with this server's credential, as a blob URL the caller revokes. */
  image(url: string): Promise<string>;
  /**
   * Follows the server's events after `lastId` (null: from now) until the returned function is
   * called. `onOpen` runs each time the stream (re)connects.
   */
  events(params: { lastId: number | null; onEvent: (e: ServerEvent) => void; onOpen?: () => void }): () => void;
}

/** How long the event reader waits before reconnecting after the stream breaks. */
const RETRY_MS = 2000;

async function errorFrom(status: number, body: string): Promise<ServerError> {
  try {
    const parsed = JSON.parse(body) as { code?: string; message?: string };
    if (parsed.message) return new ServerError({ code: parsed.code ?? "internal", status, message: parsed.message });
  } catch {
    // Not the server's JSON error (a proxy's page, an empty body).
  }
  return new ServerError({ code: "internal", status, message: body.trim() || `The server answered ${status}.` });
}

function filenameFrom(disposition: string | null): string | null {
  const match = disposition?.match(/filename="([^"]*)"/);
  return match ? match[1] : null;
}

/**
 * Talks to a server with `fetch`. The app reaches its local server on loopback with the token
 * from `server.token`; a browser reaches the server that served the page, with its `sp_session`
 * cookie (and the `x-sp-csrf` header the server requires on anything but GET).
 */
export class FetchTransport implements Transport {
  private baseUrl: string;
  private token: string | null;
  private refresh: (() => Promise<{ baseUrl: string; token: string }>) | null;
  private refreshing: Promise<boolean> | null = null;

  /**
   * `token` null uses the browser's cookie; `baseUrl` "" is the page's own origin. `refresh`
   * reads the credential again after a 401 or a broken event stream: the local server writes a
   * new `server.token` each time it starts, and in the app reading it starts a server when none
   * runs.
   */
  constructor({ baseUrl, token, refresh }: { baseUrl: string; token: string | null; refresh?: () => Promise<{ baseUrl: string; token: string }> }) {
    this.baseUrl = baseUrl.replace(/\/$/, "");
    this.token = token;
    this.refresh = refresh ?? null;
  }

  /** Reads the credential again; true when it changed. Concurrent 401s share one refresh. */
  private renew(): Promise<boolean> {
    const refresh = this.refresh;
    if (!refresh) return Promise.resolve(false);
    this.refreshing ??= refresh()
      .then((c) => {
        const changed = c.token !== this.token || c.baseUrl.replace(/\/$/, "") !== this.baseUrl;
        this.baseUrl = c.baseUrl.replace(/\/$/, "");
        this.token = c.token;
        return changed;
      })
      .catch(() => false)
      .finally(() => (this.refreshing = null));
    return this.refreshing;
  }

  private init(method: "GET" | "POST", headers: Record<string, string> = {}): RequestInit {
    const all: Record<string, string> = { ...headers };
    if (this.token) all.authorization = `Bearer ${this.token}`;
    else if (method !== "GET") all["x-sp-csrf"] = "1";
    return { method, headers: all, credentials: this.token ? "omit" : "same-origin" };
  }

  /** `build` makes the request's init, again after a refresh, so it carries the new token. */
  private async send(path: string, build: () => RequestInit, retried = false): Promise<Response> {
    let res: Response;
    try {
      res = await fetch(`${this.baseUrl}${path}`, build());
    } catch (e) {
      throw new ServerError({ code: "network", status: 0, message: `Couldn't reach the server: ${e instanceof Error ? e.message : String(e)}` });
    }
    if (res.status === 401 && !retried && (await this.renew())) return this.send(path, build, true);
    if (!res.ok) throw await errorFrom(res.status, await res.text().catch(() => ""));
    return res;
  }

  async call<T>(op: string, req: unknown): Promise<T> {
    const body = JSON.stringify(req ?? {});
    const res = await this.send(`/api/v1/${op}`, () => ({ ...this.init("POST", { "content-type": "application/json" }), body }));
    return (await res.json()) as T;
  }

  async upload<T>(op: string, query: Record<string, string>, body: Blob): Promise<T> {
    const q = new URLSearchParams(query).toString();
    const res = await this.send(`/api/v1/${op}${q ? `?${q}` : ""}`, () => ({ ...this.init("POST", { "content-type": "application/octet-stream" }), body }));
    return (await res.json()) as T;
  }

  async download(op: string, req: unknown): Promise<Download> {
    const body = JSON.stringify(req ?? {});
    const res = await this.send(`/api/v1/${op}`, () => ({ ...this.init("POST", { "content-type": "application/json" }), body }));
    return { blob: await res.blob(), filename: filenameFrom(res.headers.get("content-disposition")) };
  }

  async image(url: string): Promise<string> {
    const res = await this.send(url, () => this.init("GET"));
    return URL.createObjectURL(await res.blob());
  }

  events({ lastId, onEvent, onOpen }: { lastId: number | null; onEvent: (e: ServerEvent) => void; onOpen?: () => void }): () => void {
    let last = lastId;
    let stopped = false;
    let controller = new AbortController();
    let retry: ReturnType<typeof setTimeout> | undefined;

    const connect = async () => {
      controller = new AbortController();
      const init = this.init("GET", last === null ? {} : { "last-event-id": String(last) });
      try {
        const res = await fetch(`${this.baseUrl}/api/v1/events`, { ...init, signal: controller.signal });
        if (!res.ok || !res.body) throw await errorFrom(res.status, await res.text().catch(() => ""));
        let opened = false;
        for await (const frame of sseFrames(res.body)) {
          // The server's first bytes are a comment, sent once it has subscribed.
          if (!opened) {
            opened = true;
            onOpen?.();
          }
          if (frame === null) continue;
          if (!acceptEvent(last, frame)) continue;
          last = frame.id;
          onEvent(frame);
        }
      } catch {
        // Broken, refused or aborted; reconnect below unless closed.
      }
      // Read before the next attempt: the server may have restarted, or stopped.
      if (!stopped) await this.renew();
      if (!stopped) retry = setTimeout(() => void connect(), RETRY_MS);
    };
    void connect();
    return () => {
      stopped = true;
      clearTimeout(retry);
      controller.abort();
    };
  }
}

/**
 * Whether to handle `e`, given the id of the last event handled (null: none yet). Ids only
 * increase, so an event at or below `last` is a replay already seen and is dropped. A snapshot is
 * always taken: the server sends one in place of events the page missed, and after a restart its
 * ids start lower. The caller then sets `last` to `e.id`, even when that is lower.
 */
export function acceptEvent(last: number | null, e: ServerEvent): boolean {
  return e.event.name === "snapshot" || last === null || e.id > last;
}

/**
 * Reads an SSE body and yields each event's parsed `data:` (a ServerEvent), or null for a frame
 * with no data (comments and keepalives), so the caller learns the stream is open.
 */
export async function* sseFrames(body: ReadableStream<Uint8Array>): AsyncGenerator<ServerEvent | null> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) return;
      buffer += decoder.decode(value, { stream: true }).replace(/\r\n?/g, "\n");
      let end: number;
      while ((end = buffer.indexOf("\n\n")) >= 0) {
        const block = buffer.slice(0, end);
        buffer = buffer.slice(end + 2);
        const data = block
          .split("\n")
          .filter((line) => line.startsWith("data:"))
          .map((line) => line.slice(5).replace(/^ /, ""))
          .join("\n");
        yield data ? (JSON.parse(data) as ServerEvent) : null;
      }
    }
  } finally {
    reader.releaseLock();
  }
}

interface Frame {
  status: number;
  headers: [string, string][];
  body: Uint8Array;
}

/** `[u32 BE head length][head JSON][body]`, as the shell's relay commands answer. */
export function parseFrame(buffer: ArrayBuffer): Frame {
  if (buffer.byteLength < 4) throw new ServerError({ code: "network", status: 0, message: "The relay's answer is damaged." });
  const n = new DataView(buffer).getUint32(0, false);
  if (4 + n > buffer.byteLength) throw new ServerError({ code: "network", status: 0, message: "The relay's answer is damaged." });
  const head = JSON.parse(new TextDecoder().decode(new Uint8Array(buffer, 4, n))) as { status: number; headers: [string, string][] };
  return { status: head.status, headers: head.headers, body: new Uint8Array(buffer, 4 + n) };
}

const headerOf = (frame: Frame, name: string) => frame.headers.find(([k]) => k === name)?.[1] ?? null;

/** The shell's relay commands (native.ts), passed in so this module loads outside the app too. */
export interface Relay {
  request(body: Uint8Array, headers: Record<string, string>): Promise<ArrayBuffer>;
  image(server: string, path: string): Promise<ArrayBuffer>;
  openStream(params: { server: string; lastId: number | null; onEvent: (e: ServerEvent) => void }): Promise<number>;
  closeStream(id: number): Promise<void>;
}

/** Talks to a paired remote server through the app shell, which holds its token and pinned certificate. */
export class RelayTransport implements Transport {
  private server: string;
  private relay: Relay;

  constructor({ server, relay }: { server: string; relay: Relay }) {
    this.server = server;
    this.relay = relay;
  }

  private async request(params: { op: string; body: Uint8Array; contentType: string; query?: Record<string, string> }): Promise<Frame> {
    const headers: Record<string, string> = { server: encodeURIComponent(this.server), op: params.op, "content-type": params.contentType };
    const q = params.query ? new URLSearchParams(params.query).toString() : "";
    if (q) headers.query = q;
    let framed: ArrayBuffer;
    try {
      framed = await this.relay.request(params.body, headers);
    } catch (e) {
      throw new ServerError({ code: "network", status: 0, message: String(e) });
    }
    const frame = parseFrame(framed);
    if (frame.status < 200 || frame.status > 299) throw await errorFrom(frame.status, new TextDecoder().decode(frame.body));
    return frame;
  }

  private json(req: unknown) {
    return new TextEncoder().encode(JSON.stringify(req ?? {}));
  }

  async call<T>(op: string, req: unknown): Promise<T> {
    const frame = await this.request({ op, body: this.json(req), contentType: "application/json" });
    return JSON.parse(new TextDecoder().decode(frame.body)) as T;
  }

  async upload<T>(op: string, query: Record<string, string>, body: Blob): Promise<T> {
    const bytes = new Uint8Array(await body.arrayBuffer());
    const frame = await this.request({ op, body: bytes, contentType: "application/octet-stream", query });
    return JSON.parse(new TextDecoder().decode(frame.body)) as T;
  }

  async download(op: string, req: unknown): Promise<Download> {
    const frame = await this.request({ op, body: this.json(req), contentType: "application/json" });
    const type = headerOf(frame, "content-type") ?? "application/octet-stream";
    return { blob: new Blob([frame.body.slice()], { type }), filename: filenameFrom(headerOf(frame, "content-disposition")) };
  }

  async image(url: string): Promise<string> {
    let framed: ArrayBuffer;
    try {
      framed = await this.relay.image(this.server, url);
    } catch (e) {
      throw new ServerError({ code: "network", status: 0, message: String(e) });
    }
    const frame = parseFrame(framed);
    if (frame.status !== 200) throw await errorFrom(frame.status, new TextDecoder().decode(frame.body));
    return URL.createObjectURL(new Blob([frame.body.slice()], { type: headerOf(frame, "content-type") ?? "application/octet-stream" }));
  }

  events({ lastId, onEvent, onOpen }: { lastId: number | null; onEvent: (e: ServerEvent) => void; onOpen?: () => void }): () => void {
    // The shell reconnects by itself from the last id it delivered; this only drops repeats.
    let last = lastId;
    let closed = false;
    const opening = this.relay.openStream({
      server: this.server,
      lastId,
      onEvent: (e) => {
        if (!acceptEvent(last, e)) return;
        last = e.id;
        onEvent(e);
      },
    });
    opening.then(
      (id) => {
        if (closed) void this.relay.closeStream(id);
        else onOpen?.();
      },
      () => {},
    );
    return () => {
      closed = true;
      void opening.then((id) => this.relay.closeStream(id)).catch(() => {});
    };
  }
}
