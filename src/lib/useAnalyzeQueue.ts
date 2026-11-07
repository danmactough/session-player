import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import type { SessionRow } from "../types";
import { sessionKey } from "./notes";

export type QueueStatus = "waiting" | "checking" | "running" | "done" | "skipped" | "failed" | "cancelled";

export interface QueueItem {
  /** The row's path; a pi branch has its own. */
  id: string;
  key: string;
  path: string;
  branch?: string;
  title: string;
  /** Analyze it even when its analysis is current. */
  reanalyze: boolean;
  /** What Analyze focuses on; absent uses the built-in text. */
  focus?: string;
  status: QueueStatus;
  progress?: { done: number; total: number };
  message?: string;
}

const active = (s: QueueStatus) => s === "waiting" || s === "checking" || s === "running";

/**
 * Analyzes sessions one at a time. The queue lives only in memory, so
 * quitting the app drops whatever hasn't run. A session whose analysis is
 * current for its transcript is skipped unless it was queued to be
 * re-analyzed.
 */
export function useAnalyzeQueue() {
  const [items, setItems] = useState<QueueItem[]>([]);
  const itemsRef = useRef(items);
  useLayoutEffect(() => {
    itemsRef.current = items;
  });
  // The item being checked or analyzed. A ref rather than a status, so the
  // next item can't start in the moment before a status change renders.
  const current = useRef<string | null>(null);

  const patch = useCallback((id: string, change: Partial<QueueItem>) => {
    setItems((list) => list.map((i) => (i.id === id ? { ...i, ...change } : i)));
  }, []);
  /** Records how an item ended and lets the next one start. */
  const finish = useCallback(
    (id: string, change: Partial<QueueItem>) => {
      if (current.current === id) current.current = null;
      patch(id, { progress: undefined, ...change });
    },
    [patch],
  );

  const enqueue = useCallback((rows: SessionRow[], reanalyze: boolean, focus?: string) => {
    setItems((list) => {
      // A session already waiting or running isn't queued twice; a finished one can be queued again.
      const queued = new Set(list.filter((i) => active(i.status)).map((i) => i.id));
      const added = rows
        .filter((s) => !queued.has(s.path))
        .map<QueueItem>((s) => ({
          id: s.path,
          key: sessionKey(s),
          path: s.branch_of ?? s.path,
          branch: s.branch_leaf ?? undefined,
          title: s.title,
          reanalyze,
          focus,
          status: "waiting",
        }));
      const requeued = new Set(added.map((a) => a.id));
      return [...list.filter((i) => !requeued.has(i.id)), ...added];
    });
  }, []);

  // Starts the next waiting session once the one before it has ended.
  useEffect(() => {
    if (current.current) return;
    const next = items.find((i) => i.status === "waiting");
    if (!next) return;
    current.current = next.id;
    void (async () => {
      try {
        patch(next.id, { status: "checking" });
        if (!next.reanalyze) {
          const n = await api.sessionNotes(next.key, next.path, next.branch);
          const analyzedFrom = n.meta?.generated_by?.revision ?? n.annotations.find((a) => a.revision)?.revision;
          if (analyzedFrom && analyzedFrom === n.revision) {
            finish(next.id, { status: "skipped", message: "Already analyzed" });
            return;
          }
        }
        // Cancelled while its notes were being checked.
        if (current.current !== next.id) return;
        patch(next.id, { status: "running", progress: { done: 0, total: 0 } });
        await api.generateNotes(next.key, next.path, next.branch, next.focus);
      } catch (e) {
        finish(next.id, { status: "failed", message: String(e) });
      }
    })();
  }, [items, patch, finish]);

  useEffect(() => {
    const running = (key: string) => itemsRef.current.find((i) => i.key === key && i.status === "running");
    const un = [
      listen<{ key: string; done: number; total: number }>("generate-progress", (e) => {
        const item = running(e.payload.key);
        if (item) patch(item.id, { progress: { done: e.payload.done, total: e.payload.total } });
      }),
      listen<{ key: string; error: string | null; cancelled: boolean }>("generate-done", (e) => {
        const item = running(e.payload.key);
        if (!item) return;
        const { error, cancelled } = e.payload;
        finish(item.id, { status: error ? "failed" : cancelled ? "cancelled" : "done", message: error ?? undefined });
      }),
    ];
    return () => un.forEach((p) => p.then((f) => f()));
  }, [patch, finish]);

  const cancel = useCallback(
    (id: string) => {
      const item = itemsRef.current.find((i) => i.id === id);
      if (!item) return;
      // A running one is marked cancelled when the backend says it stopped.
      if (item.status === "running") void api.cancelGenerate(item.key);
      else if (item.status === "checking") finish(id, { status: "cancelled" });
      else if (item.status === "waiting") patch(id, { status: "cancelled" });
    },
    [patch, finish],
  );

  const cancelAll = useCallback(() => {
    for (const i of itemsRef.current) if (active(i.status)) cancel(i.id);
  }, [cancel]);

  const clearFinished = useCallback(() => setItems((list) => list.filter((i) => active(i.status))), []);

  return { items, enqueue, cancel, cancelAll, clearFinished };
}
