import { useEffect, useMemo, useRef, useState } from "react";
import { ask, save } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import type { ActionResponse, ActionStatus, CitedAnnotation, Evidence, LatestVerdict, Report, UserImpact } from "../types";
import { KIND_LABEL, OUTCOME_LABEL } from "../lib/notes";
import { fullDate } from "../lib/format";
import { RemoteImages } from "../lib/remoteImages";
import { CATEGORY_LABEL, PRIORITY_LABEL, STATUS_LABEL, USER_IMPACT_LABEL, VERDICT_LABEL, modelName, reportMarkdown, verdictOrder } from "../lib/reportMarkdown";
import { Markdown } from "./Markdown";
import { Menu } from "./Menu";

interface Props {
  id: string;
  /** Opens a session in Transcripts; absent for one no longer in the list. */
  onOpenSession: (key: string) => void;
  canOpenSession: (key: string) => boolean;
  onDeleted: () => void;
  onOpenReport: (id: string) => void;
}

/** One saved report, filling the space right of Insights' report list.
 * Keyed by the report's id, so opening another starts fresh. */
export function ReportView({ id, onOpenSession, canOpenSession, onDeleted, onOpenReport }: Props) {
  const [report, setReport] = useState<Report | null | undefined>(undefined);
  const [cited, setCited] = useState<Map<string, CitedAnnotation>>(() => new Map());
  const [notice, setNotice] = useState<string | null>(null);
  const [showFocus, setShowFocus] = useState(false);
  const [responses, setResponses] = useState<Map<number, ActionResponse>>(() => new Map());
  const [verdicts, setVerdicts] = useState<Map<number, LatestVerdict>>(() => new Map());

  useEffect(() => {
    let live = true;
    Promise.all([api.getReport(id), api.reportAnnotations(id), api.actionResponses(id), api.actionVerdicts(id)])
      .then(([r, list, savedResponses, latest]) => {
        if (!live) return;
        setReport(r);
        setCited(new Map(list.map((c) => [`${c.annotation.id}@${c.annotation.version}`, c])));
        setResponses(new Map(savedResponses.map((x) => [x.action_index, x])));
        setVerdicts(new Map(latest.map((v) => [v.action_index, v])));
      })
      .catch((e) => live && setNotice(String(e)));
    return () => {
      live = false;
    };
  }, [id]);

  const markdown = useMemo(() => (report ? reportMarkdown(report, cited, { responses, verdicts }) : ""), [report, cited, responses, verdicts]);

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
          {/* The header's empty space drags the window; its text is selectable. */}
          <h1>{report.title}</h1>
          <p className="dim">
            {fullDate(report.created_at)} · {report.sessions.length} {report.sessions.length === 1 ? "session" : "sessions"} ·{" "}
            {modelName(report.model)}
            {report.focus && (
              <>
                {" · "}
                <button className="pick-link report-focus" onClick={() => setShowFocus((v) => !v)} aria-expanded={showFocus}>
                  Custom prompt
                </button>
              </>
            )}
          </p>
          {/* Top right, like the transcript's ⋯ menu. */}
          <Menu<"copy" | "save" | "delete" | "none">
            className="actions-menu report-menu"
            label="Report actions"
            title="Report actions"
            value="none"
            options={[
              { key: "copy", label: "Copy as Markdown" },
              { key: "save", label: "Save as .md…" },
              { key: "delete", label: "Delete…" },
            ]}
            onChange={(a) => {
              if (a === "copy") void copy();
              else if (a === "save") void saveFile();
              else if (a === "delete") void remove();
            }}
            trigger={
              <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
                <circle cx="5" cy="12" r="1.8" />
                <circle cx="12" cy="12" r="1.8" />
                <circle cx="19" cy="12" r="1.8" />
              </svg>
            }
          />
          {showFocus && report.focus && <p className="report-focus-text">{report.focus}</p>}
          {notice && <p className="report-notice">{notice}</p>}
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
                {a.follows_up && (
                  <p className="report-follows">
                    Follows up on{" "}
                    <button className="pick-link" onClick={() => onOpenReport(a.follows_up!.report_id)}>
                      {a.follows_up.title}
                    </button>
                  </p>
                )}
                <Markdown text={a.change} className="report-change" />
                {a.success_check && (
                  <p className="report-success">
                    <strong>Success check:</strong> {a.success_check}
                  </p>
                )}
                {chips(a.evidence)}
                {verdicts.has(i) && (
                  <p className="report-verdict">
                    <span className={`verdict verdict-${verdicts.get(i)!.verdict}`}>{VERDICT_LABEL[verdicts.get(i)!.verdict]}</span>, per{" "}
                    <button className="pick-link" onClick={() => onOpenReport(verdicts.get(i)!.report_id)}>
                      {verdicts.get(i)!.report_title}
                    </button>
                  </p>
                )}
                <ActionResponseControls
                  reportId={report.id}
                  index={i}
                  saved={responses.get(i)}
                  onError={(e) => setNotice(`Couldn’t save your response: ${e}`)}
                  onSaved={(r) =>
                    setResponses((m) => {
                      const next = new Map(m);
                      if (r) next.set(i, r);
                      else next.delete(i);
                      return next;
                    })
                  }
                />
              </article>
            ))}
          </section>
        )}

        {report.impact.length > 0 && (
          <section className="report-section">
            <h2>Did earlier actions work?</h2>
            {[...report.impact]
              .sort((a, b) => verdictOrder(a.verdict) - verdictOrder(b.verdict))
              .map((v) => (
                <article key={`${v.action.report_id}:${v.action.action_index}`} className="report-action report-impact">
                  <div className="report-action-head">
                    <span className={`verdict verdict-${v.verdict}`}>{VERDICT_LABEL[v.verdict]}</span>
                  </div>
                  <h3>
                    <button className="pick-link" onClick={() => onOpenReport(v.action.report_id)}>
                      {v.action_title}
                    </button>
                  </h3>
                  <p>{v.reasoning}</p>
                  {chips(v.evidence)}
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
                <button className="report-session" onClick={() => onOpenSession(s.key)} disabled={!canOpenSession(s.key)} title={canOpenSession(s.key) ? "Open in Transcripts" : "This session is no longer in the list"}>
                  <span className="t">{s.title}</span>
                  <span className={`outcome outcome-${s.outcome}`}>{OUTCOME_LABEL[s.outcome]}</span>
                  <span className="dim">
                    {s.annotations.length} {s.annotations.length === 1 ? "annotation" : "annotations"}
                  </span>
                </button>
              </li>
            ))}
          </ul>
          {report.rejected > 0 && (
            <p className="dim report-rejected">
              Left out {report.rejected} {report.rejected === 1 ? "item that failed" : "items that failed"} the checks on what it cites.
            </p>
          )}
        </section>
      </div>
    </RemoteImages.Provider>
  );
}

const STATUSES: ActionStatus[] = ["done", "in_progress", "declined", "considering"];
const IMPACTS: UserImpact[] = ["helped", "did_not_help"];

/** How long typing pauses before the note is saved. */
const NOTE_SAVE_DELAY = 600;

/** The developer's status and note for one action. The note saves as you
 * type, and once more if the view closes mid-edit (switching reports
 * unmounts it), so nothing typed is lost. Each save sends the whole
 * response, so saves run in order. */
function ActionResponseControls({
  reportId,
  index,
  saved,
  onSaved,
  onError,
}: {
  reportId: string;
  index: number;
  saved: ActionResponse | undefined;
  onSaved: (r: ActionResponse | null) => void;
  onError: (e: unknown) => void;
}) {
  const [note, setNote] = useState(saved?.note ?? "");
  const status = saved?.status ?? null;
  const impact = saved?.user_impact ?? null;
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  // Read by the delayed and unmount saves, which outlive the render that scheduled them.
  const latest = useRef({ note, status, impact, savedNote: saved?.note ?? "" });
  useEffect(() => {
    latest.current = { note, status, impact, savedNote: saved?.note ?? "" };
  });

  const persist = (next: { note: string; status: ActionStatus | null; user_impact: UserImpact | null }) => {
    clearTimeout(timer.current);
    latest.current.savedNote = next.note.trim();
    queue.current = queue.current
      .then(() => api.setActionResponse({ report_id: reportId, action_index: index, ...next }))
      .then(onSaved)
      .catch(onError);
  };
  const persistRef = useRef(persist);
  useEffect(() => {
    persistRef.current = persist;
  });

  useEffect(
    () => () => {
      const { note, status, impact, savedNote } = latest.current;
      if (note.trim() !== savedNote) persistRef.current({ note, status, user_impact: impact });
    },
    [],
  );

  const typed = (value: string) => {
    setNote(value);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      const { note, status, impact, savedNote } = latest.current;
      if (note.trim() !== savedNote) persistRef.current({ note, status, user_impact: impact });
    }, NOTE_SAVE_DELAY);
  };

  return (
    <div className="report-response">
      <div className="report-response-row" role="group" aria-label="Status">
        {STATUSES.map((s) => (
          <button
            key={s}
            className="report-toggle"
            aria-pressed={status === s}
            onClick={() => persist({ note, status: status === s ? null : s, user_impact: s === "done" && status === s ? null : impact })}
          >
            {STATUS_LABEL[s]}
          </button>
        ))}
        {saved?.done_at != null && <span className="dim">Done on {fullDate(saved.done_at)}</span>}
      </div>
      {status === "done" && (
        <div className="report-response-row" role="group" aria-label="Did it help?">
          {IMPACTS.map((x) => (
            <button key={x} className="report-toggle" aria-pressed={impact === x} onClick={() => persist({ note, status, user_impact: impact === x ? null : x })}>
              {USER_IMPACT_LABEL[x]}
            </button>
          ))}
        </div>
      )}
      <div className="report-note">
        <textarea rows={2} placeholder="Note" aria-label="Note" value={note} onChange={(e) => typed(e.target.value)} />
        {note && (
          <button
            className="report-toggle report-note-clear"
            onClick={() => {
              setNote("");
              persist({ note: "", status, user_impact: impact });
            }}
          >
            Clear
          </button>
        )}
      </div>
    </div>
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
      <button className={`evidence-chip cites-${e.type}`} onClick={() => setOpen((o) => !o)} aria-expanded={open} title={session?.title}>
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
            Open in Transcripts
          </button>
        </span>
      )}
    </span>
  );
}
