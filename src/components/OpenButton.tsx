import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { OpenTarget, SessionRow } from "../types";
import { resumeCommand } from "../lib/format";
import { Menu } from "./Menu";
import { AppIcon, CopyIcon } from "./AppIcon";

const COPY = { id: "copy", label: "Copy resume command" };

/**
 * A split button: the left part runs the chosen action (also ⌘O), the chevron
 * picks which action that is (picking only changes the choice). Besides
 * copying the resume command, it offers each installed app that already knows
 * this session. A choice the session doesn't support falls back to copying.
 */
export function OpenButton({
  session,
  choice,
  onChoice,
  keyboard,
}: {
  session: SessionRow;
  choice: string;
  onChoice: (id: string) => void;
  /** False while a sheet covers the app, so ⌘O doesn't reach this. */
  keyboard: boolean;
}) {
  const [targets, setTargets] = useState<OpenTarget[]>([]);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    // The transcript is keyed by session, so this runs once per session.
    let live = true;
    api
      .openTargets(session.source, session.native_id)
      .then((t) => live && setTargets(t))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [session.source, session.native_id]);

  const options = [COPY, ...targets.map((t) => ({ id: t.id, label: t.label }))];
  const current = options.find((o) => o.id === choice) ?? COPY;
  const icon = (id: string) => (id === "copy" ? <CopyIcon /> : <AppIcon id={id} />);

  const run = async () => {
    const target = targets.find((t) => t.id === current.id);
    if (target) {
      await api.openInApp(target.url).catch((e) => api.log("error", `open ${target.url}: ${e}`));
      return;
    }
    await navigator.clipboard.writeText(resumeCommand(session));
    setCopied(true);
    setTimeout(() => setCopied(false), 1400);
  };

  const runRef = useRef(run);
  runRef.current = run;
  useEffect(() => {
    if (!keyboard) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "o") {
        e.preventDefault();
        void runRef.current();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [keyboard]);

  return (
    <div className="split">
      <button className="split-main" onClick={run} title="⌘O">
        {icon(current.id)}
        {current.id === "copy" && copied ? "Copied" : current.label}
      </button>
      <Menu
        className="split-menu"
        label="Resume or open with"
        value={current.id}
        options={options.map((o) => ({
          key: o.id,
          label: o.label,
          icon: icon(o.id),
          hint: o.id === current.id ? <kbd>⌘O</kbd> : undefined,
        }))}
        onChange={onChoice}
        trigger={<span className="caret">▾</span>}
      />
    </div>
  );
}
