import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { api, onServerEvent } from "../api";
import type { ReportSummary } from "../types";
import { ServerError } from "./transport";

const INTERRUPTED = "Interrupted: the server stopped while this report was being written.";

/** A report being written in the background, or one that failed. */
export interface ReportJob {
  job: string;
  sessions: number;
  progress?: { done: number; total: number };
  error?: string;
}

/**
 * Saved reports, newest first, and the reports being written. `onWritten`
 * is called with the id of each report as it is saved.
 */
export function useReports(onWritten: (id: string) => void) {
  const [list, setList] = useState<ReportSummary[]>([]);
  // Until the first list arrives, "no reports" isn't known yet.
  const [loaded, setLoaded] = useState(false);
  const [jobs, setJobs] = useState<ReportJob[]>([]);
  const jobsRef = useRef(jobs);
  useLayoutEffect(() => {
    jobsRef.current = jobs;
  });
  const written = useRef(onWritten);
  useEffect(() => {
    written.current = onWritten;
  });

  const reload = useCallback(() => {
    api
      .listReports()
      .then((l) => {
        setList(l);
        setLoaded(true);
      })
      .catch((e) => api.log("error", `list_reports failed: ${e}`));
  }, []);

  useEffect(() => {
    reload();
    const fail = (job: string, error: string) => setJobs((list) => list.map((j) => (j.job === job ? { ...j, error, progress: undefined } : j)));
    const followed = () => jobsRef.current.filter((j) => !j.error);
    const un = [
      onServerEvent("report-progress", ({ job, done, total }) => {
        setJobs((list) => list.map((j) => (j.job === job ? { ...j, progress: { done, total } } : j)));
      }),
      onServerEvent("report-done", ({ job, report, error }) => {
        // A failed job stays, with its error, until dismissed; others go.
        if (error) fail(job, error);
        else setJobs((list) => list.filter((j) => j.job !== job));
        if (report) {
          reload();
          written.current(report);
        }
      }),
      // Sent when the page missed events, such as after the server restarted.
      onServerEvent("snapshot", ({ running }) => {
        const live = new Set(running.map((j) => j.id));
        for (const j of followed()) if (!live.has(j.job)) fail(j.job, INTERRUPTED);
      }),
      // After a reconnect, ask about each followed job in case its end was missed.
      onServerEvent("stream-open", () => {
        for (const j of followed()) {
          api.job(j.job).then(
            (found) => {
              if (found.state === "running") return;
              if (found.state === "failed") fail(j.job, found.error ?? "The report failed.");
              else setJobs((list) => list.filter((x) => x.job !== j.job));
              reload();
            },
            (e) => e instanceof ServerError && e.code === "not_found" && fail(j.job, INTERRUPTED),
          );
        }
      }),
    ];
    return () => un.forEach((f) => f());
  }, [reload]);

  /** `sessions` are session files as list_sessions gives them. */
  const start = useCallback(async (sessions: string[], model?: string, focus?: string) => {
    const job = await api.generateReport(sessions, model, focus);
    setJobs((list) => [{ job, sessions: sessions.length }, ...list]);
  }, []);

  const cancel = useCallback((job: string) => void api.cancelReport(job), []);
  const dismiss = useCallback((job: string) => setJobs((list) => list.filter((j) => j.job !== job)), []);

  return { list, jobs, loaded, start, cancel, dismiss, reload };
}
