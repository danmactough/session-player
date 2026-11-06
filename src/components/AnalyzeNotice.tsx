import { useEffect, useRef, useState } from "react";

/**
 * Shown before Analyze sends anything, from a transcript's Analyze button or
 * the Analysis view's queue. "Don't show this again" takes effect only when
 * the user also clicks OK; Settings can bring the notice back.
 */
export function AnalyzeNotice({ count, onOk, onCancel }: { count: number; onOk: (dontShowAgain: boolean) => void; onCancel: () => void }) {
  const [dontShow, setDontShow] = useState(false);
  const okRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    okRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onCancel();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  return (
    <div className="sheet-backdrop" onClick={onCancel}>
      <div className="sheet notice-sheet" role="alertdialog" aria-labelledby="analyze-notice-title" onClick={(e) => e.stopPropagation()}>
        <h2 id="analyze-notice-title">Send {count === 1 ? "this transcript" : `${count} transcripts`} to Anthropic?</h2>
        <p className="sheet-note">
          Analyze sends each session&rsquo;s transcript, and the instructions the agent was given, to Anthropic through the
          claude command and your Claude Code sign-in, so Claude Haiku can find the outcome, components and annotations.
          Nothing is sent until you click OK.
        </p>
        <label className="setting-toggle notice-check">
          <input type="checkbox" checked={dontShow} onChange={(e) => setDontShow(e.target.checked)} />
          <span>Don&rsquo;t show this again</span>
        </label>
        <div className="notice-actions">
          <button className="ghost" onClick={onCancel}>
            Cancel
          </button>
          <button className="primary" ref={okRef} onClick={() => onOk(dontShow)}>
            OK
          </button>
        </div>
      </div>
    </div>
  );
}
