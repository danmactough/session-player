import { memo, useEffect, useLayoutEffect, useMemo, useRef } from "react";
import { useVirtualizer, type VirtualItem } from "@tanstack/react-virtual";
import type { SearchHit, NoteSummary } from "../types";
import { OUTCOME_LABEL, sessionKey } from "../lib/notes";
import type { ListRow } from "../lib/tree";
import { basename, compact, dayBucket, relTime } from "../lib/format";
import { BranchIcon } from "./BranchIcon";

type Item = { kind: "head"; label: string } | { kind: "row"; row: ListRow };

/** How a row was clicked: ⌘-click toggles it in the selection, ⇧-click selects a range. */
export interface ListMemo {
  scroll?: number;
  measurements?: VirtualItem[];
}

export interface ClickHow {
  toggle: boolean;
  range: boolean;
}

interface Props {
  rows: ListRow[];
  selected: string | null;
  /** Sessions picked with ⌘-click or ⇧-click, by path. */
  picked: Set<string>;
  onSelect: (path: string, how: ClickHow) => void;
  onToggle: (path: string) => void;
  hits: Map<string, SearchHit> | null;
  showProject: boolean;
  notes: Map<string, NoteSummary>;
  groupByDay: boolean;
  /** A new identity means the user asked for a different list (filters, sort,
   * project, search), which starts at the top instead of keeping its place. */
  resetKey: unknown;
  /** Where the list was scrolled to and how tall its rows measured, kept
   * here while it's away so it comes back exactly as it was left. */
  memo?: ListMemo;
  /** A click on the list's empty space, between or after the rows. */
  onBlankClick?: () => void;
}

const itemKey = (it: Item) => (it.kind === "head" ? `head:${it.label}` : it.row.s.path);

export function SessionList({ rows, selected, picked, onSelect, onToggle, hits, showProject, notes, groupByDay, resetKey, memo, onBlankClick }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const items = useMemo<Item[]>(() => {
    if (!groupByDay) return rows.map((row) => ({ kind: "row", row }));
    const out: Item[] = [];
    let last = "";
    const now = new Date();
    for (const row of rows) {
      // Children sit under their parent regardless of their own dates.
      if (row.depth === 0) {
        const b = dayBucket(row.s.updated_at, now);
        if (b !== last) {
          out.push({ kind: "head", label: b });
          last = b;
        }
      }
      out.push({ kind: "row", row });
    }
    return out;
  }, [rows, groupByDay]);
  const keys = useMemo(() => items.map(itemKey), [items]);

  const virt = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (i) => (items[i].kind === "head" ? 34 : hits ? 96 : 66),
    // Stable keys let measured heights follow an item when the list changes.
    getItemKey: (i) => keys[i],
    overscan: 12,
    paddingEnd: 24,
    initialOffset: memo?.scroll,
    // Measured heights from last time, so restoring the offset doesn't shift as rows are measured again.
    initialMeasurementsCache: memo?.measurements,
  });
  useEffect(
    () => () => {
      if (memo) memo.measurements = virt.measurementsCache;
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  // Scroll anchoring: when the list changes under the user (expanding a
  // session, sessions added while open), keep the top-most visible item where
  // it was on screen so everything else reflows around it. The scroll offset is read during render, before React
  // touches the DOM (afterwards the browser may already have clamped it), and
  // compared with item positions from the previous commit.
  const scrollTopBeforeCommit = scrollRef.current?.scrollTop ?? 0;
  const previous = useRef<{ keys: string[]; positions: { start: number; end: number }[] } | null>(null);
  const lastReset = useRef(resetKey);
  const resetting = lastReset.current !== resetKey;

  useLayoutEffect(() => {
    if (!resetting) return;
    lastReset.current = resetKey;
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
  });

  useLayoutEffect(() => {
    const el = scrollRef.current;
    const prev = previous.current;
    if (!el || !prev || prev.keys === keys || resetting) return;
    const top = scrollTopBeforeCommit;
    const oldIndex = prev.positions.findIndex((m) => m.end > top);
    if (oldIndex < 0) return;
    const indexOf = new Map(keys.map((k, i) => [k, i]));
    let idx: number | undefined;
    // If the anchor item disappeared, use the next one that survived.
    for (let i = oldIndex; idx === undefined && i < prev.keys.length; i++) idx = indexOf.get(prev.keys[i]);
    const m = idx === undefined ? undefined : virt.measurementsCache[idx];
    if (m) el.scrollTop = Math.max(0, m.start - (prev.positions[oldIndex].start - top));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [keys]);

  // Runs after the effect above on every commit, so it always sees the
  // positions from the commit before.
  useLayoutEffect(() => {
    previous.current = { keys, positions: virt.measurementsCache.map((m) => ({ start: m.start, end: m.end })) };
  });

  // Bring a newly selected session into view (keyboard navigation, links),
  // but not when the list merely reflows around an existing selection, or
  // comes back scrolled where the user left it.
  const restoring = useRef(memo?.scroll !== undefined);
  useEffect(() => {
    if (restoring.current) {
      restoring.current = false;
      return;
    }
    const i = keys.indexOf(selected ?? "");
    if (i >= 0) virt.scrollToIndex(i, { align: "auto" });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected]);

  return (
    <div
      className="list-scroll"
      ref={scrollRef}
      onScroll={memo && ((e) => (memo.scroll = e.currentTarget.scrollTop))}
      onClick={onBlankClick && ((e) => !(e.target as Element).closest(".srow") && onBlankClick())}
    >
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
                  row={it.row}
                  active={it.row.s.path === selected}
                  picked={picked.has(it.row.s.path)}
                  onSelect={onSelect}
                  onToggle={onToggle}
                  hit={hits?.get(it.row.s.path)}
                  showProject={showProject}
                  // A pi branch shares its session's notes; its own label stays.
                  note={it.row.s.branch_of ? undefined : notes.get(sessionKey(it.row.s))}
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
  row,
  active,
  picked,
  onSelect,
  onToggle,
  hit,
  showProject,
  note,
}: {
  row: ListRow;
  note?: NoteSummary;
  active: boolean;
  picked: boolean;
  onSelect: (p: string, how: ClickHow) => void;
  onToggle: (p: string) => void;
  hit?: SearchHit;
  showProject: boolean;
}) {
  const { s, depth, kind } = row;
  return (
    <div
      className={`srow ${active ? "active" : ""} ${picked ? "picked" : ""} ${depth ? "child" : ""}`}
      style={depth ? { paddingLeft: 18 + depth * 18 } : undefined}
      role="button"
      tabIndex={-1}
      aria-selected={picked}
      // Keeps ⇧-click from selecting the text between the rows.
      onMouseDown={(e) => e.shiftKey && e.preventDefault()}
      onClick={(e) => onSelect(s.path, { toggle: e.metaKey || e.ctrlKey, range: e.shiftKey })}
    >
      <div className="srow-title">
        {kind === "branch" ? <BranchIcon className="branch-glyph" /> : <span className={`sdot src-${s.source}`} title={s.source} />}
        <span className="t">{s.title}</span>
        {/* An analyzed session shows its outcome even when Analyze couldn't tell, so it doesn't look unanalyzed. */}
        {note && (note.outcome !== "unknown" || note.analyzed) && <span className={`outcome outcome-${note.outcome}`}>{OUTCOME_LABEL[note.outcome]}</span>}
      </div>
      {hit && <Snippet text={hit.snippet} />}
      <div className="srow-meta">
        {kind && (
          <span className="kind">
            {kind}
            {s.fork_entry ? (s.fork_turn ? ` after prompt ${s.fork_turn}` : " from the start") : ""}
          </span>
        )}
        {showProject && !depth && <span className="sproj">{basename(s.project)}</span>}
        <span>{relTime(s.updated_at)}</span>
        <span>
          {s.user_turns} {s.user_turns === 1 ? "prompt" : "prompts"}
        </span>
        {s.output_tokens > 0 && <span>{compact(s.output_tokens)} out</span>}
        {s.error_count > 0 && (
          <span className="err-count">
            {s.error_count} {s.error_count === 1 ? "error" : "errors"}
          </span>
        )}
        {!depth && s.is_subagent && <span className="tag">sub</span>}
        {s.from_bundle && <span className="tag">imported</span>}
      </div>
      {row.childCount > 0 && (
        <button
          className={`kids-toggle ${row.expanded ? "open" : ""}`}
          onClick={(e) => {
            e.stopPropagation();
            onToggle(s.path);
          }}
        >
          <span className="caret">▸</span>
          {row.childSummary}
        </button>
      )}
    </div>
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
