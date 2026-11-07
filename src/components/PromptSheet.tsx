import { useEffect, useRef, useState } from "react";

interface Props {
  title: string;
  /** The run button's label, such as "Analyze". */
  runLabel: string;
  /** The app's own text, for Restore. */
  builtIn: string;
  /** The user's saved default, or null when they use the built-in text. */
  saved: string | null;
  onRun: (text: string, setDefault: boolean) => void;
  onCancel: () => void;
}

/**
 * Edits what Analyze or a report should focus on, for one run. "Set as
 * default" also saves it, so the plain button uses it from then on. Only this
 * part of the prompt can change; the rules about evidence and the reply
 * format stay.
 */
export function PromptSheet({ title, runLabel, builtIn, saved, onRun, onCancel }: Props) {
  const [text, setText] = useState(saved ?? builtIn);
  const [setDefault, setSetDefault] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  const empty = !text.trim();
  useEffect(() => {
    ref.current?.focus();
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
      <div className="sheet prompt-sheet" role="dialog" aria-labelledby="prompt-sheet-title" onClick={(e) => e.stopPropagation()}>
        <h2 id="prompt-sheet-title">{title}</h2>
        <p className="sheet-note">
          What to look for. The rules about citing evidence and the shape of the reply stay the same.
          {saved !== null && " This is your saved default."}
        </p>
        <textarea
          ref={ref}
          className="prompt-text"
          value={text}
          rows={8}
          spellCheck
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && !empty) {
              e.preventDefault();
              onRun(text, setDefault);
            }
          }}
        />
        <div className="prompt-row">
          <label className="setting-toggle">
            <input type="checkbox" checked={setDefault} onChange={(e) => setSetDefault(e.target.checked)} />
            <span>Set as default</span>
          </label>
          <button className="clear-link" onClick={() => setText(builtIn)} disabled={text === builtIn} title="Put back the app's own text">
            Restore built-in
          </button>
        </div>
        <div className="notice-actions">
          <button className="ghost" onClick={onCancel}>
            Cancel
          </button>
          <button className="primary" onClick={() => onRun(text, setDefault)} disabled={empty} title="⌘↩">
            {runLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
