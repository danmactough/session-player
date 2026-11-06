import { useEffect, useRef, useState } from "react";
import type { QueueItem } from "../lib/useAnalyzeQueue";

interface Props {
  items: QueueItem[];
  onOpen: (path: string) => void;
  onCancel: (id: string) => void;
  onCancelAll: () => void;
  onClear: () => void;
}

function statusText(i: QueueItem): string {
  switch (i.status) {
    case "waiting":
      return "Waiting";
    case "checking":
      return "Starting";
    case "running":
      return i.progress && i.progress.total > 1 ? `Analyzing part ${Math.min(i.progress.done + 1, i.progress.total)} of ${i.progress.total}` : "Analyzing";
    case "done":
      return "Done";
    case "skipped":
      return i.message ?? "Skipped";
    case "failed":
      return `Failed: ${i.message ?? "unknown error"}`;
    case "cancelled":
      return "Cancelled";
  }
}

const isActive = (i: QueueItem) => i.status === "waiting" || i.status === "checking" || i.status === "running";

/**
 * The batch Analyze queue, kept out of the way in the sidebar's footer: a
 * spinner while it runs, a tally once it's done. Clicking it shows each
 * session's result.
 */
export function QueueStatus({ items, onOpen, onCancel, onCancelAll, onClear }: Props) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const count = (status: QueueItem["status"]) => items.filter((i) => i.status === status).length;
  const active = items.filter(isActive).length;
  const finished = items.length - active;
  const failed = count("failed");
  const busy = active > 0;
  const summary = [
    [count("done"), "analyzed"],
    [count("skipped"), "skipped"],
    [failed, "failed"],
    [count("cancelled"), "cancelled"],
  ]
    .filter(([n]) => n)
    .map(([n, what]) => `${n} ${what}`)
    .join(" · ");

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [open]);

  return (
    <div className="queue-status" ref={ref}>
      <button
        className={`ghost queue-button ${failed ? "has-failed" : ""}`}
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        title={busy ? `Analyzing ${Math.min(finished + 1, items.length)} of ${items.length}` : summary}
      >
        {busy ? (
          <>
            <span className="spinner" aria-hidden="true" />
            {finished}/{items.length}
          </>
        ) : (
          <>
            <span className={`queue-dot ${failed ? "err" : "ok"}`} aria-hidden="true" />
            {failed ? `${failed} failed` : "Analyzed"}
          </>
        )}
      </button>
      {open && (
        <div className="queue-pop" role="dialog" aria-label="Analyze queue">
          <header className="queue-head">
            <span className="queue-title">{busy ? `Analyzing ${Math.min(finished + 1, items.length)} of ${items.length}` : summary}</span>
            {busy ? (
              <button className="clear-link" onClick={onCancelAll}>
                Cancel all
              </button>
            ) : (
              <button
                className="clear-link"
                onClick={() => {
                  setOpen(false);
                  onClear();
                }}
              >
                Clear
              </button>
            )}
          </header>
          <ul className="queue-list">
            {items.map((i) => (
              <li key={i.id} className={`queue-item queue-${i.status}`}>
                <button className="queue-open" onClick={() => onOpen(i.id)} title={`Select ${i.title}`}>
                  <span className="queue-name">{i.title}</span>
                  <span className="queue-state" title={statusText(i)}>
                    {statusText(i)}
                  </span>
                </button>
                {isActive(i) && (
                  <button className="queue-cancel" onClick={() => onCancel(i.id)} aria-label={`Cancel ${i.title}`} title="Cancel">
                    ×
                  </button>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
