import { memo, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { SessionEvent, SessionRow } from "../types";
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
  resumeCommand,
  tildify,
  toolLabel,
  toolSummary,
} from "../lib/format";
import { Markdown } from "./Markdown";
import { BranchIcon } from "./BranchIcon";

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
}

export function Transcript({ session, parent, children, highlight, onOpen, focus, keyboard, countExits }: Props) {
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

  // Error navigator: blocks holding a failed tool call or an error notice.
  const errorBlocks = useMemo(() => {
    const out: number[] = [];
    blocks.forEach((b, i) => {
      const failed =
        b.type === "tools"
          ? b.steps.some((s) => s.result?.is_error)
          : b.type === "command"
            ? !!b.output?.is_error
            : b.type === "system" || b.type === "orphan_result"
              ? !!b.ev.is_error
              : false;
      if (failed) out.push(i);
    });
    return out;
  }, [blocks]);
  const [errIdx, setErrIdx] = useState<number | null>(null);
  const errorTarget = errIdx === null ? -1 : (errorBlocks[errIdx] ?? -1);
  const stepError = (dir: 1 | -1) => {
    if (!errorBlocks.length) return;
    const n = errorBlocks.length;
    const next = errIdx === null ? (dir === 1 ? 0 : n - 1) : (errIdx + dir + n) % n;
    setErrIdx(next);
    virt.scrollToIndex(errorBlocks[next], { align: "center" });
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
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const target = e.target as HTMLElement;
    if (target.closest("input, textarea, select, [contenteditable], [role=listbox]")) return;
    const el = scrollRef.current;
    if (!el) return;
    // A clicked button keeps focus, and Space would press it again. Inside the
    // transcript and its outline, paging matters more (Enter still presses).
    if (target.closest("button") && !target.closest(".t-scroll, .t-outline, .err-nav")) return;
    switch (e.key) {
      case " ": {
        e.preventDefault();
        // At either end, wrap around to the other.
        const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 2;
        if (!e.shiftKey && atBottom) el.scrollTo({ top: 0 });
        else if (e.shiftKey && el.scrollTop <= 0) el.scrollTo({ top: el.scrollHeight });
        else el.scrollBy({ top: (e.shiftKey ? -1 : 1) * el.clientHeight * 0.85, behavior: "smooth" });
        break;
      }
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
    <div className="transcript">
      <header className="t-head" data-tauri-drag-region>
        <div className="t-eyebrow">
          <span className={`src src-${session.source}`}>{SOURCE_LABEL[session.source]}</span>
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
        </div>
        <h1 className="t-title">{session.title}</h1>
        {session.title_source !== "prompt" && session.first_prompt && (
          <p className="t-prompt">{session.first_prompt.split("\n").find((l) => l.trim()) ?? ""}</p>
        )}
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
        <div className="t-actions">
          <button onClick={() => copy("resume", resumeCommand(session))}>
            {copied === "resume" ? "Copied" : "Copy resume command"}
          </button>
          <button onClick={() => api.reveal(session.path)}>Reveal file</button>
          <button onClick={() => copy("path", session.path)}>{copied === "path" ? "Copied" : "Copy path"}</button>
          <span className="t-file" title={session.path}>
            {tildify(session.cwd)} · {bytes(session.size)}
          </span>
        </div>
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
        {errorBlocks.length > 0 && (
          <div className="err-nav" title="Jump between failed tool calls and errors">
            <span className="err-dot" />
            <span className="err-label">
              {errIdx === null ? errorBlocks.length : `${errIdx + 1}/${errorBlocks.length}`}{" "}
              {errorBlocks.length === 1 ? "error" : "errors"}
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
                  className={`row ${matchSet.has(item.index) ? "hit" : ""} ${isCurrent ? "hit-current" : ""} ${item.index === focusIndex || (item.index === errorTarget && b.type !== "tools") ? "focused" : ""}`}
                  style={{ transform: `translateY(${item.start}px)` }}
                >
                  <BlockView block={b} onOpen={onOpen} openErrors={item.index === errorTarget} />
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
  onOpen,
  openErrors,
}: {
  block: Block;
  onOpen: (path: string, focus?: string) => void;
  /** Expand failed tool calls (set while the error navigator points here). */
  openErrors?: boolean;
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
      return <Thinking ev={b.ev} />;
    case "tools":
      return <Tools steps={b.steps} openErrors={openErrors} />;
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
      return <Compaction ev={b.ev} />;
    case "orphan_result":
      return (
        <section className="b-tools">
          <Output text={b.ev.text} isError={b.ev.is_error} lines={12} />
        </section>
      );
  }
});

function Thinking({ ev }: { ev: SessionEvent }) {
  const [open, setOpen] = useState(false);
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

function Compaction({ ev }: { ev: SessionEvent }) {
  const [open, setOpen] = useState(false);
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

function Tools({ steps, openErrors }: { steps: ToolStep[]; openErrors?: boolean }) {
  const errors = steps.filter((s) => s.result?.is_error).length;
  return (
    <section className="b-tools">
      {steps.length > 2 && (
        <div className="b-tools-head">
          {steps.length} tool calls{errors ? <span className="err"> · {errors} failed</span> : null}
        </div>
      )}
      {steps.map((s, i) => (
        <ToolRow key={s.call.call_id ?? i} step={s} forceOpen={openErrors && s.result?.is_error} />
      ))}
    </section>
  );
}

function ToolRow({ step, forceOpen }: { step: ToolStep; forceOpen?: boolean }) {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (forceOpen) setOpen(true);
  }, [forceOpen]);
  const { call, result } = step;
  const summary = useMemo(() => toolSummary(call.tool_name, call.text), [call]);
  const status = !result ? "pending" : result.is_error ? "error" : "ok";
  return (
    // In a group of tool calls, the error navigator marks only the failed call.
    <div className={`tool ${open ? "open" : ""} ${status} ${forceOpen ? "focused-step" : ""}`}>
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

function Images({ images }: { images?: string[] }) {
  const [zoom, setZoom] = useState<string | null>(null);
  if (!images?.length) return null;
  return (
    <div className="imgs">
      {images.map((src, i) =>
        src.startsWith("omitted") ? (
          <span key={i} className="img-omitted">image omitted (too large)</span>
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
