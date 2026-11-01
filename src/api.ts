import { invoke } from "@tauri-apps/api/core";
import type {
  ExportPreview,
  ImportReport,
  LibraryError,
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
  exportSave: (token: number, dest: string) => invoke<string>("export_save", { token, dest }),
  exportCancel: () => invoke<void>("export_cancel"),
  importBundle: (path: string, onConflict?: OnConflict) =>
    invoke<ImportReport>("import_bundle", { path, onConflict: onConflict ?? null }),
  removeImported: (path: string) => invoke<void>("remove_imported", { path }),
  libraryErrors: () => invoke<LibraryError[]>("library_errors"),
};
