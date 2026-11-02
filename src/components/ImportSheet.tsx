import { useEffect } from "react";
import type { ImportReport, OnConflict } from "../types";

export type ImportState =
  | { status: "working" }
  | { status: "done"; file: string; report: ImportReport }
  | { status: "failed"; file: string; error: string };

const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/**
 * The outcome of importing a bundle. When sessions in it are already in the
 * library with different contents, nothing has been written yet and the user
 * picks whether the bundle's copies replace the library's or are skipped.
 */
export function ImportSheet({
  state,
  onResolve,
  onClose,
}: {
  state: ImportState;
  onResolve: (choice: OnConflict) => void;
  onClose: () => void;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const report = state.status === "done" ? state.report : null;
  const conflicts = report?.conflicts ?? [];
  return (
    <div className="sheet-backdrop" onClick={onClose}>
      <div className="sheet" role="dialog" aria-label="Import session" onClick={(e) => e.stopPropagation()}>
        <div className="sheet-head">
          <h2>Import session</h2>
          <button className="ghost" onClick={onClose}>
            {conflicts.length ? "Cancel" : "Done"}
          </button>
        </div>
        {state.status === "working" && <p className="sheet-note pulse">Importing…</p>}
        {state.status === "failed" && (
          <>
            <div className="sheet-err">Couldn&rsquo;t import {state.file}: {state.error}</div>
            <p className="sheet-note">Nothing was added to your library. Ask the sender to export the session again.</p>
          </>
        )}
        {report && conflicts.length > 0 && (
          <>
            <p className="sheet-note">
              {conflicts.length === 1 ? "This session is" : "These sessions are"} already in your library with different
              contents. Nothing has been imported yet.
            </p>
            <ul className="roots">
              {conflicts.map((c) => (
                <li key={c.id}>
                  <div>
                    <div>{c.incoming_title}</div>
                    <div className="root-meta">
                      {c.existing_title === c.incoming_title ? "Library copy differs" : `Library copy: ${c.existing_title}`}
                    </div>
                  </div>
                </li>
              ))}
            </ul>
            <div className="sheet-foot">
              <button className="primary" onClick={() => onResolve("replace")}>
                Replace library copies
              </button>
              <button className="ghost" onClick={() => onResolve("keep")}>
                Keep library copies
              </button>
            </div>
          </>
        )}
        {report && conflicts.length === 0 && (
          <p className="sheet-note">
            {[
              report.imported.length && `Imported ${count(report.imported.length, "session", "sessions")}.`,
              report.unchanged.length && `${count(report.unchanged.length, "session was", "sessions were")} already in your library.`,
              report.kept.length && `Kept your library's ${count(report.kept.length, "copy", "copies")} of ${count(report.kept.length, "session", "sessions")}.`,
              report.kept_notes > 0 &&
                `Kept ${count(report.kept_notes, "detail or annotation", "details and annotations")} you had changed since the last import.`,
              report.empty.length &&
                `${count(report.empty.length, "session was", "sessions were")} empty (no prompts, replies or tool calls) and ${report.empty.length === 1 ? "wasn't" : "weren't"} imported.`,
            ]
              .filter(Boolean)
              .join(" ")}{" "}
            Imported sessions are marked &ldquo;imported&rdquo; in the list. They can be read and exported again, but not
            resumed.
          </p>
        )}
      </div>
    </div>
  );
}
