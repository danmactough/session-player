import { useCallback, useEffect, useRef, useState } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { api } from "../api";

const FIRST_CHECK_DELAY_MS = 5_000;
const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

export type UpdateStatus =
  | { phase: "idle" }
  | { phase: "available"; version: string }
  | { phase: "downloading"; version: string; percent: number | null }
  | { phase: "ready"; version: string }
  | { phase: "installing"; version: string };

interface Options {
  autoCheck: boolean;
  onNotice: (text: string, ms: number) => void;
}

export function useUpdater({ autoCheck, onNotice }: Options) {
  const [status, setStatusState] = useState<UpdateStatus>({ phase: "idle" });
  const [dismissed, setDismissed] = useState(false);
  const [checking, setChecking] = useState(false);
  const [checkResult, setCheckResult] = useState<string | null>(null);
  const update = useRef<Update | null>(null);
  const statusRef = useRef(status);
  const checkingRef = useRef(false);

  const setStatus = useCallback((next: UpdateStatus) => {
    statusRef.current = next;
    setStatusState(next);
  }, []);

  const runCheck = useCallback(
    async (manual: boolean) => {
      if (checkingRef.current) return;
      // Replacing the Update object here would discard a finished or running download.
      const { phase } = statusRef.current;
      if (phase === "downloading" || phase === "ready" || phase === "installing") {
        if (manual) setDismissed(false);
        return;
      }
      checkingRef.current = true;
      setChecking(true);
      if (manual) setCheckResult(null);
      try {
        const found = await check();
        const previous = statusRef.current;
        const sameVersion = previous.phase === "available" && previous.version === found?.version;
        await update.current?.close().catch(() => {});
        update.current = found;
        if (found) {
          // "Later" holds for this version until the user asks, or a newer one is released.
          if (manual || !sameVersion) setDismissed(false);
          setStatus({ phase: "available", version: found.version });
          if (manual) setCheckResult(`Version ${found.version} is available.`);
        } else {
          setStatus({ phase: "idle" });
          if (manual) {
            setCheckResult("You have the latest version.");
            onNotice("You have the latest version", 4000);
          }
        }
      } catch (e) {
        // A scheduled check can fail whenever the Mac is offline, which isn't worth interrupting for.
        api.log("error", `update check failed: ${String(e)}`);
        if (manual) {
          setCheckResult(`Couldn't check for updates: ${String(e)}`);
          onNotice("Couldn't check for updates", 6000);
        }
      } finally {
        checkingRef.current = false;
        setChecking(false);
      }
    },
    [onNotice, setStatus],
  );

  const restart = useCallback(async () => {
    const current = update.current;
    const { phase } = statusRef.current;
    if (!current || phase !== "ready") return;
    const generating = await api.generating().catch(() => []);
    if (generating.length > 0) {
      const proceed = await ask("Notes are still being generated. Restarting now stops that work.", {
        title: "Restart Session Player?",
        kind: "warning",
        okLabel: "Restart",
        cancelLabel: "Not Now",
      });
      if (!proceed) return;
    }
    setStatus({ phase: "installing", version: current.version });
    try {
      await current.install();
      await relaunch();
    } catch (e) {
      api.log("error", `update install failed: ${String(e)}`);
      onNotice("Couldn't install the update", 6000);
      // The plugin consumes the downloaded bytes when it installs, so a retry downloads again.
      setStatus({ phase: "available", version: current.version });
    }
  }, [onNotice, setStatus]);

  const install = useCallback(async () => {
    const current = update.current;
    if (!current || statusRef.current.phase !== "available") return;
    const version = current.version;
    let total: number | null = null;
    let received = 0;
    setStatus({ phase: "downloading", version, percent: null });
    try {
      await current.download((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? null;
        } else if (event.event === "Progress") {
          received += event.data.chunkLength;
          if (total) setStatus({ phase: "downloading", version, percent: Math.min(100, Math.round((received / total) * 100)) });
        }
      });
    } catch (e) {
      api.log("error", `update download failed: ${String(e)}`);
      onNotice("Couldn't download the update", 6000);
      setStatus({ phase: "available", version });
      return;
    }
    setStatus({ phase: "ready", version });
    await restart();
  }, [onNotice, restart, setStatus]);

  useEffect(() => {
    // `tauri dev` and the browser preview shouldn't offer to replace themselves with a release.
    if (!autoCheck || import.meta.env.DEV) return;
    const first = window.setTimeout(() => void runCheck(false), FIRST_CHECK_DELAY_MS);
    const repeat = window.setInterval(() => void runCheck(false), CHECK_INTERVAL_MS);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(repeat);
    };
  }, [autoCheck, runCheck]);

  return {
    status,
    bannerHidden: dismissed,
    checking,
    checkResult,
    checkNow: useCallback(() => runCheck(true), [runCheck]),
    install,
    restart,
    later: useCallback(() => setDismissed(true), []),
  };
}
