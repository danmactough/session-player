import { memo, useEffect, useMemo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { SearchHit, SessionRow } from "../types";
import { basename, compact, dayBucket, relTime } from "../lib/format";

type Item = { kind: "head"; label: string } | { kind: "row"; s: SessionRow };

interface Props {
  sessions: SessionRow[];
  selected: string | null;
  onSelect: (path: string) => void;
  hits: Map<string, SearchHit> | null;
  showProject: boolean;
  groupByDay: boolean;
}

export function SessionList({ sessions, selected, onSelect, hits, showProject, groupByDay }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const items = useMemo<Item[]>(() => {
    if (!groupByDay) return sessions.map((s) => ({ kind: "row", s }));
    const out: Item[] = [];
    let last = "";
    const now = new Date();
    for (const s of sessions) {
      const b = dayBucket(s.updated_at, now);
      if (b !== last) {
        out.push({ kind: "head", label: b });
        last = b;
      }
      out.push({ kind: "row", s });
    }
    return out;
  }, [sessions, groupByDay]);

  const virt = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (i) => (items[i].kind === "head" ? 34 : hits ? 96 : 66),
    overscan: 12,
    paddingEnd: 24,
  });

  const selIndex = items.findIndex((it) => it.kind === "row" && it.s.path === selected);
  useEffect(() => {
    if (selIndex >= 0) virt.scrollToIndex(selIndex, { align: "auto" });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selIndex]);

  return (
    <div className="list-scroll" ref={scrollRef}>
      <div style={{ height: virt.getTotalSize(), position: "relative" }}>
        {virt.getVirtualItems().map((v) => {
          const it = items[v.index];
          return (
            <div
              key={v.key}
              data-index={v.index}
              ref={virt.measureElement}
              className="vrow"
              style={{ transform: `translateY(${v.start}px)` }}
            >
              {it.kind === "head" ? (
                <div className="list-head">{it.label}</div>
              ) : (
                <Row
                  s={it.s}
                  active={it.s.path === selected}
                  onSelect={onSelect}
                  hit={hits?.get(it.s.path)}
                  showProject={showProject}
                />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

const Row = memo(function Row({
  s,
  active,
  onSelect,
  hit,
  showProject,
}: {
  s: SessionRow;
  active: boolean;
  onSelect: (p: string) => void;
  hit?: SearchHit;
  showProject: boolean;
}) {
  return (
    <button className={`srow ${active ? "active" : ""}`} onClick={() => onSelect(s.path)}>
      <div className="srow-title">
        <span className={`sdot src-${s.source}`} title={s.source} />
        <span className="t">{s.title}</span>
      </div>
      {hit && <Snippet text={hit.snippet} />}
      <div className="srow-meta">
        {showProject && <span className="sproj">{basename(s.project)}</span>}
        <span>{relTime(s.updated_at)}</span>
        <span>{s.user_turns} {s.user_turns === 1 ? "prompt" : "prompts"}</span>
        {s.output_tokens > 0 && <span>{compact(s.output_tokens)} out</span>}
        {s.is_subagent && <span className="tag">sub</span>}
      </div>
    </button>
  );
});

/** FTS snippets mark matches with \u0001 … \u0002. */
function Snippet({ text }: { text: string }) {
  const parts = text.split(/(\u0001[^\u0002]*\u0002)/);
  return (
    <div className="snippet">
      {parts.map((p, i) =>
        p.startsWith("\u0001") ? <mark key={i}>{p.slice(1, -1)}</mark> : <span key={i}>{p.replace(/\s+/g, " ")}</span>,
      )}
    </div>
  );
}
