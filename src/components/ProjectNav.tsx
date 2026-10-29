import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
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
  active: string | null;
  onSelect: (project: string | null) => void;
  onHide: (project: string) => void;
  /** Called with the full new order after a drag; omit to disable dragging. */
  onReorder?: (order: string[]) => void;
}

const SOURCES: Source[] = ["claude", "codex", "pi"];
// Pointer travel before a press becomes a drag, so clicks still select.
const DRAG_THRESHOLD = 4;

export function ProjectNav({ projects, total, active, onSelect, onHide, onReorder }: Props) {
  const navRef = useRef<HTMLElement>(null);
  const [drag, setDrag] = useState<{ project: string; index: number } | null>(null);
  const dragRef = useRef<{ project: string; index: number } | null>(null);
  const press = useRef<{ project: string; y: number; dragging: boolean } | null>(null);
  // The click that follows a drag's pointerup must not select the project.
  const suppressClick = useRef(false);

  useScrollAnchor(navRef, [projects]);

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
    <nav className={`projects ${drag ? "dragging" : ""}`} ref={navRef}>
      <button className={`proj ${active === null ? "active" : ""}`} data-anchor="(all)" onClick={() => onSelect(null)}>
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
            className={`proj ${active === x.project ? "active" : ""}`}
            onClick={() => {
              if (suppressClick.current) {
                suppressClick.current = false;
                return;
              }
              onSelect(x.project);
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
