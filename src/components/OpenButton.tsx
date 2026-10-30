import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { OpenTarget, SessionRow, TerminalApp } from "../types";
import { resumeCommand } from "../lib/format";
import { Menu } from "./Menu";
import { AppIcon, CopyIcon } from "./AppIcon";

/** Which terminals are installed doesn't change while the app runs. */
let terminalApps: Promise<TerminalApp[]> | null = null;

const COPY = { id: "copy", label: "Copy resume command" };

/** The agent's own app, which is the choice until the user picks another. */
const HOME_APP: Partial<Record<string, string>> = { claude: "claude", codex: "chatgpt" };

/**
 * A split button: the left part runs the chosen action (also ⌘O), the chevron
 * picks which action that is (picking only changes the choice). Besides
 * copying the resume command, it offers each installed app that can open this
 * session, and a flyout of installed terminals that run the resume command. With no choice made, it uses the agent's own app; a choice
 * the session doesn't support falls back to that, then to copying.
 */
export function OpenButton({
  session,
  choice,
  onChoice,
  keyboard,
}: {
  session: SessionRow;
  choice: string | undefined;
  onChoice: (id: string) => void;
  /** False while a sheet covers the app, so ⌘O doesn't reach this. */
  keyboard: boolean;
}) {
  const [targets, setTargets] = useState<OpenTarget[]>([]);
  const [terminals, setTerminals] = useState<TerminalApp[]>([]);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    // The transcript is keyed by session, so this runs once per session.
    let live = true;
    api
      .openTargets(session.source, session.native_id, session.path)
      .then((t) => live && setTargets(t))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [session.source, session.native_id, session.path]);

  useEffect(() => {
    terminalApps ??= api.terminalApps().catch(() => []);
    let live = true;
    void terminalApps.then((t) => live && setTerminals(t));
    return () => {
      live = false;
    };
  }, []);

  // Terminal choices are "terminal:<app>", so one agent can use Ghostty and
  // another iTerm.
  const inTerminal = terminals.map((t) => ({ id: `terminal:${t.id}`, app: t.id, label: `Open in ${t.label}`, short: t.label }));
  const options = [COPY, ...targets.map((t) => ({ id: t.id, label: t.label }))];
  const current =
    [...options, ...inTerminal].find((o) => o.id === choice) ??
    options.find((o) => o.id === HOME_APP[session.source]) ??
    COPY;
  const icon = (id: string) => (id === "copy" ? <CopyIcon /> : <AppIcon id={id.replace(/^terminal:/, "")} />);
  const hint = (id: string) => (id === current.id ? <kbd>⌘O</kbd> : undefined);
  const terminalChoice = inTerminal.find((t) => t.id === current.id);

  const run = async () => {
    const target = targets.find((t) => t.id === current.id);
    if (target) {
      await api.openInApp(target.url).catch((e) => api.log("error", `open ${target.url}: ${e}`));
      return;
    }
    const terminal = inTerminal.find((t) => t.id === current.id);
    if (terminal) {
      await api
        .openInTerminal(terminal.app, session)
        .catch((e) => api.log("error", `open in ${terminal.app}: ${e}`));
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
        options={[
          ...options.map((o) => ({ key: o.id, label: o.label, icon: icon(o.id), hint: hint(o.id) })),
          ...(inTerminal.length > 0
            ? [
                {
                  key: "terminal",
                  label: "Open in Terminal",
                  icon: icon(terminalChoice?.id ?? "terminal"),
                  submenu: inTerminal.map((t) => ({ key: t.id, label: t.short, icon: icon(t.id), hint: hint(t.id) })),
                },
              ]
            : []),
        ]}
        onChange={onChoice}
        trigger={<span className="caret">▾</span>}
      />
    </div>
  );
}
