import type {
  ExportPreview,
  ActionResponse,
  ActionResponseInput,
  HelloResponse,
  ImportReport,
  Job,
  LatestVerdict,
  LibraryError,
  Annotation,
  NoteSummary,
  CitedAnnotation,
  PullResponse,
  Report,
  ReportSummary,
  SessionMeta,
  OnConflict,
  RootInfo,
  SearchHit,
  ServerEvent,
  ServerEventName,
  ServerEventPayloads,
  SessionEvent,
  SessionRow,
} from "./types";
import type { Download, Transport } from "./lib/transport";
import * as native from "./lib/native";

// The server every call goes to. ServerProvider sets it before the app renders and replaces it
// when the user switches servers (the app is remounted then).
let active: Transport | null = null;

export function setTransport(transport: Transport | null) {
  active = transport;
}

function server(): Transport {
  if (!active) throw new Error("No server is connected.");
  return active;
}

const call = <T>(op: string, req: unknown = {}): Promise<T> => {
  try {
    return server().call<T>(op, req);
  } catch (e) {
    return Promise.reject(e);
  }
};

// ---- Server events ----

type Handler = (payload: never) => void;
const handlers = new Map<string, Set<Handler>>();

/**
 * Runs `handler` for each event of this name from the active server's event stream; returns the
 * unsubscribe. `stream-open` runs each time the stream reconnects, so followed jobs can be checked.
 */
export function onServerEvent<N extends ServerEventName>(name: N, handler: (payload: ServerEventPayloads[N]) => void): () => void;
export function onServerEvent(name: "stream-open", handler: () => void): () => void;
export function onServerEvent(name: string, handler: (payload: never) => void): () => void {
  let set = handlers.get(name);
  if (!set) handlers.set(name, (set = new Set()));
  set.add(handler);
  return () => void set.delete(handler);
}

/** Hands an event from the stream to its handlers. */
export function dispatchServerEvent(e: ServerEvent) {
  const payload = "payload" in e.event ? e.event.payload : undefined;
  for (const h of handlers.get(e.event.name) ?? []) (h as (p: unknown) => void)(payload);
}

export function dispatchStreamOpen() {
  for (const h of handlers.get("stream-open") ?? []) (h as () => void)();
}

// ---- Operations ----

// Overlapping previews can finish in any order; the server keeps the bundle of the newest
// request. Based on the clock so numbers keep increasing after the page reloads while the server
// keeps running.
let lastExportRequest = 0;
const nextExportRequest = () => (lastExportRequest = Math.max(Date.now(), lastExportRequest + 1));

export const api = {
  hello: () => call<HelloResponse>("hello"),
  job: (id: string) => call<Job>("job", { id }),
  listSessions: () => call<SessionRow[]>("list_sessions", { repo: null, since: null }),
  search: (query: string) => call<SearchHit[]>("search", { query }),
  loadSession: (path: string, branch?: string) => call<SessionEvent[]>("load_session", { path, branch: branch ?? null }),
  getRoots: () => call<RootInfo[]>("get_roots"),
  suggestedRoots: () => call<string[]>("suggested_roots"),
  addRoot: (path: string) => call<RootInfo>("add_root", { path }),
  removeRoot: (path: string) => call<void>("remove_root", { path }),
  /** Resolves to the index job's id; progress arrives as index-* events. */
  reindex: (full: boolean) => call<string>("reindex", { full }),
  log: (level: "info" | "error", message: string) => call<void>("log_frontend", { level, message }).catch(() => {}),
  getSetting: (key: string) => call<string | null>("get_setting", { key }),
  setSetting: (key: string, value: string) => call<void>("set_setting", { key, value }),
  /** `subagents` lists the subagent paths to include; omit it to include them all. */
  exportPreview: (path: string, branch?: string, subagents?: string[]) =>
    call<ExportPreview>("export_preview", { path, branch: branch ?? null, subagents: subagents ?? null, request: nextExportRequest() }),
  /** The previewed bundle as a file. `exportedBy` is the user's name; the bundle records it as the exporter. */
  exportDownload: (token: number, exportedBy: string): Promise<Download> => {
    try {
      return server().download("export_session", { token, exported_by: exportedBy });
    } catch (e) {
      return Promise.reject(e);
    }
  },
  exportCancel: () => call<void>("export_cancel"),
  importBundle: (file: Blob, onConflict?: OnConflict): Promise<ImportReport> => {
    try {
      return server().upload<ImportReport>("import_bundle", onConflict ? { on_conflict: onConflict } : {}, file);
    } catch (e) {
      return Promise.reject(e);
    }
  },
  removeImported: (path: string) => call<void>("remove_imported", { path }),
  libraryErrors: () => call<LibraryError[]>("library_errors"),
  sessionNotes: (key: string, path: string, branch?: string) =>
    call<{ meta: SessionMeta | null; annotations: Annotation[]; revision: string | null }>("session_notes", { key, path, branch: branch ?? null }),
  /** Starts generating; resolves to the job's id. Watch for "generate-progress" and "generate-done". */
  /** `focus` is the user's own text for what to look for; omit it for the built-in text. */
  generateNotes: (key: string, path: string, branch?: string, focus?: string) =>
    call<string>("generate_notes", { key, path, branch: branch ?? null, focus: focus ?? null }),
  /** The built-in focus text for Analyze and for reports. */
  defaultPrompts: () => call<{ analyze: string; report: string }>("default_prompts"),
  cancelGenerate: (key: string) => call<void>("cancel_generate", { key }),
  generating: () => call<string[]>("generating"),
  /** Resolves to the metadata as saved, or null when every field was empty. */
  saveMeta: (meta: SessionMeta) => call<SessionMeta | null>("save_meta", { meta }),
  saveAnnotation: (annotation: Annotation) => call<Annotation>("save_annotation", { annotation }),
  deleteAnnotation: (id: string) => call<void>("delete_annotation", { id }),
  noteSummaries: () => call<NoteSummary[]>("note_summaries"),
  /** Starts a report and resolves to its job id; watch for "report-progress" and "report-done".
   * `paths` are session files as list_sessions gives them. `model` is an alias such as "sonnet"; omit it for the default. */
  generateReport: (paths: string[], model?: string, focus?: string) =>
    call<string>("generate_report", { paths, model: model ?? null, focus: focus ?? null, analyze_first: false }),
  cancelReport: (job: string) => call<void>("cancel_report", { job }),
  /** Newest first. */
  listReports: () => call<ReportSummary[]>("list_reports"),
  getReport: (id: string) => call<Report | null>("get_report", { id }),
  deleteReport: (id: string) => call<void>("delete_report", { id }),
  actionResponses: (id: string) => call<ActionResponse[]>("action_responses", { id }),
  setActionResponse: (input: ActionResponseInput) => call<ActionResponse | null>("set_action_response", { input }),
  actionVerdicts: (id: string) => call<LatestVerdict[]>("action_verdicts", { id }),
  reportAnnotations: (id: string) => call<CitedAnnotation[]>("report_annotations", { id }),
  /** An image URL from load_session as a blob URL the caller revokes. */
  image: (url: string) => server().image(url),

  // Native work, done by the app shell on this machine.
  reveal: native.reveal,
  openTargets: native.openTargets,
  openInApp: native.openInApp,
  terminalApps: native.terminalApps,
  openInTerminal: native.openInTerminal,
  openAnalyzeLog: native.openAnalyzeLog,
};

/**
 * Pulls a paired server's sessions of the project checked out at `project` into this machine's
 * library: the `pull` operation on the local server, which reaches the paired one itself.
 */
export async function pullToLocal(params: { server: string; project: string; local: Transport }): Promise<PullResponse> {
  return params.local.call<PullResponse>("pull", { server: params.server, project: params.project, since: null, dry_run: false });
}
