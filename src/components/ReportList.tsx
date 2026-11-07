import type { ReportSummary } from "../types";
import type { ReportJob } from "../lib/useReports";
import { relTime } from "../lib/format";
import { modelName } from "../lib/reportMarkdown";

interface Props {
  reports: ReportSummary[];
  jobs: ReportJob[];
  selected: string | null;
  onSelect: (id: string) => void;
  onCancel: (job: string) => void;
  onDismiss: (job: string) => void;
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** The middle column of the Analysis view's Reports tab: reports being written, then saved reports, newest first. */
export function ReportList({ reports, jobs, selected, onSelect, onCancel, onDismiss }: Props) {
  if (!reports.length && !jobs.length) {
    return (
      <div className="empty">
        <p>No reports yet.</p>
        <p className="dim">Switch to Statistics, select sessions in the list, and click Report.</p>
      </div>
    );
  }
  return (
    <div className="list-scroll report-list">
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
