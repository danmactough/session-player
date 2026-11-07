import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import type { IndexStats, LibraryError, RootInfo } from "../types";
import { basename, tildify } from "../lib/format";
import type { Theme } from "../lib/theme";

const THEMES: { key: Theme; label: string }[] = [
  { key: "system", label: "System" },
  { key: "light", label: "Light" },
  { key: "dark", label: "Dark" },
];

interface Props {
  onClose: () => void;
  stats: IndexStats | null;
  indexing: boolean;
  version: number;
  hidden: { project: string; count: number }[];
  onUnhide: (project: string) => void;
  countExits: boolean;
  onCountExits: (on: boolean) => void;
  theme: Theme;
  onTheme: (theme: Theme) => void;
  hideSubagents: boolean;
  onHideSubagents: (hide: boolean) => void;
  authorName: string;
  onAuthorName: (name: string) => void;
  showAnalyzeNotice: boolean;
  onShowAnalyzeNotice: (show: boolean) => void;
  autoCheckUpdates: boolean;
  onAutoCheckUpdates: (on: boolean) => void;
  checkingForUpdates: boolean;
  updateCheckResult: string | null;
  onCheckForUpdates: () => void;
}

export function SettingsSheet({
  onClose,
  stats,
  indexing,
  version,
  hidden,
  onUnhide,
  countExits,
  onCountExits,
  theme,
  onTheme,
  hideSubagents,
  onHideSubagents,
  authorName,
  onAuthorName,
  showAnalyzeNotice,
  onShowAnalyzeNotice,
  autoCheckUpdates,
  onAutoCheckUpdates,
  checkingForUpdates,
  updateCheckResult,
  onCheckForUpdates,
}: Props) {
  const [roots, setRoots] = useState<RootInfo[]>([]);
  const [suggested, setSuggested] = useState<string[]>([]);
  const [damaged, setDamaged] = useState<LibraryError[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [appVersion, setAppVersion] = useState<string | null>(null);

  const refresh = () => {
    api.getRoots().then(setRoots).catch((e) => setErr(String(e)));
    api.suggestedRoots().then(setSuggested).catch(() => {});
    api.libraryErrors().then(setDamaged).catch(() => {});
  };
  useEffect(refresh, [version]);
  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => {});
  }, []);

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
      <div className="sheet settings" role="dialog" aria-label="Settings" onClick={(e) => e.stopPropagation()}>
        <div className="sheet-head">
          <h2>Settings</h2>
          <button className="ghost" onClick={onClose}>
            Done
          </button>
        </div>
        <h3 className="settings-head">Appearance</h3>
        <div className="segmented" role="radiogroup" aria-label="Appearance">
          {THEMES.map((t) => (
            <button key={t.key} role="radio" aria-checked={theme === t.key} className={theme === t.key ? "on" : ""} onClick={() => onTheme(t.key)}>
              {t.label}
            </button>
          ))}
        </div>

        <h3 className="settings-head">Session sources</h3>
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

        <h3 className="settings-head">Session list</h3>
        <label className="setting-toggle">
          <input type="checkbox" checked={hideSubagents} onChange={(e) => onHideSubagents(e.target.checked)} />
          <span>
            Hide subagent sessions
            <span className="setting-note">
              Leaves subagent runs out of the session list, counts and search. They stay linked from the session that
              started them.
            </span>
          </span>
        </label>

        <h3 className="settings-head">Errors</h3>
        <label className="setting-toggle">
          <input type="checkbox" checked={countExits} onChange={(e) => onCountExits(e.target.checked)} />
          <span>
            Count failed shell commands as errors
            <span className="setting-note">
              Errors are tool calls that couldn&rsquo;t run (an edit that didn&rsquo;t apply, a timeout, a denied
              command) and harness or model errors. Turn this on to also count commands that ran and exited non-zero,
              such as failing tests. Exit 1 from <code>grep</code>, <code>rg</code>, <code>diff</code> and similar
              means &ldquo;no match&rdquo; or &ldquo;differs&rdquo; and never counts.
            </span>
          </span>
        </label>

        <h3 className="settings-head">Annotations</h3>
        <label className="setting-field">
          <span>Your name</span>
          <input value={authorName} onChange={(e) => onAuthorName(e.target.value)} />
          <span className="setting-note">Shown as the author of annotations you add. Annotations from Analyze show the model instead.</span>
        </label>
        <label className="setting-toggle">
          <input type="checkbox" checked={showAnalyzeNotice} onChange={(e) => onShowAnalyzeNotice(e.target.checked)} />
          <span>
            Show the notice before Analyze sends transcripts
            <span className="setting-note">
              Explains that Analyze sends transcripts to Anthropic, and asks before it does. Ticking &ldquo;Don&rsquo;t show
              this again&rdquo; on the notice turns this off.
            </span>
          </span>
        </label>
        <div className="setting-field">
          <span>Analyze log</span>
          <button className="ghost setting-button" onClick={() => void api.openAnalyzeLog()}>
            Open Log
          </button>
          <span className="setting-note">
            Each Analyze and report run, and each call to claude. When one fails, the log has claude&rsquo;s own error output.
          </span>
        </div>

        <h3 className="settings-head">Imported sessions</h3>
        <p className="sheet-note">
          To remove an imported session, open it and choose Remove… from the ⋯ menu at the top right of its transcript. Files in the library that can&rsquo;t
          be read are listed here.
        </p>
        <ul className="roots">
          {damaged.map((d) => (
            <li key={d.path} className="missing">
              <div>
                <div className="root-path">{basename(d.path)}</div>
                <div className="root-meta">{d.error}</div>
              </div>
              <button
                className="ghost danger"
                onClick={async () => {
                  await api.removeImported(d.path).catch((e) => setErr(String(e)));
                  refresh();
                }}
              >
                Remove
              </button>
            </li>
          ))}
          {damaged.length === 0 && <li className="none">No damaged files.</li>}
        </ul>

        <h3 className="settings-head">Hidden projects</h3>
        <ul className="roots">
          {hidden.map((x) => (
            <li key={x.project}>
              <div>
                <div className="root-path">{tildify(x.project)}</div>
                <div className="root-meta">
                  {x.count} {x.count === 1 ? "session" : "sessions"}
                </div>
              </div>
              <button className="ghost" onClick={() => onUnhide(x.project)}>
                Unhide
              </button>
            </li>
          ))}
          {hidden.length === 0 && <li className="none">None. Hover a project in the sidebar and click Hide.</li>}
        </ul>

        <h3 className="settings-head">Updates</h3>
        <label className="setting-toggle">
          <input type="checkbox" checked={autoCheckUpdates} onChange={(e) => onAutoCheckUpdates(e.target.checked)} />
          <span>
            Check for updates automatically
            <span className="setting-note">
              Asks GitHub for the latest version when the app opens and every 6 hours. Your sessions aren&rsquo;t sent,
              and nothing installs until you choose to.
            </span>
          </span>
        </label>
        <div className="sheet-foot">
          <button className="ghost" disabled={checkingForUpdates} onClick={onCheckForUpdates}>
            {checkingForUpdates ? "Checking…" : "Check now"}
          </button>
          {appVersion && <span className="sheet-stats">Version {appVersion}</span>}
        </div>
        {updateCheckResult && <p className="sheet-note">{updateCheckResult}</p>}
      </div>
    </div>
  );
}
