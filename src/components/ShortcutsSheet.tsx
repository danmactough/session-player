import { useEffect } from "react";

const GROUPS: { title: string; keys: { keys: string[][]; what: string }[] }[] = [
  {
    title: "General",
    keys: [
      { keys: [["j"], ["↓"]], what: "Next session" },
      { keys: [["k"], ["↑"]], what: "Previous session" },
      { keys: [["?"]], what: "Show this list" },
      { keys: [["⌘", ","]], what: "Settings" },
      { keys: [["⌘", "⇧", "I"]], what: "Import a shared session" },
    ],
  },
  {
    title: "Search",
    keys: [
      { keys: [["⌘", "K"], ["/"]], what: "Search sessions" },
      { keys: [["Esc"]], what: "Clear the search, then leave the box" },
    ],
  },
  {
    title: "Transcript",
    keys: [
      { keys: [["Space"], ["⌥", "↓"]], what: "Page down" },
      { keys: [["⇧", "Space"], ["⌥", "↑"]], what: "Page up" },
      { keys: [["⌘", "↑"]], what: "Top" },
      { keys: [["⌘", "↓"]], what: "Bottom" },
      { keys: [["⌘", "]"]], what: "Next prompt" },
      { keys: [["⌘", "["]], what: "Previous prompt" },
      { keys: [["⌘", "⇧", "]"]], what: "Next response" },
      { keys: [["⌘", "⇧", "["]], what: "Previous response" },
      { keys: [["⌘", "O"]], what: "Copy resume command, or open in the chosen app" },
      { keys: [["⌘", "F"]], what: "Find in session" },
      { keys: [["Enter"]], what: "Next match" },
      { keys: [["⇧", "Enter"]], what: "Previous match" },
      { keys: [["Esc"]], what: "Close find" },
    ],
  },
  {
    title: "Analysis",
    keys: [
      { keys: [["⌘", "click"]], what: "Add or remove a session from the selection" },
      { keys: [["⇧", "click"]], what: "Select a run of sessions" },
      { keys: [["⌘", "A"]], what: "Select every session in the list" },
      { keys: [["Esc"]], what: "Clear the selection (or click an empty part of the list)" },
    ],
  },
  {
    title: "Agents",
    keys: [{ keys: [["⌥", "click"]], what: "Show only that agent" }],
  },
  {
    title: "Projects",
    keys: [
      { keys: [["⌘", "click"]], what: "Add or remove a project" },
      { keys: [["⇧", "click"]], what: "Select a run of projects" },
      { keys: [["Esc"]], what: "Back to All projects (in the list), clear the filter, or cancel a drag" },
    ],
  },
];

export function ShortcutsSheet({ onClose }: { onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" || e.key === "?") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="sheet-backdrop" onClick={onClose}>
      <div className="sheet shortcuts" role="dialog" aria-label="Keyboard shortcuts" onClick={(e) => e.stopPropagation()}>
        <div className="sheet-head">
          <h2>Keyboard shortcuts</h2>
          <button className="ghost" onClick={onClose}>
            Done
          </button>
        </div>
        <div className="shortcut-groups">
          {GROUPS.map((g) => (
            <section key={g.title}>
              <h3>{g.title}</h3>
              <dl>
                {g.keys.map((k) => (
                  <div key={k.what} className="shortcut">
                    <dt>
                      {k.keys.map((combo, i) => (
                        <span key={i} className="combo">
                          {i > 0 && <span className="or">or</span>}
                          {combo.map((key) => (
                            <kbd key={key}>{key}</kbd>
                          ))}
                        </span>
                      ))}
                    </dt>
                    <dd>{k.what}</dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
        </div>
      </div>
    </div>
  );
}
