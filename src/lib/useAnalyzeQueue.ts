import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { api, onServerEvent } from "../api";
import { ServerError } from "./transport";
import type { SessionRow } from "../types";
import { sessionKey } from "./notes";

/** "interrupted": the server lost the job (it restarted while the session was being analyzed). */
export type QueueStatus = "waiting" | "checking" | "running" | "done" | "skipped" | "failed" | "cancelled" | "interrupted";

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
  /** The server's job for this session, once it has started. */
  job?: string;
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
        const job = await api.generateNotes(next.key, next.path, next.branch, next.focus);
        setItems((list) => list.map((i) => (i.id === next.id && i.status === "running" ? { ...i, job } : i)));
      } catch (e) {
        finish(next.id, { status: "failed", message: String(e) });
      }
    })();
  }, [items, patch, finish]);

  useEffect(() => {
    const running = (key: string) => itemsRef.current.find((i) => i.key === key && i.status === "running");
    const followed = () => itemsRef.current.filter((i) => i.status === "running" && i.job);
    const un = [
      onServerEvent("generate-progress", ({ key, done, total }) => {
        const item = running(key);
        if (item) patch(item.id, { progress: { done, total } });
      }),
      onServerEvent("generate-done", ({ key, error, cancelled }) => {
        const item = running(key);
        if (!item) return;
        finish(item.id, { status: error ? "failed" : cancelled ? "cancelled" : "done", message: error ?? undefined });
      }),
      // Sent when the page missed events, such as after the server restarted.
      onServerEvent("snapshot", ({ running: jobs }) => {
        const live = new Set(jobs.map((j) => j.id));
        for (const i of followed()) if (!live.has(i.job!)) finish(i.id, { status: "interrupted" });
      }),
      // After a reconnect, ask about each followed job in case its end was missed.
      onServerEvent("stream-open", () => {
        for (const i of followed()) {
          api.job(i.job!).then(
            (j) => {
              if (j.state === "running") return;
              finish(i.id, { status: j.state === "failed" ? "failed" : j.state === "cancelled" ? "cancelled" : "done", message: j.error ?? undefined });
            },
            (e) => e instanceof ServerError && e.code === "not_found" && finish(i.id, { status: "interrupted" }),
          );
        }
      }),
    ];
    return () => un.forEach((f) => f());
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
