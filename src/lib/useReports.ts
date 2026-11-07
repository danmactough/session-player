import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import type { ReportSummary } from "../types";

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
    const un = [
      listen<{ job: string; done: number; total: number }>("report-progress", (e) => {
        const { job, done, total } = e.payload;
        setJobs((list) => list.map((j) => (j.job === job ? { ...j, progress: { done, total } } : j)));
      }),
      listen<{ job: string; report: string | null; error: string | null; cancelled: boolean }>("report-done", (e) => {
        const { job, report, error } = e.payload;
        // A failed job stays, with its error, until dismissed; others go.
        setJobs((list) => (error ? list.map((j) => (j.job === job ? { ...j, error, progress: undefined } : j)) : list.filter((j) => j.job !== job)));
        if (report) {
          reload();
          written.current(report);
        }
      }),
    ];
    return () => un.forEach((p) => p.then((f) => f()));
  }, [reload]);

  const start = useCallback(async (sessions: { key: string; title: string }[], model?: string) => {
    const job = await api.generateReport(sessions, model);
    setJobs((list) => [{ job, sessions: sessions.length }, ...list]);
  }, []);

  const cancel = useCallback((job: string) => void api.cancelReport(job), []);
  const dismiss = useCallback((job: string) => setJobs((list) => list.filter((j) => j.job !== job)), []);

  return { list, jobs, loaded, start, cancel, dismiss, reload };
}
