import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import type { IndexStats, RootInfo } from "../types";
import { tildify } from "../lib/format";

interface Props {
  onClose: () => void;
  stats: IndexStats | null;
  indexing: boolean;
  version: number;
}

export function SourcesSheet({ onClose, stats, indexing, version }: Props) {
  const [roots, setRoots] = useState<RootInfo[]>([]);
  const [suggested, setSuggested] = useState<string[]>([]);
  const [err, setErr] = useState<string | null>(null);

  const refresh = () => {
    api.getRoots().then(setRoots).catch((e) => setErr(String(e)));
    api.suggestedRoots().then(setSuggested).catch(() => {});
  };
  useEffect(refresh, [version]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const add = async (path?: string) => {
    setErr(null);
    const p = path ?? ((await open({ directory: true, multiple: false, title: "Choose a session directory" })) as string | null);
    if (!p) return;
    await api.addRoot(p).catch((e) => setErr(String(e)));
    refresh();
  };

  const remove = async (path: string) => {
    await api.removeRoot(path).catch((e) => setErr(String(e)));
    refresh();
  };

  return (
    <div className="sheet-backdrop" onClick={onClose}>
      <div className="sheet" onClick={(e) => e.stopPropagation()}>
        <div className="sheet-head">
          <h2>Session sources</h2>
          <button className="ghost" onClick={onClose}>
            Done
          </button>
        </div>
        <p className="sheet-note">
          Folders are scanned recursively for <code>.jsonl</code> transcripts. The format (Claude Code, Codex or pi) is
          detected per file, and changes are picked up automatically while the app is open.
        </p>
        <ul className="roots">
          {roots.map((r) => (
            <li key={r.path} className={r.exists ? "" : "missing"}>
              <div>
                <div className="root-path">{tildify(r.path)}</div>
                <div className="root-meta">{r.exists ? `${r.sessions} sessions` : "Folder not found"}</div>
              </div>
              <button className="ghost danger" onClick={() => remove(r.path)}>
                Remove
              </button>
            </li>
          ))}
          {roots.length === 0 && <li className="none">No folders yet.</li>}
        </ul>
        {suggested.length > 0 && (
          <div className="suggest">
            <div className="suggest-head">Found on this Mac</div>
            {suggested.map((p) => (
              <button key={p} className="chip" onClick={() => add(p)}>
                + {tildify(p)}
              </button>
            ))}
          </div>
        )}
        {err && <div className="sheet-err">{err}</div>}
        <div className="sheet-foot">
          <button className="primary" onClick={() => add()}>
            Add folder…
          </button>
          <button className="ghost" disabled={indexing} onClick={() => api.reindex(true)}>
            {indexing ? "Indexing…" : "Rebuild index"}
          </button>
          {stats && (
            <span className="sheet-stats">
              {stats.scanned} files · last pass {stats.elapsed_ms} ms
            </span>
          )}
        </div>
      </div>
    </div>
  );
}
