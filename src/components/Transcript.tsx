import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { Annotation, AnnotationKind, EventRef, SessionEvent, SessionMeta, SessionRow } from "../types";
import { ask, message } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { buildBlocks, blockText, type Block, type ToolStep } from "../lib/blocks";
import {
  SOURCE_LABEL,
  basename,
  bytes,
  clock,
  compact,
  duration,
  fullDate,
  tildify,
  toolLabel,
  toolSummary,
} from "../lib/format";
import { Markdown } from "./Markdown";
import { Analysis, AnnotationEditor, AnnotationItem } from "./Notes";
import { blockEvents, ref, sessionKey } from "../lib/notes";
import { RemoteImages } from "../lib/remoteImages";
import { BranchIcon } from "./BranchIcon";
import { OpenButton } from "./OpenButton";
import { SourceIcon } from "./AppIcon";

interface Props {
  session: SessionRow;
  parent?: SessionRow;
  children: SessionRow[];
  highlight: string;
  /** Opens a session; `focus` names a child whose split point to scroll to. */
  onOpen: (path: string, focus?: string) => void;
  /** A child whose split point should be scrolled into view on open. */
  focus?: string | null;
  /** False while a sheet covers the app, so its keys don't reach the transcript. */
  keyboard: boolean;
  /** Treat shell commands that exited with a failing status as errors. */
  countExits: boolean;
  /** The chosen resume/open action (see OpenButton). */
  openWith: string | undefined;
  onOpenWith: (id: string) => void;
  onExport: () => void;
  /** Shown as the author of annotations made here. */
  authorName: string | null;
  /** Metadata or annotations changed, so the list's titles and filters should refresh. */
  onNotesChanged: () => void;
  /** Asks, the first time, whether sending a transcript to Anthropic is all right. */
  confirmGenerate: () => Promise<boolean>;
  /** The summary under the title (first prompt, analysis, stats) is hidden. */
  collapsed: boolean;
  onCollapse: (collapsed: boolean) => void;
}

export function Transcript({
  session,
  parent,
  children,
  highlight,
  onOpen,
  focus,
  keyboard,
  countExits,
  openWith,
  onOpenWith,
  onExport,
  authorName,
  onNotesChanged,
  confirmGenerate,
  collapsed,
  onCollapse,
}: Props) {
  const [events, setEvents] = useState<SessionEvent[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showTools, setShowTools] = useState(true);
  const [showThinking, setShowThinking] = useState(true);
  const [find, setFind] = useState("");
  const [findOpen, setFindOpen] = useState(false);
  const [findIdx, setFindIdx] = useState(0);
  const [copied, setCopied] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const findRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    // The component is keyed by path, so this only re-runs when the open
    // session's file grows. Keep showing the old events until the new ones
    // arrive so the scroll position survives live updates.
    let live = true;
    setError(null);
    api
      .loadSession(session.branch_of ?? session.path, session.branch_leaf)
      .then((ev) => {
        if (!live) return;
        setEvents(ev);
      })
      .catch((e) => {
        if (!live) return;
        setError(String(e));
        api.log("error", `load_session failed for ${session.path}: ${e}`);
      });
    return () => {
      live = false;
    };
    // Reload when the file changes on disk (updated_at/size move with it).
  }, [session.path, session.branch_of, session.branch_leaf, session.size, session.updated_at]);

  useEffect(() => {
    setFind(highlight);
    setFindIdx(0);
  }, [highlight, session.path]);

  // Branches and pi fork files start where they split off, so they get a
  // link back to that point; children that split off from this transcript
  // get a marker where they do.
  const forks = useMemo(() => {
    const isSplit = session.branch_of || session.fork_entry;
    return {
      from: isSplit && parent ? { source: parent, self: session } : undefined,
      to: children.filter((c) => c.fork_entry),
    };
  }, [session, parent, children]);

  const blocks = useMemo(() => {
    if (!events) return [];
    const shown = countExits ? events.map((e) => (e.failed_exit ? { ...e, is_error: true } : e)) : events;
    return buildBlocks(shown, forks).filter((b) => {
      if (!showTools && (b.type === "tools" || b.type === "orphan_result")) return false;
      if (!showThinking && b.type === "thinking") return false;
      return true;
    });
  }, [events, forks, showTools, showThinking, countExits]);

  const matches = useMemo(() => {
    const terms = find.toLowerCase().split(/\s+/).filter(Boolean);
    if (!terms.length) return [];
    const out: number[] = [];
    blocks.forEach((b, i) => {
      const t = blockText(b).toLowerCase();
      if (terms.every((term) => t.includes(term))) out.push(i);
    });
    return out;
  }, [blocks, find]);
  const matchSet = useMemo(() => new Set(matches), [matches]);

  const outline = useMemo(
    () =>
      blocks
        .map((b, i) => ({ b, i }))
        .filter((x): x is { b: Extract<Block, { type: "user" }>; i: number } => x.b.type === "user" && !x.b.ev.sidechain),
    [blocks],
  );

  const virt = useVirtualizer({
    count: blocks.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 96,
    overscan: 8,
    paddingStart: 8,
    paddingEnd: 160,
  });

  // Metadata and annotations. Anchors are checked against the transcript on
  // the backend, so they are fetched again whenever the events reload.
  const key = useMemo(() => sessionKey(session), [session]);
  const [notes, setNotes] = useState<{ meta: SessionMeta | null; annotations: Annotation[]; revision: string | null }>({
    meta: null,
    annotations: [],
    revision: null,
  });
  const [composing, setComposing] = useState<{ block: number; refs: EventRef[] } | null>(null);
  // `n` changes with every jump, so a block collapsed since the last one opens again.
  const [reveal, setReveal] = useState<{ block: number; step?: number; n: number } | null>(null);
  const [pendingJump, setPendingJump] = useState<Annotation | null>(null);
  const loadNotes = useCallback(
    () =>
      api
        .sessionNotes(key, session.branch_of ?? session.path, session.branch_leaf)
        // Annotations made on another branch of a pi session aren't about this view.
        .then((n) => setNotes({ ...n, annotations: n.annotations.filter((a) => !a.anchors.length || !a.anchors.every((r) => r.elsewhere)) }))
        .catch((e) => api.log("error", `session_notes failed for ${key}: ${e}`)),
    [key, session.branch_of, session.path, session.branch_leaf],
  );
  useEffect(() => {
    if (events) void loadNotes();
  }, [events, loadNotes]);

  // Generating notes with Claude runs in the backend; progress arrives as events.
  const [gen, setGen] = useState<{ done: number; total: number } | null>(null);
  const [genMessage, setGenMessage] = useState<string | null>(null);
  const loadNotesRef = useRef(loadNotes);
  loadNotesRef.current = loadNotes;
  useEffect(() => {
    let live = true;
    // An event that arrives first is newer than what generating() reports.
    let heard = false;
    void api.generating().then((keys) => live && !heard && keys.includes(key) && setGen({ done: 0, total: 0 }));
    const un = [
      listen<{ key: string; done: number; total: number }>("generate-progress", (e) => {
        if (e.payload.key !== key) return;
        heard = true;
        setGen({ done: e.payload.done, total: e.payload.total });
      }),
      listen<{ key: string; error: string | null; cancelled: boolean; rejected: number }>("generate-done", (e) => {
        if (e.payload.key !== key) return;
        heard = true;
        setGen(null);
        const { error, cancelled, rejected } = e.payload;
        if (error) setGenMessage(`Couldn't generate notes: ${error}`);
        else if (cancelled) setGenMessage("Generating was cancelled.");
        else {
          setGenMessage(
            rejected
              ? `Left out ${rejected} generated ${rejected === 1 ? "annotation that pointed" : "annotations that pointed"} at events this transcript doesn't have.`
              : null,
          );
          // The list refreshes itself (App listens too); this transcript reloads its notes.
          void loadNotesRef.current();
        }
      }),
    ];
    return () => {
      live = false;
      un.forEach((p) => p.then((f) => f()));
    };
  }, [key]);
  const generate = async () => {
    if (!(await confirmGenerate())) return;
    setGenMessage(null);
    setGen({ done: 0, total: 0 });
    await api.generateNotes(key, session.branch_of ?? session.path, session.branch_leaf).catch((e) => {
      setGen(null);
      setGenMessage(String(e));
    });
  };

  const saveMeta = async (meta: SessionMeta) => {
    const saved = await api.saveMeta(meta);
    setNotes((n) => ({ ...n, meta: saved }));
    onNotesChanged();
  };
  const saveAnnotation = async (a: Annotation) => {
    await api.saveAnnotation(a);
    await loadNotes();
    onNotesChanged();
  };
  const deleteAnnotation = async (a: Annotation) => {
    await api.deleteAnnotation(a.id);
    await loadNotes();
    onNotesChanged();
  };
  const addAnnotation = (anchors: EventRef[], kind: AnnotationKind, note: string) =>
    saveAnnotation({ id: "", key, kind, note, anchors, author: { type: "person", name: authorName }, created_at: 0, updated_at: 0 });

  const blockOfAnchor = useMemo(() => {
    const m = new Map<string, number>();
    blocks.forEach((b, i) => blockEvents(b).forEach((e) => e.anchor && m.set(e.anchor, i)));
    return m;
  }, [blocks]);
  // Each annotation is shown under the block holding the first event it points at.
  const notesByBlock = useMemo(() => {
    const m = new Map<number, Annotation[]>();
    for (const a of notes.annotations) {
      const r = a.anchors.find((r) => r.found && blockOfAnchor.has(r.anchor));
      if (!r) continue;
      const i = blockOfAnchor.get(r.anchor)!;
      m.set(i, [...(m.get(i) ?? []), a]);
    }
    return m;
  }, [notes.annotations, blockOfAnchor]);
  const jumpTo = (a: Annotation) => {
    const r = a.anchors.find((r) => r.found && blockOfAnchor.has(r.anchor));
    if (!r) {
      // The event is in a block hidden by the Tool calls or Reasoning toggle: show it, then jump.
      const hidden = a.anchors.find((r) => r.found);
      if (!hidden) return;
      if (hidden.kind === "thinking") setShowThinking(true);
      else setShowTools(true);
      setPendingJump(a);
      return;
    }
    const i = blockOfAnchor.get(r.anchor)!;
    const b = blocks[i];
    const step = b.type === "tools" ? b.steps.findIndex((s) => s.call.anchor === r.anchor || s.result?.anchor === r.anchor) : undefined;
    setReveal((prev) => ({ block: i, step, n: (prev?.n ?? 0) + 1 }));
    virt.scrollToIndex(i, { align: "center" });
  };
  // The annotation navigator steps through annotations in transcript order.
  const anchorNumber = (a: Annotation) => Math.min(...a.anchors.filter((r) => r.found).map((r) => Number(r.anchor.slice(1)) || 0));
  const noteStops = useMemo(
    () => notes.annotations.filter((a) => a.anchors.some((r) => r.found)).sort((x, y) => anchorNumber(x) - anchorNumber(y)),
    [notes.annotations],
  );
  const [noteIdx, setNoteIdx] = useState<number | null>(null);
  const stepNote = (dir: 1 | -1) => {
    if (!noteStops.length) return;
    const n = noteStops.length;
    const next = noteIdx === null || noteIdx >= n ? (dir === 1 ? 0 : n - 1) : (noteIdx + dir + n) % n;
    setNoteIdx(next);
    jumpTo(noteStops[next]);
  };
  // Show in the summary moves the navigator to that annotation too.
  const showNote = (a: Annotation) => {
    const i = noteStops.findIndex((x) => x.id === a.id);
    setNoteIdx(i >= 0 ? i : null);
    jumpTo(a);
  };
  const jumpRef = useRef(jumpTo);
  jumpRef.current = jumpTo;
  useEffect(() => {
    if (!pendingJump) return;
    setPendingJump(null);
    jumpRef.current(pendingJump);
  }, [pendingJump, blockOfAnchor]);
  const annotateStep = useCallback((block: number, step: ToolStep) => {
    setComposing({ block, refs: (step.result ? [step.call, step.result] : [step.call]).map(ref) });
  }, []);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: 0 });
    // Take keyboard focus so Space, Page Down and the arrows scroll the
    // transcript, unless the user is typing (arrowing through search results).
    const typing = document.activeElement?.matches("input, textarea, select, [contenteditable]");
    if (!typing) scrollRef.current?.focus({ preventScroll: true });
  }, [session.path]);

  // Jump to the first in-session match once the transcript has loaded.
  useEffect(() => {
    if (matches.length && events) {
      const i = Math.min(findIdx, matches.length - 1);
      virt.scrollToIndex(matches[i], { align: "center" });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [matches, findIdx, events]);

  // Arriving from a child's "split from" link: show the split point.
  const focusIndex = focus ? blocks.findIndex((b) => b.type === "fork_to" && b.child.path === focus) : -1;

  // Error navigator: one stop per error, so its count matches the session's.
  // A group of tool calls can hold several failed calls; each is a stop
  // (`step` is its place in the group).
  const errorStops = useMemo(() => {
    const out: { block: number; step?: number }[] = [];
    blocks.forEach((b, i) => {
      if (b.type === "tools") {
        b.steps.forEach((s, step) => {
          if (s.result?.is_error) out.push({ block: i, step });
        });
      } else if (b.type === "command" ? b.output?.is_error : (b.type === "system" || b.type === "orphan_result") && b.ev.is_error) {
        out.push({ block: i });
      }
    });
    return out;
  }, [blocks]);
  const [errIdx, setErrIdx] = useState<number | null>(null);
  const errorTarget = errIdx === null ? undefined : errorStops[errIdx];
  const stepError = (dir: 1 | -1) => {
    if (!errorStops.length) return;
    const n = errorStops.length;
    const next = errIdx === null ? (dir === 1 ? 0 : n - 1) : (errIdx + dir + n) % n;
    setErrIdx(next);
    virt.scrollToIndex(errorStops[next].block, { align: "center" });
  };
  useEffect(() => {
    if (focusIndex >= 0) virt.scrollToIndex(focusIndex, { align: "center" });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focusIndex]);

  // Scrolls to the next or previous block of a kind, relative to the block at
  // the top of the view, wrapping from the last to the first and back.
  const jump = (isTarget: (b: Block) => boolean, dir: 1 | -1) => {
    const el = scrollRef.current;
    if (!el) return;
    const targets = blocks.flatMap((b, i) => (isTarget(b) ? [i] : []));
    if (!targets.length) return;
    // Tolerance for a block that was scrolled to but sits a few pixels off,
    // e.g. when the view can't scroll any further.
    const top = el.scrollTop;
    const start = (i: number) => virt.measurementsCache[i]?.start ?? 0;
    // At the bottom, blocks below the top can't be scrolled to the top any
    // further, so going forward wraps instead.
    const atBottom = top + el.clientHeight >= el.scrollHeight - 2;
    const next =
      dir === 1
        ? ((atBottom ? undefined : targets.find((i) => start(i) > top + 8)) ?? targets[0])
        : (targets.findLast((i) => start(i) < top - 8) ?? targets[targets.length - 1]);
    virt.scrollToIndex(next, { align: "start" });
  };

  // Transcript keys. Space is handled here rather than left to the browser:
  // the browser pages only the focused scroller, and focus drops to the page
  // body when the row holding it scrolls out and is unmounted.
  const keyRef = useRef<(e: KeyboardEvent) => void>(() => {});
  keyRef.current = (e: KeyboardEvent) => {
    if (!keyboard) return;
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
      e.preventDefault();
      setFindOpen(true);
      requestAnimationFrame(() => findRef.current?.select());
      return;
    }
    // ⌘] / ⌘[ step through prompts and ⌘⇧] / ⌘⇧[ through responses, like
    // Forward and Back on macOS. Matched by physical key, since Shift turns
    // the bracket into a brace on some layouts but not others.
    if ((e.metaKey || e.ctrlKey) && !e.altKey && (e.code === "BracketRight" || e.code === "BracketLeft")) {
      e.preventDefault();
      const dir = e.code === "BracketRight" ? 1 : -1;
      if (e.shiftKey) jump((b) => b.type === "assistant", dir);
      else jump((b) => b.type === "user" && !b.ev.sidechain, dir);
      return;
    }
    const target = e.target as HTMLElement;
    // Text fields keep their own editing keys (⌘↑, Home, Space…).
    if (target.closest("input, textarea, select, [contenteditable], [role=listbox]")) return;
    const el = scrollRef.current;
    if (!el) return;
    // Pages by most of a screen, wrapping around at either end.
    const page = (dir: 1 | -1) => {
      const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 2;
      if (dir === 1 && atBottom) el.scrollTo({ top: 0 });
      else if (dir === -1 && el.scrollTop <= 0) el.scrollTo({ top: el.scrollHeight });
      else el.scrollBy({ top: dir * el.clientHeight * 0.85, behavior: "smooth" });
    };
    const arrow = e.key === "ArrowUp" ? -1 : e.key === "ArrowDown" ? 1 : 0;
    // macOS: ⌘↑ / ⌘↓ go to the top and bottom, ⌥↑ / ⌥↓ page.
    if (arrow && e.metaKey && !e.altKey && !e.ctrlKey && !e.shiftKey) {
      e.preventDefault();
      el.scrollTo({ top: arrow === 1 ? el.scrollHeight : 0 });
      return;
    }
    if (arrow && e.altKey && !e.metaKey && !e.ctrlKey && !e.shiftKey) {
      e.preventDefault();
      page(arrow);
      return;
    }
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    switch (e.key) {
      case " ":
        // A clicked button keeps focus, and Space would press it again. Inside
        // the transcript and its outline, paging matters more (Enter still presses).
        if (target.closest("button") && !target.closest(".t-scroll, .t-outline, .err-nav")) return;
        e.preventDefault();
        page(e.shiftKey ? -1 : 1);
        break;
      case "PageDown":
      case "PageUp":
        e.preventDefault();
        page(e.key === "PageDown" ? 1 : -1);
        break;
      case "Home":
      case "End":
        e.preventDefault();
        el.scrollTo({ top: e.key === "End" ? el.scrollHeight : 0 });
        break;
    }
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => keyRef.current(e);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const copy = async (label: string, text: string) => {
    await navigator.clipboard.writeText(text);
    setCopied(label);
    setTimeout(() => setCopied(null), 1400);
  };

  const elapsed = (session.updated_at ?? 0) - (session.started_at ?? 0);
  const totalIn = session.input_tokens + session.cache_read_tokens;

  return (
    <RemoteImages.Provider value={!session.from_bundle}>
      <div className="transcript">
        <header className="t-head" data-tauri-drag-region>
          <div className="t-eyebrow">
            <span className={`src src-${session.source}`}>
              <SourceIcon source={session.source} />
              {SOURCE_LABEL[session.source]}
            </span>
            <span className="sep">/</span>
            <span title={session.project}>{basename(session.project)}</span>
            {session.git_branch && (
              <>
                <span className="sep">/</span>
                <span className="branch">{session.git_branch}</span>
              </>
            )}
            {session.is_subagent && <span className="tag">subagent</span>}
            {session.branch_of && <span className="tag">branch</span>}
            {session.imported_from && <span className="tag">imported from {session.imported_from}</span>}
            {session.from_bundle && <span className="tag">imported bundle</span>}
          </div>
          <h1 className="t-title">{session.title}</h1>
          {/* The toggle sits in the margin beside whatever comes first: the summary, or the buttons when it's hidden. */}
          <div className={`t-fold ${collapsed ? "collapsed" : ""}`}>
            <button
              className="collapse-toggle"
              aria-expanded={!collapsed}
              aria-label={collapsed ? "Show the summary" : "Hide the summary"}
              title={collapsed ? "Show the summary" : "Hide the summary"}
              onClick={() => onCollapse(!collapsed)}
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                {collapsed ? (
                  <>
                    <path d="m7 15 5 5 5-5" />
                    <path d="m7 9 5-5 5 5" />
                  </>
                ) : (
                  <>
                    <path d="m7 20 5-5 5 5" />
                    <path d="m7 4 5 5 5-5" />
                  </>
                )}
              </svg>
            </button>
            {!collapsed && (
              <div className="t-summary">
                {session.title_source !== "prompt" && session.first_prompt && (
                  <p className="t-prompt">{session.first_prompt.split("\n").find((l) => l.trim()) ?? ""}</p>
                )}
                <Analysis
                  sessionKey={key}
                  meta={notes.meta}
                  revision={notes.revision}
                  annotations={notes.annotations}
                  onSaveMeta={saveMeta}
                  onSaveAnnotation={saveAnnotation}
                  onDeleteAnnotation={deleteAnnotation}
                  onJump={showNote}
                />
                <dl className="t-meta">
                  <div>
                    <dt>Started</dt>
                    <dd>{fullDate(session.started_at)}</dd>
                  </div>
                  <div>
                    <dt>Span</dt>
                    <dd>{duration(elapsed)}</dd>
                  </div>
                  <div>
                    <dt>Turns</dt>
                    <dd>
                      {session.user_turns} <span className="dim">/ {session.tool_calls} tools</span>
                      {session.error_count > 0 && (
                        <span className="err-count">
                          {" "}
                          · {session.error_count} {session.error_count === 1 ? "error" : "errors"}
                        </span>
                      )}
                    </dd>
                  </div>
                  {/* Token totals belong to the whole file; a branch has none of its own. */}
                  {!session.branch_of && (
                    <div>
                      <dt>Tokens</dt>
                      <dd title={`${session.input_tokens.toLocaleString()} input · ${session.cache_read_tokens.toLocaleString()} cache read · ${session.output_tokens.toLocaleString()} output`}>
                        {compact(totalIn)} <span className="dim">in</span> · {compact(session.output_tokens)} <span className="dim">out</span>
                      </dd>
                    </div>
                  )}
                  {session.models.length > 0 ? (
                    <div>
                      <dt>Model</dt>
                      <dd>{session.models.join(", ")}</dd>
                    </div>
                  ) : (
                    session.imported_from && (
                      <div>
                        <dt>Model</dt>
                        <dd title={`Codex imported this session from ${session.imported_from} without recording the model`}>
                          <span className="dim">not recorded</span>
                        </dd>
                      </div>
                    )
                  )}
                </dl>
              </div>
            )}
            <div className="t-actions">
              {session.from_bundle ? (
                <button disabled title="Imported sessions have no agent files on this Mac to resume or open">
                  Can&rsquo;t resume an imported session
                </button>
              ) : (
                <OpenButton session={session} choice={openWith} onChoice={onOpenWith} keyboard={keyboard} />
              )}
              {gen ? (
                <span className="generating">
                  <span className="pulse">Generating notes{gen.total > 1 ? ` · part ${Math.min(gen.done + 1, gen.total)} of ${gen.total}` : "…"}</span>
                  <button onClick={() => void api.cancelGenerate(key)}>Cancel</button>
                </span>
              ) : (
                <button onClick={() => void generate()} title="Suggest details and annotations with Claude Haiku">
                  Generate notes
                </button>
              )}
              <button onClick={onExport}>Export…</button>
            {session.from_bundle && (
              <button onClick={() => void removeImported(session, children)}>Remove…</button>
            )}
              <button onClick={() => api.reveal(session.path)}>Reveal file</button>
              <button onClick={() => copy("path", session.path)}>{copied === "path" ? "Copied" : "Copy path"}</button>
              <span className="t-file" title={session.path}>
                {tildify(session.cwd)} · {bytes(session.size)}
              </span>
            </div>
          </div>
          {genMessage && <p className="gen-message">{genMessage}</p>}
          {(parent || children.length > 0) && (
            <div className="t-family">
              {parent && (
                <button className="link" onClick={() => onOpen(parent.path)}>
                  ↖ {session.branch_of ? "Branched from" : session.fork_entry ? "Forked from" : "Parent"}: {parent.title}
                </button>
              )}
              {children.length > 0 && (
                <span className="kids">
                  {childSummary(children)}:
                  {children.slice(0, 8).map((c) => (
                    <button key={c.path} className="link" onClick={() => onOpen(c.path)}>
                      {c.branch_of && <BranchIcon className="inline" />}
                      {c.title}
                    </button>
                  ))}
                  {children.length > 8 && <span className="dim">and {children.length - 8} more in the list</span>}
                </span>
              )}
            </div>
          )}
        </header>

        <div className="t-toolbar">
          <label className="toggle">
            <input type="checkbox" checked={showTools} onChange={(e) => setShowTools(e.target.checked)} />
            Tool calls
          </label>
          <label className="toggle">
            <input type="checkbox" checked={showThinking} onChange={(e) => setShowThinking(e.target.checked)} />
            Reasoning
          </label>
          {noteStops.length > 0 && (
            <div className="err-nav note-nav" title="Jump between annotations">
              <span className="err-dot" />
              <span className="err-label">
                {noteIdx === null ? noteStops.length : `${noteIdx + 1}/${noteStops.length}`}{" "}
                {noteStops.length === 1 ? "annotation" : "annotations"}
              </span>
              <button aria-label="Previous annotation" onClick={() => stepNote(-1)}>
                ↑
              </button>
              <button aria-label="Next annotation" onClick={() => stepNote(1)}>
                ↓
              </button>
            </div>
          )}
          {errorStops.length > 0 && (
            <div className="err-nav" title="Jump between failed tool calls and errors">
              <span className="err-dot" />
              <span className="err-label">
                {errIdx === null ? errorStops.length : `${errIdx + 1}/${errorStops.length}`}{" "}
                {errorStops.length === 1 ? "error" : "errors"}
              </span>
              <button aria-label="Previous error" onClick={() => stepError(-1)}>
                ↑
              </button>
              <button aria-label="Next error" onClick={() => stepError(1)}>
                ↓
              </button>
            </div>
          )}
          <div className={`t-find ${findOpen || find ? "open" : ""}`}>
            <input
              ref={findRef}
              placeholder="Find in session  ⌘F"
              value={find}
              onChange={(e) => {
                setFind(e.target.value);
                setFindIdx(0);
              }}
              onFocus={() => setFindOpen(true)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && matches.length) {
                  setFindIdx((i) => (e.shiftKey ? (i - 1 + matches.length) % matches.length : (i + 1) % matches.length));
                }
                if (e.key === "Escape") {
                  setFind("");
                  setFindOpen(false);
                  (e.target as HTMLInputElement).blur();
                }
              }}
            />
            {find && (
              <span className="count">
                {matches.length ? `${(findIdx % matches.length) + 1}/${matches.length}` : "0"}
              </span>
            )}
          </div>
        </div>

        <div className="t-body">
          <div className="t-scroll" ref={scrollRef} tabIndex={-1}>
            {error && <div className="empty">Could not read this session: {error}</div>}
            {!events && !error && <div className="empty pulse">Reading session…</div>}
            {events && blocks.length === 0 && <div className="empty">Nothing to show.</div>}
            <div className="t-stream" style={{ height: virt.getTotalSize() }}>
              {virt.getVirtualItems().map((item) => {
                const b = blocks[item.index];
                const isCurrent = matches.length > 0 && matches[findIdx % matches.length] === item.index;
                return (
                  <div
                    key={item.key}
                    data-index={item.index}
                    ref={virt.measureElement}
                    className={`row ${matchSet.has(item.index) ? "hit" : ""} ${isCurrent ? "hit-current" : ""} ${item.index === focusIndex || (item.index === errorTarget?.block && b.type !== "tools") || (item.index === reveal?.block && b.type !== "tools") ? "focused" : ""}`}
                    style={{ transform: `translateY(${item.start}px)` }}
                  >
                    <BlockView
                      block={b}
                      index={item.index}
                      onOpen={onOpen}
                      openErrors={item.index === errorTarget?.block}
                      errorStep={item.index === errorTarget?.block ? errorTarget.step : undefined}
                      reveal={item.index === reveal?.block ? reveal : undefined}
                      onAnnotateStep={annotateStep}
                    />
                    {blockEvents(b).some((e) => e.anchor) && (
                      <button
                        className="annotate-btn"
                        title="Annotate"
                        aria-label="Annotate"
                        onClick={() => setComposing({ block: item.index, refs: blockEvents(b).filter((e) => e.anchor).map(ref) })}
                      >
                        +
                      </button>
                    )}
                    {(notesByBlock.has(item.index) || composing?.block === item.index) && (
                      <div className="row-notes">
                        {notesByBlock.get(item.index)?.map((a) => (
                          <AnnotationItem key={a.id} annotation={a} revision={notes.revision} onSave={saveAnnotation} onDelete={deleteAnnotation} />
                        ))}
                        {composing?.block === item.index && (
                          <AnnotationEditor
                            onCancel={() => setComposing(null)}
                            onSave={async (kind, note) => {
                              await addAnnotation(composing.refs, kind, note);
                              setComposing(null);
                            }}
                          />
                        )}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
          {outline.length > 1 && (
            <nav className="t-outline">
              <div className="o-head">Prompts</div>
              {outline.map(({ b, i }) => (
                <button key={i} onClick={() => virt.scrollToIndex(i, { align: "start" })} title={b.ev.text.slice(0, 400)}>
                  <span className="o-n">{b.turn}</span>
                  <span className="o-t">{b.ev.text.replace(FILE_TAG, "").trim().slice(0, 90) || "(image)"}</span>
                </button>
              ))}
            </nav>
          )}
        </div>
      </div>
    </RemoteImages.Provider>
  );
}

function childSummary(children: SessionRow[]): string {
  const branches = children.filter((c) => c.branch_of).length;
  const subs = children.filter((c) => !c.branch_of && c.is_subagent).length;
  const forks = children.length - branches - subs;
  const part = (n: number, one: string, many: string) => (n ? `${n} ${n === 1 ? one : many}` : "");
  return [part(branches, "branch", "branches"), part(subs, "subagent", "subagents"), part(forks, "fork", "forks")]
    .filter(Boolean)
    .join(", ");
}

const forkNoun = (s: SessionRow) => (s.branch_of ? "Branch" : s.is_subagent ? "Subagent" : "Fork");
const afterPrompt = (n: number | null | undefined) => (n ? `after prompt ${n}` : "before the first prompt");

const BlockView = memo(function BlockView({
  block: b,
  index,
  onOpen,
  openErrors,
  errorStep,
  reveal,
  onAnnotateStep,
}: {
  block: Block;
  index: number;
  onOpen: (path: string, focus?: string) => void;
  /** Expand collapsed content (and in a tool group, this call) to show an annotated event. */
  reveal?: { step?: number; n: number };
  onAnnotateStep: (block: number, step: ToolStep) => void;
  /** Expand failed tool calls (set while the error navigator points here). */
  openErrors?: boolean;
  /** The failed tool call in this group the error navigator points at. */
  errorStep?: number;
}) {
  switch (b.type) {
    case "fork_from":
      return (
        <div className="b-fork from">
          <BranchIcon className="glyph" />
          <span>
            {b.self.branch_of ? "Branched" : "Forked"} {afterPrompt(b.self.fork_turn)} of{" "}
            <button className="link" onClick={() => onOpen(b.source.path, b.self.path)}>
              {b.source.title}
            </button>
          </span>
        </div>
      );
    case "fork_to":
      return (
        <div className="b-fork to">
          <BranchIcon className="glyph" />
          <span>
            {forkNoun(b.child)}{" "}
            <button className="link" onClick={() => onOpen(b.child.path)}>
              {b.child.title}
            </button>{" "}
            splits off here
          </span>
        </div>
      );
    case "user":
      return (
        <section className={`b-user ${b.ev.sidechain ? "side" : ""}`}>
          <div className="b-label">
            <span>{b.ev.sidechain ? "Delegated" : `Prompt ${b.turn}`}</span>
            <time>{clock(b.ev.ts)}</time>
          </div>
          {b.ev.text && <UserText text={b.ev.text} />}
          <Images images={b.ev.images} />
        </section>
      );
    case "assistant":
      return (
        <section className={`b-assistant ${b.ev.sidechain ? "side" : ""}`}>
          <Markdown text={b.ev.text} />
        </section>
      );
    case "thinking":
      return <Thinking ev={b.ev} reveal={reveal?.n} />;
    case "tools":
      return (
        <Tools
          steps={b.steps}
          openErrors={openErrors}
          errorStep={errorStep}
          revealStep={reveal?.step}
          revealN={reveal?.n}
          onAnnotate={(step) => onAnnotateStep(index, step)}
        />
      );
    case "command":
      return (
        <section className="b-command">
          <code className="cmd">{b.ev.text}</code>
          {b.output && <Output text={b.output.text} isError={b.output.is_error} lines={12} />}
        </section>
      );
    case "system":
      return b.ev.is_error ? (
        // Laid out like a failed tool call: guide line, red dot, message.
        <div className="b-system err">
          <span className="dot" />
          <span>{b.ev.text}</span>
        </div>
      ) : (
        <div className="b-system">{b.ev.text}</div>
      );
    case "compaction":
      return <Compaction ev={b.ev} reveal={reveal?.n} />;
    case "reference":
      return <Reference ev={b.ev} reveal={reveal?.n} />;
    case "orphan_result":
      return (
        <section className="b-tools">
          <Output text={b.ev.text} isError={b.ev.is_error} lines={12} />
        </section>
      );
  }
});

function Thinking({ ev, reveal }: { ev: SessionEvent; reveal?: number }) {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (reveal) setOpen(true);
  }, [reveal]);
  const first = ev.text.trim().split("\n")[0];
  return (
    <section className={`b-thinking ${open ? "open" : ""}`} onClick={() => !open && setOpen(true)}>
      <button className="b-thinking-head" onClick={() => setOpen((o) => !o)}>
        <span className="caret">{open ? "▾" : "▸"}</span>
        <span className="lbl">Reasoning</span>
        {!open && <span className="preview">{first}</span>}
      </button>
      {open && <Markdown text={ev.text} className="thinking-md" />}
    </section>
  );
}

function Compaction({ ev, reveal }: { ev: SessionEvent; reveal?: number }) {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (reveal) setOpen(true);
  }, [reveal]);
  return (
    <section className="b-compaction">
      <button onClick={() => setOpen((o) => !o)} disabled={!ev.text}>
        <span className="rule" />
        <span>Context compacted{ev.text ? (open ? " — hide summary" : " — show summary") : ""}</span>
        <span className="rule" />
      </button>
      {open && <Markdown text={ev.text} className="compaction-md" />}
    </section>
  );
}

/** Earlier conversation the session was given, such as the ChatGPT chat a
 * Codex task started from; collapsed, since the prompt that follows says what
 * was asked. */
function Reference({ ev, reveal }: { ev: SessionEvent; reveal?: number }) {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (reveal) setOpen(true);
  }, [reveal]);
  return (
    <section className="b-reference">
      <button onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <span className="caret">{open ? "▾" : "▸"}</span>
        <span>{ev.title ?? "Referenced conversation"}</span>
      </button>
      {open && <Markdown text={ev.text} className="reference-md" />}
    </section>
  );
}

function Tools({
  steps,
  openErrors,
  errorStep,
  revealStep,
  revealN,
  onAnnotate,
}: {
  steps: ToolStep[];
  openErrors?: boolean;
  errorStep?: number;
  revealStep?: number;
  revealN?: number;
  onAnnotate: (step: ToolStep) => void;
}) {
  const errors = steps.filter((s) => s.result?.is_error).length;
  return (
    <section className="b-tools">
      {steps.length > 2 && (
        <div className="b-tools-head">
          {steps.length} tool calls{errors ? <span className="err"> · {errors} failed</span> : null}
        </div>
      )}
      {steps.map((s, i) => (
        <ToolRow
          key={s.call.call_id ?? i}
          step={s}
          forceOpen={(openErrors && s.result?.is_error) || i === revealStep}
          revealN={i === revealStep ? revealN : undefined}
          current={i === errorStep || i === revealStep}
          onAnnotate={onAnnotate}
        />
      ))}
    </section>
  );
}

function ToolRow({
  step,
  forceOpen,
  current,
  revealN,
  onAnnotate,
}: {
  step: ToolStep;
  forceOpen?: boolean;
  current?: boolean;
  /** Changes with each jump to this call, reopening it if it was closed. */
  revealN?: number;
  onAnnotate: (step: ToolStep) => void;
}) {
  const [open, setOpen] = useState(false);
  const rowRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (forceOpen) setOpen(true);
  }, [forceOpen, revealN]);
  // The navigator scrolls to the group; in a long group, bring this call into
  // view too, once the virtualizer has placed the group.
  useEffect(() => {
    if (!current) return;
    const id = requestAnimationFrame(() => rowRef.current?.scrollIntoView({ block: "nearest" }));
    return () => cancelAnimationFrame(id);
  }, [current]);
  const { call, result } = step;
  const summary = useMemo(() => toolSummary(call.tool_name, call.text), [call]);
  const status = !result ? "pending" : result.is_error ? "error" : "ok";
  return (
    // In a group of tool calls, the error navigator marks only the call it points at.
    <div ref={rowRef} className={`tool ${open ? "open" : ""} ${status} ${current ? "focused-step" : ""}`}>
      <button className="tool-head" onClick={() => setOpen((o) => !o)}>
        <span className="dot" />
        <span className="tool-name">{toolLabel(call.tool_name)}</span>
        <span className="tool-sum">{summary}</span>
        {result?.images?.length ? <span className="tool-flag">image</span> : null}
        {result?.exit_code && !result.is_error ? <span className="tool-flag">exit {result.exit_code}</span> : null}
      </button>
      {open && (
        <div className="tool-body">
          <ToolInput name={call.tool_name} text={call.text} />
          {result && (
            <>
              <div className="tool-sub">{result.is_error ? "Error" : "Result"}</div>
              {result.text && <Output text={result.text} isError={result.is_error} lines={40} />}
              <Images images={result.images} />
            </>
          )}
          {call.anchor && (
            <button className="link tool-annotate" onClick={() => onAnnotate(step)}>
              Annotate this call
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function ToolInput({ name, text }: { name?: string; text: string }) {
  const parsed = useMemo(() => {
    try {
      return JSON.parse(text) as Record<string, unknown>;
    } catch {
      return null;
    }
  }, [text]);
  const edits = parsed ? extractEdits(parsed) : [];
  if (edits.length) {
    return (
      <div className="diff-wrap">
        {edits.map((e, i) => (
          <Diff key={i} oldText={e.old} newText={e.new} />
        ))}
      </div>
    );
  }
  if (parsed && typeof parsed.content === "string" && /write/i.test(name ?? "")) {
    return <Diff oldText="" newText={parsed.content} />;
  }
  if (!parsed && /^\*\*\* Begin Patch|^diff --git/m.test(text)) {
    return <PatchView text={text} />;
  }
  const cmd = parsed && (typeof parsed.command === "string" ? parsed.command : typeof parsed.cmd === "string" ? parsed.cmd : null);
  if (cmd) {
    return <pre className="code cmd-block">{cmd}</pre>;
  }
  return <Output text={text} lines={30} />;
}

function extractEdits(o: Record<string, unknown>): { old: string; new: string }[] {
  const pair = (x: Record<string, unknown>) => {
    const a = x.old_string ?? x.oldText ?? x.old_str;
    const b = x.new_string ?? x.newText ?? x.new_str;
    return typeof a === "string" && typeof b === "string" ? { old: a, new: b } : null;
  };
  const single = pair(o);
  if (single) return [single];
  if (Array.isArray(o.edits)) {
    return o.edits.map((e) => (e && typeof e === "object" ? pair(e as Record<string, unknown>) : null)).filter(Boolean) as {
      old: string;
      new: string;
    }[];
  }
  return [];
}

function Diff({ oldText, newText }: { oldText: string; newText: string }) {
  const lines = useMemo(() => simpleDiff(oldText, newText), [oldText, newText]);
  return <Clamp lines={lines.length} max={60} render={(n) => (
    <pre className="code diff">
      {lines.slice(0, n).map((l, i) => (
        <div key={i} className={`dl ${l.t}`}>
          <span className="sign">{l.t === "add" ? "+" : l.t === "del" ? "−" : " "}</span>
          {l.s || " "}
        </div>
      ))}
    </pre>
  )} />;
}

/** Line diff via common prefix/suffix trimming; enough for typical edit hunks. */
function simpleDiff(a: string, b: string) {
  const A = a ? a.split("\n") : [];
  const B = b ? b.split("\n") : [];
  let pre = 0;
  while (pre < A.length && pre < B.length && A[pre] === B[pre]) pre++;
  let suf = 0;
  while (suf < A.length - pre && suf < B.length - pre && A[A.length - 1 - suf] === B[B.length - 1 - suf]) suf++;
  const out: { t: "ctx" | "add" | "del"; s: string }[] = [];
  A.slice(Math.max(0, pre - 2), pre).forEach((s) => out.push({ t: "ctx", s }));
  A.slice(pre, A.length - suf).forEach((s) => out.push({ t: "del", s }));
  B.slice(pre, B.length - suf).forEach((s) => out.push({ t: "add", s }));
  A.slice(A.length - suf, A.length - suf + 2).forEach((s) => out.push({ t: "ctx", s }));
  return out;
}

function PatchView({ text }: { text: string }) {
  const lines = text.split("\n");
  return <Clamp lines={lines.length} max={60} render={(n) => (
    <pre className="code diff">
      {lines.slice(0, n).map((l, i) => {
        const t = l.startsWith("+") && !l.startsWith("+++") ? "add" : l.startsWith("-") && !l.startsWith("---") ? "del" : l.startsWith("***") || l.startsWith("@@") ? "hunk" : "ctx";
        return (
          <div key={i} className={`dl ${t}`}>
            {l || " "}
          </div>
        );
      })}
    </pre>
  )} />;
}

function Output({ text, isError, lines = 30 }: { text: string; isError?: boolean; lines?: number }) {
  const all = useMemo(() => text.replace(/\s+$/, "").split("\n"), [text]);
  return <Clamp lines={all.length} max={lines} render={(n) => (
    <pre className={`code out ${isError ? "err" : ""}`}>{all.slice(0, n).join("\n")}</pre>
  )} />;
}

// Attached files, in the form the backend normalizes every agent's file
// mentions to (pi writes it natively for `@path` arguments):
//   <file name="/abs/path" note="active file">contents</file>
// plus <open_tabs>one path per line</open_tabs> from Codex's IDE context.
const ATTACHMENT = /<file name="([^"]*)"(?: note="([^"]*)")?>([\s\S]*?)<\/file>|<open_tabs>([\s\S]*?)<\/open_tabs>/g;
const FILE_TAG = /<file name="[^"]*"(?: note="[^"]*")?>|<\/file>|<open_tabs>[\s\S]*?<\/open_tabs>/g;

type PromptPart =
  | { kind: "text"; text: string }
  | { kind: "file"; name: string; note: string; body: string }
  | { kind: "tabs"; paths: string[] };

/** A user prompt, with attached files shown as cards. */
function UserText({ text }: { text: string }) {
  const parts: PromptPart[] = [];
  let at = 0;
  for (const m of text.matchAll(ATTACHMENT)) {
    const before = text.slice(at, m.index).trim();
    if (before) parts.push({ kind: "text", text: before });
    if (m[4] !== undefined) parts.push({ kind: "tabs", paths: m[4].split("\n").filter(Boolean) });
    else parts.push({ kind: "file", name: m[1], note: m[2] ?? "", body: m[3].replace(/^\n+|\s+$/g, "") });
    at = m.index + m[0].length;
  }
  const rest = text.slice(at).trim();
  if (rest) parts.push({ kind: "text", text: rest });
  return (
    <>
      {parts.map((p, i) =>
        p.kind === "text" ? (
          <div key={i} className="b-user-text">
            {p.text}
          </div>
        ) : p.kind === "tabs" ? (
          <div key={i} className="open-tabs" title={p.paths.join("\n")}>
            Open in editor: {p.paths.map((x) => basename(x)).join(", ")}
          </div>
        ) : (
          <FileCard key={i} name={p.name} note={p.note} body={p.body} />
        ),
      )}
    </>
  );
}

function FileCard({ name, note, body }: { name: string; note: string; body: string }) {
  const [open, setOpen] = useState(false);
  const file = basename(name);
  // Temporary files (subagent task files, pasted images) have no useful directory to show.
  const dir = /^\/(private\/)?(var\/folders|tmp)\//.test(name)
    ? "temporary file"
    : name.includes("/")
      ? tildify(name.slice(0, -file.length - 1))
      : "";
  const lines = useMemo(() => body.split("\n"), [body]);
  const meta = [note, dir, body ? `${lines.length} ${lines.length === 1 ? "line" : "lines"}` : ""].filter(Boolean).join(" · ");
  return (
    <div className={`file-card ${open ? "open" : ""}`}>
      <button className="file-head" title={name} onClick={() => setOpen((o) => !o)} disabled={!body}>
        {body ? <span className="caret">▸</span> : null}
        <svg className="file-icon" width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
          <path d="M2.5 1h4.5l2.5 2.5V11h-7z M7 1v2.5h2.5" />
        </svg>
        <span className="file-name">{file}</span>
        <span className="file-dir">{meta}</span>
      </button>
      {open && (
        <div className="file-body">
          <Clamp lines={lines.length} max={40} render={(n) => <pre className="code">{lines.slice(0, n).join("\n")}</pre>} />
        </div>
      )}
    </div>
  );
}

function Clamp({ lines, max, render }: { lines: number; max: number; render: (n: number) => React.ReactNode }) {
  const [full, setFull] = useState(false);
  const n = full ? lines : Math.min(lines, max);
  return (
    <>
      {render(n)}
      {lines > max && (
        <button className="more" onClick={() => setFull((f) => !f)}>
          {full ? "Show less" : `Show all ${lines} lines`}
        </button>
      )}
    </>
  );
}

async function removeImported(session: SessionRow, children: SessionRow[]) {
  const subagents = children.filter((c) => c.from_bundle && c.is_subagent).length;
  const ok = await ask(
    `“${session.title}”${subagents ? ` and its ${subagents === 1 ? "subagent" : `${subagents} subagents`}` : ""} will be deleted from your library. The original sender's copy isn't affected.`,
    { title: "Remove imported session?", kind: "warning", okLabel: "Remove", cancelLabel: "Cancel" },
  );
  if (!ok) return;
  await api
    .removeImported(session.path)
    .catch((e) => message(String(e), { title: "Couldn't remove the session", kind: "error" }));
}

function Images({ images }: { images?: string[] }) {
  const [zoom, setZoom] = useState<string | null>(null);
  if (!images?.length) return null;
  return (
    <div className="imgs">
      {images.map((src, i) =>
        src.startsWith("omitted") ? (
          <span key={i} className="img-omitted">
            {src === "omitted:remote" ? "remote image not included" : "image omitted (too large)"}
          </span>
        ) : (
          <img
            key={i}
            src={src}
            onClick={() => setZoom(src)}
            loading="lazy"
            onError={() => api.log("error", `image failed to load: ${src.slice(0, 60)}`)}
          />
        ),
      )}
      {zoom && (
        <div className="lightbox" onClick={() => setZoom(null)}>
          <img src={zoom} />
        </div>
      )}
    </div>
  );
}
