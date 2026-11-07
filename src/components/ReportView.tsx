import { useEffect, useMemo, useState } from "react";
import { ask, save } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import type { CitedAnnotation, Evidence, Report } from "../types";
import { KIND_LABEL, OUTCOME_LABEL } from "../lib/notes";
import { fullDate } from "../lib/format";
import { RemoteImages } from "../lib/remoteImages";
import { CATEGORY_LABEL, PRIORITY_LABEL, modelName, reportMarkdown } from "../lib/reportMarkdown";
import { Markdown } from "./Markdown";

interface Props {
  id: string;
  /** Opens a session in the Sessions view; absent for one no longer in the list. */
  onOpenSession: (key: string) => void;
  canOpenSession: (key: string) => boolean;
  onDeleted: () => void;
}

/** The Analysis view's right-hand pane on the Reports tab: one saved report.
 * Keyed by the report's id, so opening another starts fresh. */
export function ReportView({ id, onOpenSession, canOpenSession, onDeleted }: Props) {
  const [report, setReport] = useState<Report | null | undefined>(undefined);
  const [cited, setCited] = useState<Map<string, CitedAnnotation>>(() => new Map());
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    Promise.all([api.getReport(id), api.reportAnnotations(id)])
      .then(([r, list]) => {
        if (!live) return;
        setReport(r);
        setCited(new Map(list.map((c) => [`${c.annotation.id}@${c.annotation.version}`, c])));
      })
      .catch((e) => live && setNotice(String(e)));
    return () => {
      live = false;
    };
  }, [id]);

  const markdown = useMemo(() => (report ? reportMarkdown(report, cited) : ""), [report, cited]);

  if (report === undefined) return <div className="report-view empty pulse">{notice ?? "Loading report…"}</div>;
  if (report === null) return <div className="report-view empty">This report no longer exists.</div>;

  const copy = async () => {
    await navigator.clipboard.writeText(markdown);
    setNotice("Copied as Markdown.");
  };
  const saveFile = async () => {
    const name = report.title.replace(/[/\\:*?"<>|]+/g, " ").replace(/\s+/g, " ").trim().slice(0, 80) || "Report";
    const dest = await save({ title: "Save report", defaultPath: `${name}.md`, filters: [{ name: "Markdown", extensions: ["md"] }] });
    if (!dest) return;
    try {
      setNotice(`Saved to ${await api.saveMarkdown(dest, markdown)}`);
    } catch (e) {
      setNotice(`Couldn’t save: ${e}`);
    }
  };
  const remove = async () => {
    const ok = await ask("The report is deleted for good. Annotations it kept as they were when it was written are released.", {
      title: `Delete “${report.title}”?`,
      kind: "warning",
      okLabel: "Delete",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    await api.deleteReport(report.id);
    onDeleted();
  };

  const chips = (evidence: Evidence[]) => (
    <div className="evidence">
      {evidence.map((e, i) => (
        <EvidenceChip key={i} e={e} report={report} cited={cited} onOpenSession={onOpenSession} canOpenSession={canOpenSession} />
      ))}
    </div>
  );

  return (
    // The model wrote this text; it never loads anything from the web.
    <RemoteImages.Provider value={false}>
      <div className="report-view">
        <header className="report-head" data-tauri-drag-region>
          <h1 data-tauri-drag-region>{report.title}</h1>
          <p className="dim" data-tauri-drag-region>
            {fullDate(report.created_at)} · {report.sessions.length} {report.sessions.length === 1 ? "session" : "sessions"} ·{" "}
            {modelName(report.model)}
          </p>
          <div className="report-tools">
            <button className="ghost" onClick={() => void copy()}>
              Copy as Markdown
            </button>
            <button className="ghost" onClick={() => void saveFile()}>
              Save as .md…
            </button>
            <button className="ghost danger" onClick={() => void remove()}>
              Delete…
            </button>
            {notice && <span className="report-notice">{notice}</span>}
          </div>
        </header>

        {report.summary && <p className="report-summary">{report.summary}</p>}

        {report.actions.length > 0 && (
          <section className="report-section">
            <h2>Recommended actions</h2>
            {report.actions.map((a, i) => (
              <article key={i} className={`report-action priority-${a.priority}`}>
                <div className="report-action-head">
                  <span className={`priority priority-${a.priority}`}>{PRIORITY_LABEL[a.priority]}</span>
                  <span className="category">{CATEGORY_LABEL[a.category]}</span>
                </div>
                <h3>{a.title}</h3>
                <Markdown text={a.change} className="report-change" />
                {chips(a.evidence)}
              </article>
            ))}
          </section>
        )}

        {report.takeaways.length > 0 && (
          <section className="report-section">
            <h2>Key takeaways</h2>
            <ol className="report-takeaways">
              {report.takeaways.map((t, i) => (
                <li key={i}>
                  <p>{t.text}</p>
                  {chips(t.evidence)}
                </li>
              ))}
            </ol>
          </section>
        )}

        <section className="report-section">
          <h2>Sessions</h2>
          <ul className="report-sessions">
            {report.sessions.map((s) => (
              <li key={s.key}>
                <button className="report-session" onClick={() => onOpenSession(s.key)} disabled={!canOpenSession(s.key)} title={canOpenSession(s.key) ? "Open in Sessions" : "This session is no longer in the list"}>
                  <span className="t">{s.title}</span>
                  {s.outcome !== "unknown" && <span className={`outcome outcome-${s.outcome}`}>{OUTCOME_LABEL[s.outcome]}</span>}
                  <span className="dim">
                    {s.annotations.length} {s.annotations.length === 1 ? "annotation" : "annotations"}
                  </span>
                </button>
              </li>
            ))}
          </ul>
          {report.rejected > 0 && (
            <p className="dim report-rejected">
              Left out {report.rejected} {report.rejected === 1 ? "takeaway or action that cited" : "takeaways or actions that cited"} nothing the report was given.
            </p>
          )}
        </section>
      </div>
    </RemoteImages.Provider>
  );
}

/** A citation: click to see the note as the report saw it, and to open its session. */
function EvidenceChip({
  e,
  report,
  cited,
  onOpenSession,
  canOpenSession,
}: {
  e: Evidence;
  report: Report;
  cited: Map<string, CitedAnnotation>;
  onOpenSession: (key: string) => void;
  canOpenSession: (key: string) => boolean;
}) {
  const [open, setOpen] = useState(false);
  const session = report.sessions.find((s) => s.key === e.key);
  const c = e.type === "annotation" ? cited.get(`${e.id}@${e.version}`) : undefined;
  const label = e.type === "annotation" ? (c ? KIND_LABEL[c.annotation.kind] : "Annotation") : `${OUTCOME_LABEL[session?.outcome ?? "unknown"]} session`;
  const since = c && (c.current_version === null ? "deleted since" : c.current_version !== c.annotation.version ? "edited since" : null);
  return (
    <span className={`evidence-item ${open ? "open" : ""}`}>
      <button className={`evidence-chip ${e.type}`} onClick={() => setOpen((o) => !o)} aria-expanded={open} title={session?.title}>
        {label}
        {since && <span className="since"> · {since}</span>}
      </button>
      {open && (
        <span className="evidence-detail">
          <span className="evidence-session">{session?.title ?? "A session"}</span>
          <span className="evidence-note">
            {e.type === "annotation" ? (c?.annotation.note ?? "This annotation can’t be found.") : (session?.outcome_note ?? "No explanation recorded.")}
          </span>
          {since && <span className="dim">This is the note as the report saw it; it has been {since.replace(" since", "")} since.</span>}
          <button className="pick-link" onClick={() => onOpenSession(e.key)} disabled={!canOpenSession(e.key)}>
            Open in Sessions
          </button>
        </span>
      )}
    </span>
  );
}
