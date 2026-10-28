import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import type { IndexStats, SearchHit, SessionRow, SortKey, Source } from "./types";
import { basename, dirname, relTime, setHome, SOURCE_LABEL, tildify } from "./lib/format";
import { SessionList } from "./components/SessionList";
import { Transcript } from "./components/Transcript";
import { SourcesSheet } from "./components/SourcesSheet";
import { ancestors, branchChildren, branchRows, branchSourcePath, buildTree, type ListRow } from "./lib/tree";
import { useScrollAnchor } from "./lib/useScrollAnchor";

const ALL_SOURCES: Source[] = ["claude", "codex", "pi"];
const SORTS: { key: SortKey; label: string }[] = [
  { key: "recent", label: "Recent" },
  { key: "oldest", label: "Oldest" },
  { key: "longest", label: "Most prompts" },
  { key: "tokens", label: "Most tokens" },
];

interface Prefs {
  project: string | null;
  sort: SortKey;
  sources: Source[];
  showSub: boolean;
  selected: string | null;
}
const DEFAULT_PREFS: Prefs = { project: null, sort: "recent", sources: ALL_SOURCES, showSub: false, selected: null };

export default function App() {
  const [sessions, setSessions] = useState<SessionRow[] | null>(null);
  const [prefs, setPrefs] = useState<Prefs | null>(null);
  const [query, setQuery] = useState("");
  // Like Slack: a search started inside a project is scoped to it until the
  // user widens it to all projects.
  const [searchAll, setSearchAll] = useState(false);
  const [hits, setHits] = useState<Map<string, SearchHit> | null>(null);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [stats, setStats] = useState<IndexStats | null>(null);
  const [indexError, setIndexError] = useState<string | null>(null);
  const [sheet, setSheet] = useState(false);
  const [version, setVersion] = useState(0);
  const [projectFilter, setProjectFilter] = useState("");
  const searchRef = useRef<HTMLInputElement>(null);
  const projectsRef = useRef<HTMLElement>(null);
  // Sessions whose subagents, forks and branches are shown in the list.
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());

  const reload = useCallback(() => {
    api
      .listSessions()
      .then((rows) => {
        setSessions(rows);
      })
      .catch((e) => {
        setIndexError(String(e));
        api.log("error", `list_sessions failed: ${e}`);
      });
    setVersion((v) => v + 1);
  }, []);

  useEffect(() => {
    api.homeDir().then(setHome);
    api
      .getSetting("prefs")
      .then((raw) => setPrefs(raw ? { ...DEFAULT_PREFS, ...JSON.parse(raw) } : DEFAULT_PREFS))
      .catch(() => setPrefs(DEFAULT_PREFS));
    reload();
    const un = [
      listen<{ done: number; total: number }>("index-progress", (e) => setProgress(e.payload)),
      listen<IndexStats>("index-done", (e) => {
        setProgress(null);
        setStats(e.payload);
        setIndexError(null);
        const s = e.payload;
        // scanned === 0 is the synthetic event sent after a folder is removed.
        if (s.updated || s.removed || s.scanned === 0) reload();
      }),
      listen<string>("index-error", (e) => {
        setProgress(null);
        setIndexError(e.payload);
      }),
    ];
    return () => un.forEach((p) => p.then((f) => f()));
  }, [reload]);

  const update = useCallback((patch: Partial<Prefs>) => {
    setPrefs((p) => {
      const next = { ...(p ?? DEFAULT_PREFS), ...patch };
      void api.setSetting("prefs", JSON.stringify(next));
      return next;
    });
  }, []);

  // Full-text search runs in SQLite; title/path matching runs here so typing
  // feels instant even before the FTS results return.
  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setHits(null);
      return;
    }
    const t = setTimeout(() => {
      api
        .search(q)
        .then((r) => setHits(new Map(r.map((h) => [h.path, h]))))
        .catch(() => setHits(new Map()));
    }, 140);
    return () => clearTimeout(t);
  }, [query, version]);

  const p = prefs ?? DEFAULT_PREFS;

  const base = useMemo(
    () => (sessions ?? []).filter((s) => p.sources.includes(s.source) && (p.showSub || !s.is_subagent)),
    [sessions, p.sources, p.showSub],
  );

  const projects = useMemo(() => {
    const m = new Map<string, { project: string; count: number; latest: number; sources: Set<Source> }>();
    for (const s of base) {
      const e = m.get(s.project) ?? { project: s.project, count: 0, latest: 0, sources: new Set<Source>() };
      e.count++;
      e.latest = Math.max(e.latest, s.updated_at ?? 0);
      e.sources.add(s.source);
      m.set(s.project, e);
    }
    return [...m.values()].sort((a, b) => b.latest - a.latest);
  }, [base]);

  const visibleProjects = useMemo(() => {
    const f = projectFilter.trim().toLowerCase();
    return f ? projects.filter((x) => x.project.toLowerCase().includes(f)) : projects;
  }, [projects, projectFilter]);

  const searching = query.trim() !== "";
  const scopeProject = searching && searchAll ? null : p.project;

  useEffect(() => {
    if (!searching) setSearchAll(false);
  }, [searching]);
  useEffect(() => setSearchAll(false), [p.project]);

  const matchesQuery = useMemo(() => {
    const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    return (s: SessionRow) => {
      if (hits?.has(s.path)) return true;
      const hay = `${s.title} ${s.first_prompt ?? ""} ${s.cwd ?? ""} ${s.git_branch ?? ""} ${s.native_id}`.toLowerCase();
      return terms.every((t) => hay.includes(t));
    };
  }, [query, hits]);

  const matchCounts = useMemo(() => {
    if (!searching) return { project: 0, all: 0 };
    const all = base.filter(matchesQuery);
    return { all: all.length, project: p.project ? all.filter((s) => s.project === p.project).length : all.length };
  }, [base, matchesQuery, searching, p.project]);

  const clearSearch = useCallback(() => {
    setQuery("");
    searchRef.current?.focus();
  }, []);

  const list = useMemo(() => {
    let out = scopeProject ? base.filter((s) => s.project === scopeProject) : base;
    const q = query.trim().toLowerCase();
    if (q) {
      const terms = q.split(/\s+/);
      out = out.filter(matchesQuery);
      const rank = (s: SessionRow) => {
        const title = s.title.toLowerCase();
        const inTitle = terms.every((t) => title.includes(t)) ? -1000 : 0;
        return inTitle + (hits?.get(s.path)?.rank ?? 0);
      };
      return [...out].sort((a, b) => rank(a) - rank(b) || (b.updated_at ?? 0) - (a.updated_at ?? 0));
    }
    const sorted = [...out];
    switch (p.sort) {
      case "recent":
        sorted.sort((a, b) => (b.updated_at ?? 0) - (a.updated_at ?? 0));
        break;
      case "oldest":
        sorted.sort((a, b) => (a.started_at ?? 0) - (b.started_at ?? 0));
        break;
      case "longest":
        sorted.sort((a, b) => b.user_turns - a.user_turns || b.tool_calls - a.tool_calls);
        break;
      case "tokens":
        sorted.sort((a, b) => b.output_tokens + b.input_tokens - (a.output_tokens + a.input_tokens));
        break;
    }
    return sorted;
  }, [base, scopeProject, p.sort, query, hits, matchesQuery]);

  // Search results stay flat; browsing nests children under their session.
  const rows = useMemo<ListRow[]>(
    () =>
      searching
        ? list.map((s) => ({ s, depth: 0, childCount: 0, childSummary: "", expanded: false }))
        : buildTree(list, expanded),
    [list, searching, expanded],
  );

  // Every openable row: real sessions plus one per pi branch.
  const allRows = useMemo(() => (sessions ?? []).flatMap((s) => [s, ...branchRows(s)]), [sessions]);
  const byPath = useMemo(() => new Map(allRows.map((s) => [s.path, s])), [allRows]);
  const selected = p.selected ? byPath.get(p.selected) : undefined;

  // Opening a nested session (from a link or a saved selection) expands its parents.
  useEffect(() => {
    if (!p.selected) return;
    const up = ancestors(p.selected, allRows);
    if (up.some((a) => !expanded.has(a))) setExpanded((e) => new Set([...e, ...up]));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [p.selected, allRows]);

  const toggleExpanded = useCallback((path: string) => {
    setExpanded((e) => {
      const next = new Set(e);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  const family = useMemo(() => {
    if (!selected || !sessions) return { parent: undefined, children: [] as SessionRow[] };
    if (selected.branch_of) {
      return {
        parent: byPath.get(branchSourcePath(selected)!),
        children: branchChildren(selected, byPath.get(selected.branch_of)),
      };
    }
    const parent = selected.parent_id
      ? sessions.find((s) => s.source === selected.source && s.native_id === selected.parent_id && s.path !== selected.path)
      : undefined;
    const children = [
      ...branchChildren(selected, undefined),
      ...sessions.filter((s) => s.source === selected.source && s.parent_id === selected.native_id && s.path !== selected.path),
    ];
    return { parent, children };
  }, [selected, sessions, byPath]);

  // A child whose split point the transcript should scroll to, set when
  // following a "branched from" link back to the source.
  const [focus, setFocus] = useState<string | null>(null);
  const select = useCallback(
    (path: string) => {
      setFocus(null);
      update({ selected: path });
    },
    [update],
  );
  const open = useCallback(
    (path: string, focusChild?: string) => {
      setFocus(focusChild ?? null);
      update({ selected: path });
    },
    [update],
  );

  // Keyboard: j/k or arrows move through the list, / or ⌘K focuses search.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = (e.target as HTMLElement)?.tagName === "INPUT";
      if ((e.key === "/" && !typing) || ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k")) {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
        return;
      }
      if (typing && e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      const dir = e.key === "j" || e.key === "ArrowDown" ? 1 : e.key === "k" || e.key === "ArrowUp" ? -1 : 0;
      if (!dir || !rows.length) return;
      if (typing && (e.target as HTMLElement) !== searchRef.current) return;
      e.preventDefault();
      const i = rows.findIndex((r) => r.s.path === p.selected);
      const next = rows[Math.max(0, Math.min(rows.length - 1, i < 0 ? 0 : i + dir))];
      if (next) select(next.s.path);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [rows, p.selected, select]);

  useScrollAnchor(projectsRef, [visibleProjects]);

  const toggleSource = (s: Source) => {
    const has = p.sources.includes(s);
    const next = has ? p.sources.filter((x) => x !== s) : [...p.sources, s];
    update({ sources: next.length ? next : ALL_SOURCES });
  };

  const counts = useMemo(() => {
    const c: Record<Source, number> = { claude: 0, codex: 0, pi: 0 };
    for (const s of sessions ?? []) if (p.showSub || !s.is_subagent) c[s.source]++;
    return c;
  }, [sessions, p.showSub]);

  const empty = sessions !== null && sessions.length === 0;

  return (
    <div className="app">
      <aside className="rail">
        <div className="rail-top" data-tauri-drag-region>
          <span className="brand" data-tauri-drag-region>
            session<span className="brand-dot">·</span>player
          </span>
        </div>
        <div className="search" onClick={() => searchRef.current?.focus()}>
          <input
            ref={searchRef}
            placeholder={p.project ? `Search ${basename(p.project)}` : "Search all projects"}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                if (query) setQuery("");
                else (e.target as HTMLInputElement).blur();
              }
            }}
          />
          {query ? (
            <button className="search-clear" aria-label="Clear search" title="Clear search (Esc)" onClick={clearSearch}>
              ×
            </button>
          ) : (
            <kbd>⌘K</kbd>
          )}
        </div>

        <div className="sources">
          {ALL_SOURCES.map((s) => (
            <button
              key={s}
              className={`srcpill src-${s} ${p.sources.includes(s) ? "on" : ""}`}
              onClick={() => toggleSource(s)}
              title={`Show ${SOURCE_LABEL[s]} sessions`}
            >
              <span className="sdot" />
              {SOURCE_LABEL[s]}
              <span className="n">{counts[s]}</span>
            </button>
          ))}
        </div>

        <div className="rail-section">
          <span>Projects</span>
          {projects.length > 8 && (
            <input
              className="proj-filter"
              placeholder="filter"
              value={projectFilter}
              onChange={(e) => setProjectFilter(e.target.value)}
            />
          )}
        </div>
        <nav className="projects" ref={projectsRef}>
          <button
            className={`proj ${p.project === null ? "active" : ""}`}
            data-anchor="(all)"
            onClick={() => update({ project: null })}
          >
            <span className="proj-name">All projects</span>
            <span className="proj-n">{base.length}</span>
          </button>
          {visibleProjects.map((x) => (
            <button
              key={x.project}
              className={`proj ${p.project === x.project ? "active" : ""}`}
              data-anchor={x.project}
              onClick={() => update({ project: x.project })}
              title={tildify(x.project)}
            >
              <span className="proj-name">
                {basename(x.project)}
                <span className="proj-dir">{dirname(x.project)}</span>
              </span>
              <span className="proj-dots">
                {ALL_SOURCES.filter((s) => x.sources.has(s)).map((s) => (
                  <span key={s} className={`sdot src-${s}`} />
                ))}
              </span>
              <span className="proj-n">{x.count}</span>
            </button>
          ))}
        </nav>

        <footer className="rail-foot">
          <button className="ghost" onClick={() => setSheet(true)}>
            Sources
          </button>
          <span className="status">
            {progress
              ? `Indexing ${progress.done}/${progress.total}`
              : indexError
                ? "Index error"
                : stats
                  ? `Indexed · ${stats.elapsed_ms} ms`
                  : ""}
          </span>
        </footer>
      </aside>

      <section className="middle">
        <div className="mid-head" data-tauri-drag-region>
          <div className="mid-title" data-tauri-drag-region>
            <h2 data-tauri-drag-region>
              {searching ? "Results" : p.project ? basename(p.project) : "All sessions"}
            </h2>
            <span className="mid-sub" data-tauri-drag-region>
              {searching
                ? `${list.length} ${list.length === 1 ? "session matches" : "sessions match"} “${query.trim()}”`
                : p.project
                  ? tildify(p.project)
                  : `${list.length} sessions across ${projects.length} projects`}
            </span>
          </div>
          {searching ? (
            <div className="mid-controls">
              {p.project && (
                <div className="segmented" role="radiogroup" aria-label="Search scope">
                  <button role="radio" aria-checked={!searchAll} className={!searchAll ? "on" : ""} onClick={() => setSearchAll(false)}>
                    {basename(p.project)} <span className="n">{matchCounts.project}</span>
                  </button>
                  <button role="radio" aria-checked={searchAll} className={searchAll ? "on" : ""} onClick={() => setSearchAll(true)}>
                    All projects <span className="n">{matchCounts.all}</span>
                  </button>
                </div>
              )}
              <button className="clear-link" onClick={clearSearch}>
                Clear search
              </button>
            </div>
          ) : (
            <div className="mid-controls">
              <select value={p.sort} onChange={(e) => update({ sort: e.target.value as SortKey })}>
                {SORTS.map((s) => (
                  <option key={s.key} value={s.key}>
                    {s.label}
                  </option>
                ))}
              </select>
              <label className="toggle">
                <input type="checkbox" checked={p.showSub} onChange={(e) => update({ showSub: e.target.checked })} />
                Subagents
              </label>
            </div>
          )}
        </div>
        {sessions === null ? (
          <div className="empty pulse">Loading index…</div>
        ) : list.length === 0 ? (
          <div className="empty">
            {empty ? (
              <>
                <p>No sessions indexed yet.</p>
                <button className="primary" onClick={() => setSheet(true)}>
                  Add a session folder
                </button>
              </>
            ) : searching ? (
              <>
                <p>
                  Nothing {scopeProject ? `in ${basename(scopeProject)} ` : ""}matches “{query.trim()}”.
                </p>
                {scopeProject && matchCounts.all > 0 ? (
                  <button className="ghost" onClick={() => setSearchAll(true)}>
                    Show {matchCounts.all} {matchCounts.all === 1 ? "match" : "matches"} in all projects
                  </button>
                ) : (
                  <button className="ghost" onClick={clearSearch}>
                    Clear search
                  </button>
                )}
              </>
            ) : (
              "No sessions in this view."
            )}
          </div>
        ) : (
          <SessionList
            rows={rows}
            selected={p.selected}
            onSelect={select}
            onToggle={toggleExpanded}
            hits={hits}
            showProject={!scopeProject}
            groupByDay={!query && (p.sort === "recent" || p.sort === "oldest")}
          />
        )}
      </section>

      <main className="detail">
        {selected ? (
          <Transcript
            key={selected.path}
            session={selected}
            parent={family.parent}
            children={family.children}
            highlight={query.trim()}
            onOpen={open}
            focus={focus}
          />
        ) : (
          <div className="placeholder" data-tauri-drag-region>
            <div className="ph-mark">¶</div>
            <p>Pick a session to read it.</p>
            {sessions && sessions.length > 0 && (
              <p className="dim">
                Last activity {relTime(Math.max(...sessions.map((s) => s.updated_at ?? 0)))} · j / k to move · / to search
              </p>
            )}
          </div>
        )}
      </main>

      {sheet && <SourcesSheet onClose={() => setSheet(false)} stats={stats} indexing={!!progress} version={version} />}
    </div>
  );
}
