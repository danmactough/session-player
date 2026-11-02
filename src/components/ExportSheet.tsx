import { useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import type { ExportPreview, SessionRow } from "../types";
import { basename, bytes } from "../lib/format";

const RELATION: Record<string, string> = { parent: "Parent", subagent: "Subagent", branch: "Branch" };
const CONTEXT_KIND = { instructions: "Instruction file", skill: "Skill", agent: "Agent definition", mcp: "MCP server instructions" };

/**
 * Shows exactly what an export bundle holds before it is written: the
 * sessions, which related sessions are left out, and every attachment. Saving
 * writes the bundle previewed here (the backend keeps it), so the file always
 * matches what the user reviewed.
 */
export function ExportSheet({ session, onClose }: { session: SessionRow; onClose: () => void }) {
  const [preview, setPreview] = useState<ExportPreview | null>(null);
  const [subagents, setSubagents] = useState<string[] | undefined>(undefined);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);

  const path = session.branch_of ?? session.path;
  useEffect(() => {
    let live = true;
    api
      .exportPreview(path, session.branch_leaf, subagents)
      .then((p) => live && (setPreview(p), setErr(null)))
      .catch((e) => live && setErr(String(e)))
      .finally(() => live && setBusy(false));
    return () => {
      live = false;
    };
  }, [path, session.branch_leaf, subagents]);

  const close = () => {
    if (!saved) void api.exportCancel().catch(() => {});
    onClose();
  };
  // Re-registered every render so Escape sees whether the bundle was saved.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const toggle = (sub: string, on: boolean) => {
    const current = preview?.subagents.filter((s) => s.included).map((s) => s.path) ?? [];
    setBusy(true);
    setSubagents(on ? [...current, sub] : current.filter((p) => p !== sub));
  };

  const saveBundle = async () => {
    if (!preview) return;
    setErr(null);
    const dest = await save({
      title: "Export session",
      defaultPath: preview.summary.file_name,
      filters: [{ name: "Session bundle", extensions: ["spsession"] }],
    });
    if (!dest) return;
    setBusy(true);
    try {
      setSaved(await api.exportSave(preview.token, dest));
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const summary = preview?.summary;
  const sessionTitle = new Map(summary?.sessions.map((s) => [s.id, s.title]));
  return (
    <div className="sheet-backdrop" onClick={close}>
      <div className="sheet export" role="dialog" aria-label="Export session" onClick={(e) => e.stopPropagation()}>
        <div className="sheet-head">
          <h2>Export session</h2>
          <button className="ghost" onClick={close}>
            {saved ? "Done" : "Cancel"}
          </button>
        </div>
        <p className="sheet-note">
          Saves a copy someone else can open in Session Player. Everything listed here goes into the file as it
          appears in the transcript: check prompts, tool output and attachments for secrets, personal details and
          local paths before you share it.
        </p>

        {summary && (
          <>
            <h3 className="settings-head">Included</h3>
            <ul className="roots">
              {summary.sessions.map((s) => (
                <li key={s.id}>
                  <div>
                    <div>{s.title}</div>
                    <div className="root-meta">
                      {s.is_subagent ? "Subagent · " : ""}
                      {s.prompts} {s.prompts === 1 ? "prompt" : "prompts"} · {s.events} events
                      {s.has_details && " · details"}
                      {s.annotations > 0 && ` · ${s.annotations} ${s.annotations === 1 ? "annotation" : "annotations"}`}
                    </div>
                  </div>
                </li>
              ))}
            </ul>

            {preview.subagents.length > 0 && (
              <>
                <h3 className="settings-head">Subagents</h3>
                {preview.subagents.map((s) => (
                  <label key={s.path} className="setting-toggle export-choice">
                    <input type="checkbox" checked={s.included} disabled={busy || !!saved} onChange={(e) => toggle(s.path, e.target.checked)} />
                    <span>{s.title}</span>
                  </label>
                ))}
              </>
            )}

            {summary.omitted.length > 0 && (
              <>
                <h3 className="settings-head">Left out</h3>
                <ul className="roots">
                  {summary.omitted.map((r) => (
                    <li key={`${r.relation}:${r.id}`}>
                      <div>
                        <div>{r.title || r.id}</div>
                        <div className="root-meta">{RELATION[r.relation]} · not in the file</div>
                      </div>
                    </li>
                  ))}
                </ul>
              </>
            )}

            {summary.sessions.some((s) => s.context.length > 0) && (
              <>
                <h3 className="settings-head">Context</h3>
                <p className="sheet-note">
                  The instruction files, skills and agent definitions the agent was given go into the file in full. They
                  can describe internal tools, people or systems.
                </p>
                <ul className="roots">
                  {summary.sessions.flatMap((s) =>
                    s.context.map((c) => (
                      <li key={`${s.id}:${c.kind}:${c.name}`}>
                        <div>
                          <div className="root-path">{c.name}</div>
                          <div className="root-meta">
                            {CONTEXT_KIND[c.kind]}
                            {summary.sessions.length > 1 && ` · ${s.title}`}
                          </div>
                        </div>
                      </li>
                    )),
                  )}
                </ul>
              </>
            )}

            <h3 className="settings-head">Attachments</h3>
            {summary.attachments.length === 0 ? (
              <p className="sheet-note">None.</p>
            ) : (
              <>
                <p className="sheet-note">Screenshots go into the file as images. Open each one in the transcript to review it.</p>
                <ul className="roots">
                  {summary.attachments.map((a) => (
                    <li key={a.id} className={a.status === "omitted" ? "omitted" : ""}>
                      <div>
                        <div className="root-path">{a.name ? basename(a.name) : a.kind === "image" ? "Image" : "File"}</div>
                        <div className="root-meta">
                          {a.status === "included"
                            ? `${a.kind === "image" ? (a.mime ?? "image") : "file contents"} · ${bytes(a.bytes ?? 0)}`
                            : `Not included: ${a.reason}`}
                          {summary.sessions.length > 1 && ` · ${sessionTitle.get(a.session)}`}
                        </div>
                      </div>
                    </li>
                  ))}
                </ul>
              </>
            )}
          </>
        )}

        {err && <div className="sheet-err">{err}</div>}
        {saved && <div className="sheet-saved">Saved to {saved}</div>}
        <div className="sheet-foot">
          {saved ? (
            <button className="primary" onClick={() => api.reveal(saved)}>
              Reveal in Finder
            </button>
          ) : (
            <button className="primary" disabled={!preview || busy} onClick={saveBundle}>
              Save…
            </button>
          )}
          {summary && <span className="sheet-stats">{bytes(summary.bytes)}</span>}
        </div>
      </div>
    </div>
  );
}
