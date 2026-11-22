// The only module that imports @tauri-apps/* (an oxlint rule enforces it). Everything here is the
// app shell's own work: dialogs, other apps, the updater, managing the local server, and the relay
// to paired servers. In a browser, dialogs fall back to the page's own and the rest reject.
import { getVersion } from "@tauri-apps/api/app";
import { Channel, invoke as tauriInvoke, type InvokeArgs, type InvokeOptions } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ask as tauriAsk, message as tauriMessage, open as tauriOpen, save as tauriSave } from "@tauri-apps/plugin-dialog";
import { openUrl as tauriOpenUrl } from "@tauri-apps/plugin-opener";
import { relaunch as tauriRelaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import type { LocalServerState, OpenTarget, ServerEvent, ServerRef, SessionRow, TerminalApp } from "../types";
import type { Relay } from "./transport";

export type { Update };

/** True inside the desktop app; false in a browser. */
export const isApp = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** A shell command; rejects in a browser, which has no shell. */
function invoke<T>(cmd: string, args?: InvokeArgs, options?: InvokeOptions): Promise<T> {
  if (!isApp) return Promise.reject(new Error("Only the Session Player app can do this."));
  return tauriInvoke<T>(cmd, args, options);
}

// ---- Dialogs ----

export function ask(text: string, options: { title?: string; kind?: "info" | "warning" | "error"; okLabel?: string; cancelLabel?: string }): Promise<boolean> {
  if (isApp) return tauriAsk(text, options);
  return Promise.resolve(window.confirm(options.title ? `${options.title}\n\n${text}` : text));
}

/**
 * A message, or a choice among `buttons`; resolves to the label of the button pressed. In a
 * browser a three-way choice becomes OK (yes) or Cancel (no), so it can't be cancelled there.
 */
export async function message(
  text: string,
  options: { title?: string; kind?: "info" | "warning" | "error"; buttons?: { yes: string; no: string; cancel: string } },
): Promise<string> {
  if (isApp) return tauriMessage(text, options);
  const full = options.title ? `${options.title}\n\n${text}` : text;
  if (!options.buttons) {
    window.alert(full);
    return "Ok";
  }
  const { yes, no } = options.buttons;
  return window.confirm(`${full}\n\nOK: ${yes}\nCancel: ${no}`) ? yes : no;
}

/** The save dialog; null when cancelled. App only. */
export function saveDialog(options: { title: string; defaultPath?: string; filters?: { name: string; extensions: string[] }[] }): Promise<string | null> {
  return tauriSave(options);
}

/** The open dialog for one folder; null when cancelled. App only. */
export async function chooseFolder(options: { title: string; defaultPath?: string }): Promise<string | null> {
  const picked = await tauriOpen({ ...options, directory: true, multiple: false });
  return typeof picked === "string" ? picked : null;
}

// ---- Window, links, version, updater ----

export function setWindowTheme(theme: "light" | "dark" | null) {
  if (!isApp) return;
  void getCurrentWindow()
    .setTheme(theme)
    .catch(() => {});
}

export function openUrl(href: string): Promise<void> {
  if (isApp) return tauriOpenUrl(href);
  window.open(href, "_blank", "noopener");
  return Promise.resolve();
}

export function appVersion(): Promise<string> {
  return getVersion();
}

export function checkForUpdate(): Promise<Update | null> {
  return check();
}

export function relaunch(): Promise<void> {
  return tauriRelaunch();
}

/** A menu event from the shell (`open-settings`, `check-for-updates`, `import-session`). Returns the unsubscribe. */
export function onMenuEvent(name: string, handler: () => void): () => void {
  if (!isApp) return () => {};
  const un = listen(name, () => handler());
  return () => void un.then((f) => f());
}

// ---- Other apps and this machine's files ----

export const reveal = (path: string) => invoke<void>("reveal", { path });
export const openTargets = (source: string, nativeId: string, path: string) => invoke<OpenTarget[]>("open_targets", { source, nativeId, path });
export const openInApp = (url: string) => invoke<void>("open_in_app", { url });
export const terminalApps = () => invoke<TerminalApp[]>("terminal_apps");
export const openInTerminal = (app: string, s: SessionRow) =>
  invoke<void>("open_in_terminal", { app, source: s.source, nativeId: s.native_id, path: s.path, cwd: s.cwd });
/** Opens the log of Analyze and report runs on this machine. */
export const openAnalyzeLog = () => invoke<void>("open_analyze_log");
/** Writes a report's Markdown to `dest` (from the save dialog); resolves to the path written. */
export const saveMarkdown = (dest: string, text: string) => invoke<string>("save_markdown", { dest, text });
/** Writes `bytes` to `dest` (from the save dialog); resolves to the path written. */
export async function saveFile(dest: string, bytes: Blob): Promise<string> {
  return invoke<string>("save_file", new Uint8Array(await bytes.arrayBuffer()), { headers: { dest: encodeURIComponent(dest) } });
}

// ---- The local server and paired servers ----

export const localServerState = () => invoke<LocalServerState>("local_server_state");
export const localServerInstall = () => invoke<LocalServerState>("local_server_install");
export const localServerStart = () => invoke<LocalServerState>("local_server_start");
export const localServerConnection = () => invoke<{ url: string; token: string }>("local_server_connection");
export const pairedServersList = () => invoke<ServerRef[]>("paired_servers_list");
export const pairedServerConnect = (pairing: string, name: string | null) => invoke<ServerRef>("paired_server_connect", { pairing, name });
export const pairedServerRemove = (name: string) => invoke<void>("paired_server_remove", { name });

/** The shell's relay to paired servers, for RelayTransport. */
export const relay: Relay = {
  request: (body, headers) => invoke<ArrayBuffer>("relay_request", body, { headers }),
  image: (server, path) => invoke<ArrayBuffer>("relay_image", { server, path }),
  openStream: ({ server, lastId, onEvent }) => {
    const channel = new Channel<ServerEvent>();
    channel.onmessage = onEvent;
    return invoke<number>("relay_open_stream", { server, lastId, onEvent: channel });
  },
  closeStream: (id) => invoke<void>("relay_close_stream", { id }),
};
