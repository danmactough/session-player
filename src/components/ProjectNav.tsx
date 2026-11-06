import { useEffect, useLayoutEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import type { Source } from "../types";
import { basename, dirname, tildify } from "../lib/format";
import { useScrollAnchor } from "../lib/useScrollAnchor";

export interface ProjectSummary {
  project: string;
  count: number;
  latest: number;
  sources: Set<Source>;
}

interface Props {
  projects: ProjectSummary[];
  total: number;
  /** Selected projects; empty means "All projects". */
  selected: string[];
  onSelect: (projects: string[]) => void;
  onHide: (project: string) => void;
  /** A new identity means the list was filtered or re-sorted: scroll to the top. */
  resetKey: unknown;
  /** Called with the full new order after a drag; omit to disable dragging. */
  onReorder?: (order: string[]) => void;
  /** Where to start scrolled to, when coming back to a list left earlier. */
  initialScroll?: number;
  onScrollTop?: (top: number) => void;
}

const SOURCES: Source[] = ["claude", "codex", "pi"];
// Pointer travel before a press becomes a drag, so clicks still select.
const DRAG_THRESHOLD = 4;

export function ProjectNav({ projects, total, selected, onSelect, onHide, onReorder, resetKey, initialScroll, onScrollTop }: Props) {
  const navRef = useRef<HTMLElement>(null);
  const [drag, setDrag] = useState<{ project: string; index: number } | null>(null);
  const dragRef = useRef<{ project: string; index: number } | null>(null);
  const press = useRef<{ project: string; y: number; dragging: boolean } | null>(null);
  // The click that follows a drag's pointerup must not select the project.
  const suppressClick = useRef(false);
  // Where a ⇧-click range starts: the last project clicked without ⇧.
  const rangeStart = useRef<string | null>(null);

  // Selection follows Finder: a click selects one project (clicking the only
  // selected one again goes back to All projects), ⌘-click adds or removes
  // one, ⇧-click selects the run from the last clicked project, and ⌘⇧-click
  // adds that run to the selection.
  const clickProject = (project: string, e: React.MouseEvent) => {
    // WebKit doesn't focus a clicked button, so focus the list itself; that
    // is what lets Esc (below) reach it.
    navRef.current?.focus({ preventScroll: true });
    const order = projects.map((x) => x.project);
    const toggle = e.metaKey || e.ctrlKey;
    const start = rangeStart.current && order.includes(rangeStart.current) ? rangeStart.current : null;
    if (e.shiftKey && start) {
      const [a, b] = [order.indexOf(start), order.indexOf(project)].sort((x, y) => x - y);
      const range = order.slice(a, b + 1);
      onSelect(toggle ? [...new Set([...selected, ...range])] : range);
      return;
    }
    rangeStart.current = project;
    if (toggle) {
      onSelect(selected.includes(project) ? selected.filter((x) => x !== project) : [...selected, project]);
    } else {
      onSelect(selected.length === 1 && selected[0] === project ? [] : [project]);
    }
  };

  useScrollAnchor(navRef, [projects], resetKey);
  useLayoutEffect(() => {
    if (initialScroll && navRef.current) navRef.current.scrollTop = initialScroll;
    // Only when the list first appears.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const dropIndex = (clientY: number) => {
    const items = navRef.current?.querySelectorAll<HTMLElement>(".proj-item[data-draggable]") ?? [];
    let i = 0;
    for (const it of items) {
      const r = it.getBoundingClientRect();
      if (clientY < r.top + r.height / 2) return i;
      i++;
    }
    return i;
  };

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>, project: string) => {
    if (!onReorder || e.button !== 0) return;
    press.current = { project, y: e.clientY, dragging: false };
    suppressClick.current = false;
  };

  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const p = press.current;
    if (!p) return;
    if (!p.dragging) {
      if (Math.abs(e.clientY - p.y) < DRAG_THRESHOLD) return;
      p.dragging = true;
      // Captured only once dragging, so a plain click still lands on the button.
      e.currentTarget.setPointerCapture(e.pointerId);
    }
    const nav = navRef.current;
    if (nav) {
      const r = nav.getBoundingClientRect();
      if (e.clientY < r.top + 28) nav.scrollTop -= 8;
      else if (e.clientY > r.bottom - 28) nav.scrollTop += 8;
    }
    dragRef.current = { project: p.project, index: dropIndex(e.clientY) };
    setDrag(dragRef.current);
  };

  const endDrag = (commit: boolean) => {
    const p = press.current;
    press.current = null;
    if (!p?.dragging) return;
    suppressClick.current = true;
    const d = dragRef.current;
    dragRef.current = null;
    setDrag(null);
    if (!commit || !d || !onReorder) return;
    const order = projects.map((x) => x.project);
    const from = order.indexOf(d.project);
    const to = d.index > from ? d.index - 1 : d.index;
    if (from < 0 || to === from) return;
    order.splice(from, 1);
    order.splice(to, 0, d.project);
    onReorder(order);
  };

  useEffect(() => {
    if (!drag) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") endDrag(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [drag !== null]);

  return (
    <nav
      className={`projects ${drag ? "dragging" : ""}`}
      ref={navRef}
      tabIndex={-1}
      onScroll={onScrollTop && ((e) => onScrollTop(e.currentTarget.scrollTop))}
      onKeyDown={(e) => {
        // Esc here (after clicking a project) goes back to All projects. It
        // only applies inside this list, so Esc keeps its meaning elsewhere;
        // during a drag it cancels the drag instead.
        if (e.key === "Escape" && !drag && selected.length) {
          e.preventDefault();
          onSelect([]);
        }
      }}
    >
      <button className={`proj ${selected.length === 0 ? "active" : ""}`} data-anchor="(all)" onClick={() => onSelect([])}>
        <span className="proj-name">All projects</span>
        <span className="proj-n">{total}</span>
      </button>
      {projects.map((x, i) => (
        <div
          key={x.project}
          className={[
            "proj-item",
            drag?.project === x.project ? "dragged" : "",
            drag?.index === i ? "drop-before" : "",
            drag && i === projects.length - 1 && drag.index === projects.length ? "drop-after" : "",
          ].join(" ")}
          data-anchor={x.project}
          data-draggable={onReorder ? "" : undefined}
          onPointerDown={(e) => onPointerDown(e, x.project)}
          onPointerMove={onPointerMove}
          onPointerUp={() => endDrag(true)}
          onPointerCancel={() => endDrag(false)}
        >
          <button
            className={`proj ${selected.includes(x.project) ? "active" : ""}`}
            aria-pressed={selected.includes(x.project)}
            // Keeps ⇧-click from selecting text across the list.
            onMouseDown={(e) => e.shiftKey && e.preventDefault()}
            onClick={(e) => {
              if (suppressClick.current) {
                suppressClick.current = false;
                return;
              }
              clickProject(x.project, e);
            }}
            title={tildify(x.project)}
          >
            <span className="proj-name">
              {basename(x.project)}
              <span className="proj-dir">{dirname(x.project)}</span>
            </span>
            <span className="proj-dots">
              {SOURCES.filter((s) => x.sources.has(s)).map((s) => (
                <span key={s} className={`sdot src-${s}`} />
              ))}
            </span>
            <span className="proj-n">{x.count}</span>
          </button>
          <button
            className="proj-hide"
            title="Hide this project and its sessions (unhide in Settings)"
            aria-label={`Hide ${basename(x.project)}`}
            onPointerDown={(e) => e.stopPropagation()}
            onClick={() => onHide(x.project)}
          >
            Hide
          </button>
        </div>
      ))}
    </nav>
  );
}
