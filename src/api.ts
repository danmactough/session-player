import { invoke } from "@tauri-apps/api/core";
import type { RootInfo, SearchHit, SessionEvent, SessionRow } from "./types";

export const api = {
  listSessions: () => invoke<SessionRow[]>("list_sessions"),
  search: (query: string) => invoke<SearchHit[]>("search", { query }),
  loadSession: (path: string) => invoke<SessionEvent[]>("load_session", { path }),
  getRoots: () => invoke<RootInfo[]>("get_roots"),
  suggestedRoots: () => invoke<string[]>("suggested_roots"),
  addRoot: (path: string) => invoke<void>("add_root", { path }),
  removeRoot: (path: string) => invoke<void>("remove_root", { path }),
  reindex: (full: boolean) => invoke<void>("reindex", { full }),
  reveal: (path: string) => invoke<void>("reveal", { path }),
  log: (level: "info" | "error", message: string) => invoke<void>("log_frontend", { level, message }).catch(() => {}),
  homeDir: () => invoke<string>("home_dir"),
  getSetting: (key: string) => invoke<string | null>("get_setting", { key }),
  setSetting: (key: string, value: string) => invoke<void>("set_setting", { key, value }),
};
