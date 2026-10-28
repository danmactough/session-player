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

interface Props {
  session: SessionRow;
  parent?: SessionRow;
  children: SessionRow[];
  highlight: string;
  onOpen: (path: string) => void;
}

export function Transcript({ session, parent, children, highlight, onOpen }: Props) {
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
      .loadSession(session.path)
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
  }, [session.path, session.size, session.updated_at]);

  useEffect(() => {
    setFind(highlight);
    setFindIdx(0);
  }, [highlight, session.path]);

  const blocks = useMemo(() => {
    if (!events) return [];
    return buildBlocks(events).filter((b) => {
      if (!showTools && (b.type === "tools" || b.type === "orphan_result")) return false;
      if (!showThinking && b.type === "thinking") return false;
      return true;
    });
  }, [events, showTools, showThinking]);

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
  }, [session.path]);

  // Jump to the first in-session match once the transcript has loaded.
  useEffect(() => {
    if (matches.length && events) {
      const i = Math.min(findIdx, matches.length - 1);
      virt.scrollToIndex(matches[i], { align: "center" });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [matches, findIdx, events]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        setFindOpen(true);
        requestAnimationFrame(() => findRef.current?.select());
      }
    };
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
            </dd>
          </div>
          <div>
            <dt>Tokens</dt>
            <dd title={`${session.input_tokens.toLocaleString()} input · ${session.cache_read_tokens.toLocaleString()} cache read · ${session.output_tokens.toLocaleString()} output`}>
              {compact(totalIn)} <span className="dim">in</span> · {compact(session.output_tokens)} <span className="dim">out</span>
            </dd>
          </div>
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
                ↖ Parent: {parent.title}
              </button>
            )}
            {children.length > 0 && (
              <span className="kids">
                {children.length} subagent{children.length === 1 ? "" : "s"}:
                {children.slice(0, 8).map((c) => (
                  <button key={c.path} className="link" onClick={() => onOpen(c.path)}>
                    {c.title}
                  </button>
                ))}
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
        <div className="t-scroll" ref={scrollRef}>
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
                  className={`row ${matchSet.has(item.index) ? "hit" : ""} ${isCurrent ? "hit-current" : ""}`}
                  style={{ transform: `translateY(${item.start}px)` }}
                >
                  <BlockView block={b} />
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
                <span className="o-t">{b.ev.text.slice(0, 90) || "(image)"}</span>
              </button>
            ))}
          </nav>
        )}
      </div>
    </div>
  );
}

const BlockView = memo(function BlockView({ block: b }: { block: Block }) {
  switch (b.type) {
    case "user":
      return (
        <section className={`b-user ${b.ev.sidechain ? "side" : ""}`}>
          <div className="b-label">
            <span>{b.ev.sidechain ? "Delegated" : `Prompt ${b.turn}`}</span>
            <time>{clock(b.ev.ts)}</time>
          </div>
          {b.ev.text && <div className="b-user-text">{b.ev.text}</div>}
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
      return <Tools steps={b.steps} />;
    case "command":
      return (
        <section className="b-command">
          <code className="cmd">{b.ev.text}</code>
          {b.output && <Output text={b.output.text} isError={b.output.is_error} lines={12} />}
        </section>
      );
    case "system":
      return <div className={`b-system ${b.ev.is_error ? "err" : ""}`}>{b.ev.text}</div>;
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

function Tools({ steps }: { steps: ToolStep[] }) {
  const errors = steps.filter((s) => s.result?.is_error).length;
  return (
    <section className="b-tools">
      {steps.length > 2 && (
        <div className="b-tools-head">
          {steps.length} tool calls{errors ? <span className="err"> · {errors} failed</span> : null}
        </div>
      )}
      {steps.map((s, i) => (
        <ToolRow key={s.call.call_id ?? i} step={s} />
      ))}
    </section>
  );
}

function ToolRow({ step }: { step: ToolStep }) {
  const [open, setOpen] = useState(false);
  const { call, result } = step;
  const summary = useMemo(() => toolSummary(call.tool_name, call.text), [call]);
  const status = !result ? "pending" : result.is_error ? "error" : "ok";
  return (
    <div className={`tool ${open ? "open" : ""} ${status}`}>
      <button className="tool-head" onClick={() => setOpen((o) => !o)}>
        <span className="dot" />
        <span className="tool-name">{toolLabel(call.tool_name)}</span>
        <span className="tool-sum">{summary}</span>
        {result?.images?.length ? <span className="tool-flag">image</span> : null}
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
