import type { ReportSummary } from "../types";
import type { ReportJob } from "../lib/useReports";
import { relTime } from "../lib/format";
import { modelName } from "../lib/reportMarkdown";
import { api } from "../api";

interface Props {
  reports: ReportSummary[];
  jobs: ReportJob[];
  /** A report waiting for its sessions to be analyzed first. */
  preparing: { sessions: number; analyzing: number } | null;
  onCancelPreparing: () => void;
  selected: string | null;
  onSelect: (id: string) => void;
  onCancel: (job: string) => void;
  onDismiss: (job: string) => void;
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** Insights' left column: reports being written, then saved reports. */
export function ReportList({ reports, jobs, preparing, onCancelPreparing, selected, onSelect, onCancel, onDismiss }: Props) {
  if (!reports.length && !jobs.length && !preparing) {
    return (
      <div className="empty">
        <p>No reports yet.</p>
        <p className="dim">Click Create above to start one.</p>
      </div>
    );
  }
  return (
    <div className="list-scroll report-list">
      {preparing && (
        <div className="report-row report-job">
          <div className="report-row-title">
            <span className="spinner" aria-hidden="true" />
            <span className="t">Analyzing {plural(preparing.analyzing, "session", "sessions")} first</span>
            <button className="queue-cancel" onClick={onCancelPreparing} aria-label="Cancel" title="Cancel">
              ×
            </button>
          </div>
          <div className="report-row-meta">Then a report on {plural(preparing.sessions, "session", "sessions")}</div>
        </div>
      )}
      {jobs.map((j) => (
        <div key={j.job} className={`report-row report-job ${j.error ? "failed" : ""}`}>
          <div className="report-row-title">
            {j.error ? (
              <span className="sdot err" aria-hidden="true" />
            ) : (
              <span className="spinner" aria-hidden="true" />
            )}
            <span className="t">{j.error ? "Couldn’t write the report" : `Writing a report on ${plural(j.sessions, "session", "sessions")}`}</span>
            <button
              className="queue-cancel"
              onClick={() => (j.error ? onDismiss(j.job) : onCancel(j.job))}
              aria-label={j.error ? "Dismiss" : "Cancel"}
              title={j.error ? "Dismiss" : "Cancel"}
            >
              ×
            </button>
          </div>
          {j.error && (
            <button className="pick-link report-job-log" onClick={() => void api.openAnalyzeLog()}>
              Open log
            </button>
          )}
          <div className="report-row-meta">
            {j.error ?? (j.progress && j.progress.total > 1 ? `Part ${Math.min(j.progress.done + 1, j.progress.total)} of ${j.progress.total}` : "Reading the notes…")}
          </div>
        </div>
      ))}
      {reports.map((r) => (
        <button key={r.id} className={`report-row ${r.id === selected ? "active" : ""}`} onClick={() => onSelect(r.id)}>
          <span className="report-row-title">
            <span className="t">{r.title}</span>
          </span>
          <span className="report-row-meta">
            {relTime(r.created_at)} · {plural(r.sessions, "session", "sessions")} · {plural(r.actions, "action", "actions")} · {modelName(r.model)}
          </span>
        </button>
      ))}
    </div>
  );
}
