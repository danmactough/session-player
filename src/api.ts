import { invoke } from "@tauri-apps/api/core";
import type {
  ExportPreview,
  ActionResponse,
  ActionResponseInput,
  ImportReport,
  LatestVerdict,
  LibraryError,
  Annotation,
  NoteSummary,
  CitedAnnotation,
  Report,
  ReportSummary,
  SessionMeta,
  OnConflict,
  OpenTarget,
  RootInfo,
  SearchHit,
  SessionEvent,
  SessionRow,
  TerminalApp,
} from "./types";

// Overlapping previews can finish in any order; the backend keeps the bundle
// of the newest request. Based on the clock so numbers keep increasing after
// the page reloads while the backend keeps running.
let lastExportRequest = 0;
const nextExportRequest = () => (lastExportRequest = Math.max(Date.now(), lastExportRequest + 1));

export const api = {
  listSessions: () => invoke<SessionRow[]>("list_sessions"),
  search: (query: string) => invoke<SearchHit[]>("search", { query }),
  loadSession: (path: string, branch?: string) => invoke<SessionEvent[]>("load_session", { path, branch: branch ?? null }),
  getRoots: () => invoke<RootInfo[]>("get_roots"),
  suggestedRoots: () => invoke<string[]>("suggested_roots"),
  addRoot: (path: string) => invoke<void>("add_root", { path }),
  removeRoot: (path: string) => invoke<void>("remove_root", { path }),
  reindex: (full: boolean) => invoke<void>("reindex", { full }),
  reveal: (path: string) => invoke<void>("reveal", { path }),
  openTargets: (source: string, nativeId: string, path: string) =>
    invoke<OpenTarget[]>("open_targets", { source, nativeId, path }),
  openInApp: (url: string) => invoke<void>("open_in_app", { url }),
  terminalApps: () => invoke<TerminalApp[]>("terminal_apps"),
  openInTerminal: (app: string, s: SessionRow) =>
    invoke<void>("open_in_terminal", { app, source: s.source, nativeId: s.native_id, path: s.path, cwd: s.cwd }),
  log: (level: "info" | "error", message: string) => invoke<void>("log_frontend", { level, message }).catch(() => {}),
  homeDir: () => invoke<string>("home_dir"),
  getSetting: (key: string) => invoke<string | null>("get_setting", { key }),
  setSetting: (key: string, value: string) => invoke<void>("set_setting", { key, value }),
  /** `subagents` lists the subagent paths to include; omit it to include them all. */
  exportPreview: (path: string, branch?: string, subagents?: string[]) =>
    invoke<ExportPreview>("export_preview", { path, branch: branch ?? null, subagents: subagents ?? null, request: nextExportRequest() }),
  /** Writes the previewed bundle; resolves to the path written. */
  /** `exportedBy` is the user's name; the bundle records it as the exporter. */
  exportSave: (token: number, dest: string, exportedBy: string) => invoke<string>("export_save", { token, dest, exportedBy }),
  exportCancel: () => invoke<void>("export_cancel"),
  importBundle: (path: string, onConflict?: OnConflict) =>
    invoke<ImportReport>("import_bundle", { path, onConflict: onConflict ?? null }),
  removeImported: (path: string) => invoke<void>("remove_imported", { path }),
  libraryErrors: () => invoke<LibraryError[]>("library_errors"),
  sessionNotes: (key: string, path: string, branch?: string) =>
    invoke<{ meta: SessionMeta | null; annotations: Annotation[]; revision: string | null }>("session_notes", { key, path, branch: branch ?? null }),
  /** Starts generating in the background; watch for "generate-progress" and "generate-done". */
  /** `focus` is the user's own text for what to look for; omit it for the built-in text. */
  generateNotes: (key: string, path: string, branch?: string, focus?: string) =>
    invoke<void>("generate_notes", { key, path, branch: branch ?? null, focus: focus ?? null }),
  /** The built-in focus text for Analyze and for reports. */
  /** Opens the log of Analyze and report runs, which has claude's own output when a run fails. */
  openAnalyzeLog: () => invoke<void>("open_analyze_log"),
  defaultPrompts: () => invoke<{ analyze: string; report: string }>("default_prompts"),
  cancelGenerate: (key: string) => invoke<void>("cancel_generate", { key }),
  generating: () => invoke<string[]>("generating"),
  /** Resolves to the metadata as saved, or null when every field was empty. */
  saveMeta: (meta: SessionMeta) => invoke<SessionMeta | null>("save_meta", { meta }),
  saveAnnotation: (annotation: Annotation) => invoke<Annotation>("save_annotation", { annotation }),
  deleteAnnotation: (id: string) => invoke<void>("delete_annotation", { id }),
  noteSummaries: () => invoke<NoteSummary[]>("note_summaries"),
  /** Starts a report in the background and resolves to its job id; watch for "report-progress" and "report-done".
   * `model` is an alias such as "sonnet"; omit it for the default. */
  generateReport: (sessions: { key: string; title: string }[], model?: string, focus?: string) =>
    invoke<string>("generate_report", { sessions, model: model ?? null, focus: focus ?? null }),
  cancelReport: (job: string) => invoke<void>("cancel_report", { job }),
  /** Newest first. */
  listReports: () => invoke<ReportSummary[]>("list_reports"),
  getReport: (id: string) => invoke<Report | null>("get_report", { id }),
  deleteReport: (id: string) => invoke<void>("delete_report", { id }),
  actionResponses: (id: string) => invoke<ActionResponse[]>("action_responses", { id }),
  setActionResponse: (input: ActionResponseInput) => invoke<ActionResponse | null>("set_action_response", { input }),
  actionVerdicts: (id: string) => invoke<LatestVerdict[]>("action_verdicts", { id }),
  reportAnnotations: (id: string) => invoke<CitedAnnotation[]>("report_annotations", { id }),
  /** Writes `text` to `dest` (a .md file the user chose); resolves to the path written. */
  saveMarkdown: (dest: string, text: string) => invoke<string>("save_markdown", { dest, text }),
};
