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

type Item = "run" | "export" | "reveal" | "path" | "remove" | string;

/**
 * The transcript's ⋯ menu, at its top right. Its first item resumes or opens
 * the session the chosen way (also ⌘O): copying the resume command, an
 * installed app that can open it, or a terminal that runs the resume command.
 * "Resume or open with" picks which way that is (picking only changes the
 * choice). With no choice made, it uses the agent's own app; a choice the
 * session doesn't support falls back to that, then to copying. An imported
 * session can't be resumed, so it has none of these.
 */
export function TranscriptMenu({
  session,
  choice,
  onChoice,
  keyboard,
  onExport,
  onRemove,
}: {
  session: SessionRow;
  choice: string | undefined;
  onChoice: (id: string) => void;
  /** False while a sheet covers the app, so ⌘O doesn't reach this. */
  keyboard: boolean;
  onExport: () => void;
  /** For an imported session: removes it from the library. */
  onRemove?: () => void;
}) {
  const [targets, setTargets] = useState<OpenTarget[]>([]);
  const [terminals, setTerminals] = useState<TerminalApp[]>([]);
  // A short confirmation beside the button, since the menu has closed by then.
  const [flash, setFlash] = useState<string | null>(null);
  const resumable = !session.from_bundle;

  useEffect(() => {
    if (!resumable) return;
    // The transcript is keyed by session, so this runs once per session.
    let live = true;
    api
      .openTargets(session.source, session.native_id, session.path)
      .then((t) => live && setTargets(t))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [resumable, session.source, session.native_id, session.path]);

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
  const inTerminal = terminals.map((t) => ({ id: `terminal:${t.id}`, app: t.id, label: `Open in ${t.label}` }));
  const options = [COPY, ...targets.map((t) => ({ id: t.id, label: t.label })), ...inTerminal];
  const current = options.find((o) => o.id === choice) ?? options.find((o) => o.id === HOME_APP[session.source]) ?? COPY;
  const icon = (id: string) => (id === "copy" ? <CopyIcon /> : <AppIcon id={id.replace(/^terminal:/, "")} />);

  const show = (text: string) => {
    setFlash(text);
    setTimeout(() => setFlash(null), 1400);
  };

  const run = async () => {
    if (!resumable) return;
    const target = targets.find((t) => t.id === current.id);
    if (target) {
      await api.openInApp(target.url).catch((e) => api.log("error", `open ${target.url}: ${e}`));
      return;
    }
    const terminal = inTerminal.find((t) => t.id === current.id);
    if (terminal) {
      await api.openInTerminal(terminal.app, session).catch((e) => api.log("error", `open in ${terminal.app}: ${e}`));
      return;
    }
    await navigator.clipboard.writeText(resumeCommand(session));
    show("Copied resume command");
  };

  const runRef = useRef(run);
  useEffect(() => {
    runRef.current = run;
  });
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

  const choose = async (key: Item) => {
    if (key === "run") void run();
    else if (key === "export") onExport();
    else if (key === "reveal") void api.reveal(session.path);
    else if (key === "path") {
      await navigator.clipboard.writeText(session.path);
      show("Copied path");
    } else if (key === "remove") onRemove?.();
    // Anything else is a way to resume or open, picked from the flyout.
    else onChoice(key);
  };

  return (
    <div className="t-menu">
      {flash && <span className="t-menu-flash">{flash}</span>}
      <Menu<Item>
        className="actions-menu"
        label="Session actions"
        title="Session actions"
        // The current way to resume or open is ticked in the flyout.
        value={current.id}
        options={[
          ...(resumable
            ? [
                { key: "run", label: current.label, icon: icon(current.id), hint: <kbd>⌘O</kbd> },
                {
                  key: "open-with",
                  label: "Resume or open with",
                  icon: <span />,
                  // The current choice carries ⌘O, as the item above does.
                  submenu: options.map((o) => ({ key: o.id, label: o.label, icon: icon(o.id), hint: o.id === current.id ? <kbd>⌘O</kbd> : undefined })),
                },
              ]
            : []),
          { key: "export", label: "Export…", icon: <span /> },
          { key: "reveal", label: "Reveal file", icon: <span /> },
          { key: "path", label: "Copy path", icon: <span /> },
          ...(onRemove ? [{ key: "remove", label: "Remove…", icon: <span /> }] : []),
        ]}
        onChange={(key) => void choose(key)}
        trigger={
          <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
            <circle cx="5" cy="12" r="1.8" />
            <circle cx="12" cy="12" r="1.8" />
            <circle cx="19" cy="12" r="1.8" />
          </svg>
        }
      />
    </div>
  );
}
