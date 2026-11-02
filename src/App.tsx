import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ask, open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import type { AnnotationKind, IndexStats, NoteSummary, OnConflict, Outcome, SearchHit, SessionRow, SortKey, Source } from "./types";
import { KINDS, KIND_LABEL, OUTCOME_LABEL, sessionKey } from "./lib/notes";
import { MultiMenu } from "./components/MultiMenu";

const OUTCOMES = Object.keys(OUTCOME_LABEL) as Outcome[];
import { basename, relTime, setHome, SOURCE_LABEL, tildify } from "./lib/format";
import { SessionList } from "./components/SessionList";
import { Transcript } from "./components/Transcript";
import { SettingsSheet } from "./components/SettingsSheet";
import { ShortcutsSheet } from "./components/ShortcutsSheet";
import { ExportSheet } from "./components/ExportSheet";
import { ImportSheet, type ImportState } from "./components/ImportSheet";
import { Menu } from "./components/Menu";
import { SourceIcon } from "./components/AppIcon";
import { applyTheme, storedTheme, type Theme } from "./lib/theme";
import { ProjectNav, type ProjectSummary } from "./components/ProjectNav";
import { ancestors, branchChildren, branchRows, branchSourcePath, buildTree, type ListRow } from "./lib/tree";

const ALL_SOURCES: Source[] = ["claude", "codex", "pi"];
const SORTS: { key: SortKey; label: string }[] = [
  { key: "recent", label: "Recent" },
  { key: "oldest", label: "Oldest" },
  { key: "longest", label: "Most prompts" },
  { key: "tokens", label: "Most tokens" },
];

type ProjectSort = "recent" | "stale" | "az" | "za" | "manual";
const PROJECT_SORTS: { key: ProjectSort; label: string }[] = [
  { key: "recent", label: "Recent" },
  { key: "stale", label: "Oldest" },
  { key: "az", label: "A–Z" },
  { key: "za", label: "Z–A" },
  { key: "manual", label: "Manual" },
];
const byName = (a: string, b: string) =>
  basename(a).localeCompare(basename(b), undefined, { sensitivity: "base", numeric: true }) || a.localeCompare(b);

interface Prefs {
  /** Selected projects; empty means all of them. */
  projects: string[];
  sort: SortKey;
  sources: Source[];
  showSub: boolean;
  errorsOnly: boolean;
  selected: string | null;
  projectSort: ProjectSort;
  projectOrder: string[];
  hiddenProjects: string[];
  /** Count shell commands that exited with a failing status as errors. */
  countExits: boolean;
  theme: Theme;
  /** What the transcript's resume button does for each agent's sessions:
   * "copy" (the default), or an app such as "claude". Kept per agent, so
   * choosing ChatGPT for a Codex session doesn't change Claude Code sessions. */
  openWith: Partial<Record<Source, string>>;
  /** Keeps only sessions with one of these outcomes; empty keeps all. */
  outcomeFilter: Outcome[];
  /** Keeps only sessions with an annotation of one of these kinds; empty keeps all. */
  kindFilter: AnnotationKind[];
  /** Shown as the author of annotations. */
  authorName: string;
  /** The user agreed that generating notes sends the transcript to Anthropic. */
  generateConsent: boolean;
  /** The summary under a transcript's title is hidden. */
  summaryCollapsed: boolean;
}
const DEFAULT_PREFS: Prefs = {
  projects: [],
  sort: "recent",
  sources: ALL_SOURCES,
  showSub: false,
  errorsOnly: false,
  selected: null,
  projectSort: "recent",
  projectOrder: [],
  hiddenProjects: [],
  countExits: false,
  theme: storedTheme(),
  openWith: {},
  outcomeFilter: [],
  kindFilter: [],
  authorName: "",
  generateConsent: false,
  summaryCollapsed: false,
};

export default function App() {
  const [rawSessions, setSessions] = useState<SessionRow[] | null>(null);
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
  const [help, setHelp] = useState(false);
  const [exporting, setExporting] = useState<SessionRow | null>(null);
  const [importing, setImporting] = useState<{ path: string; state: ImportState } | null>(null);
  const overlay = sheet || help || !!exporting || !!importing;
  const [version, setVersion] = useState(0);
  const [projectFilter, setProjectFilter] = useState("");
  const projectFilterRef = useRef<HTMLInputElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  // Sessions whose subagents, forks and branches are shown in the list.
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());

  const importBundle = useCallback(async (path: string, onConflict?: OnConflict) => {
    const file = basename(path);
    setImporting({ path, state: { status: "working" } });
    try {
      setImporting({ path, state: { status: "done", file, report: await api.importBundle(path, onConflict) } });
    } catch (e) {
      setImporting({ path, state: { status: "failed", file, error: String(e) } });
    }
  }, []);

  const chooseBundle = useCallback(async () => {
    const path = await openDialog({
      title: "Import session",
      multiple: false,
      directory: false,
      filters: [{ name: "Session bundle", extensions: ["spsession"] }],
    });
    if (typeof path === "string") await importBundle(path);
  }, [importBundle]);

  // Titles, outcomes, tags and annotation kinds people have added, by session key.
  const [notes, setNotes] = useState<Map<string, NoteSummary>>(() => new Map());
  const reloadNotes = useCallback(() => {
    api
      .noteSummaries()
      .then((list) => setNotes(new Map(list.map((n) => [n.key, n]))))
      .catch((e) => api.log("error", `note_summaries failed: ${e}`));
  }, []);

  const reload = useCallback(() => {
    reloadNotes();
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
  }, [reloadNotes]);

  useEffect(() => {
    api.homeDir().then(setHome);
    api
      .getSetting("prefs")
      .then((raw) => {
        const saved = raw ? JSON.parse(raw) : {};
        // openWith used to be one choice for every agent.
        if (typeof saved.openWith !== "object") delete saved.openWith;
        // A single selected project used to be stored as `project`.
        if (!Array.isArray(saved.projects)) saved.projects = saved.project ? [saved.project] : [];
        delete saved.project;
        setPrefs({ ...DEFAULT_PREFS, ...saved });
      })
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
      listen("import-session", () => void chooseBundle()),
      // Generated notes change titles, outcomes and search hits, even for a session no longer open.
      listen("generate-done", () => {
        reloadNotes();
        setVersion((v) => v + 1);
      }),
      // Settings… in the app menu (⌘,).
      listen("open-settings", () => {
        setHelp(false);
        setSheet(true);
      }),
      listen<string>("index-error", (e) => {
        setProgress(null);
        setIndexError(e.payload);
      }),
    ];
    return () => un.forEach((p) => p.then((f) => f()));
  }, [reload, chooseBundle, reloadNotes]);

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

  useEffect(() => applyTheme(p.theme), [p.theme]);

  // Folding the error setting in here keeps every count, badge and filter consistent.
  const sessions = useMemo(
    () =>
      p.countExits && rawSessions
        ? rawSessions.map((s) => (s.exit_count ? { ...s, error_count: s.error_count + s.exit_count } : s))
        : rawSessions,
    [rawSessions, p.countExits],
  );

  const hiddenSet = useMemo(() => new Set(p.hiddenProjects), [p.hiddenProjects]);

  // Choices within a filter are alternatives; the two filters both apply.
  const passesNoteFilter = useCallback(
    (s: SessionRow) => {
      const n = notes.get(sessionKey(s));
      if (p.outcomeFilter.length && !p.outcomeFilter.includes(n?.outcome ?? "unknown")) return false;
      if (p.kindFilter.length && !p.kindFilter.some((k) => n?.kinds.includes(k))) return false;
      return true;
    },
    [p.outcomeFilter, p.kindFilter, notes],
  );

  const base = useMemo(
    () =>
      (sessions ?? []).filter(
        (s) =>
          !hiddenSet.has(s.project) &&
          p.sources.includes(s.source) &&
          (p.showSub || !s.is_subagent) &&
          (!p.errorsOnly || s.error_count > 0) &&
          passesNoteFilter(s),
      ),
    [sessions, hiddenSet, p.sources, p.showSub, p.errorsOnly, passesNoteFilter],
  );

  const projects = useMemo(() => {
    const m = new Map<string, ProjectSummary>();
    for (const s of base) {
      const e = m.get(s.project) ?? { project: s.project, count: 0, latest: 0, sources: new Set<Source>() };
      e.count++;
      e.latest = Math.max(e.latest, s.updated_at ?? 0);
      e.sources.add(s.source);
      m.set(s.project, e);
    }
    const out = [...m.values()].sort((a, b) => b.latest - a.latest);
    switch (p.projectSort) {
      case "stale":
        return out.reverse();
      case "az":
        return out.sort((a, b) => byName(a.project, b.project));
      case "za":
        return out.sort((a, b) => byName(b.project, a.project));
      case "manual": {
        // Projects not yet placed by hand go first, newest first, so new work is easy to find.
        const pos = new Map(p.projectOrder.map((x, i) => [x, i]));
        return out.sort((a, b) => (pos.get(a.project) ?? -1) - (pos.get(b.project) ?? -1));
      }
    }
    return out;
  }, [base, p.projectSort, p.projectOrder]);

  const hiddenProjects = useMemo(() => {
    const counts = new Map<string, number>();
    for (const s of sessions ?? []) if (hiddenSet.has(s.project)) counts.set(s.project, (counts.get(s.project) ?? 0) + 1);
    return [...p.hiddenProjects].sort(byName).map((project) => ({ project, count: counts.get(project) ?? 0 }));
  }, [sessions, hiddenSet, p.hiddenProjects]);

  const hideProject = useCallback(
    (project: string) =>
      update({
        hiddenProjects: [...p.hiddenProjects.filter((x) => x !== project), project],
        projects: p.projects.filter((x) => x !== project),
      }),
    [update, p.hiddenProjects, p.projects],
  );
  const unhideProject = useCallback(
    (project: string) => update({ hiddenProjects: p.hiddenProjects.filter((x) => x !== project) }),
    [update, p.hiddenProjects],
  );
  const reorderProjects = useCallback(
    (order: string[]) => {
      // Keep the saved positions of projects not shown right now (hidden or filtered out by source).
      const shown = new Set(order);
      update({ projectSort: "manual", projectOrder: [...order, ...p.projectOrder.filter((x) => !shown.has(x))] });
    },
    [update, p.projectOrder],
  );

  const filterText = projectFilter.trim().toLowerCase();
  const visibleProjects = useMemo(
    () => (filterText ? projects.filter((x) => x.project.toLowerCase().includes(filterText)) : projects),
    [projects, filterText],
  );
  // "All projects" means the projects the filter box lets through.
  const filtered = useMemo(
    () => (filterText ? base.filter((s) => s.project.toLowerCase().includes(filterText)) : base),
    [base, filterText],
  );

  const searching = query.trim() !== "";
  const selectedSet = useMemo(() => new Set(p.projects), [p.projects]);
  const inSelection = useCallback((s: SessionRow) => selectedSet.has(s.project), [selectedSet]);
  // The sessions the list draws from: the selected projects, or everything the
  // project filter lets through (also while searching all projects).
  const scoped = p.projects.length > 0 && !(searching && searchAll);
  // Names the selection in headers and labels: one project's name, or a count.
  const selectionName =
    p.projects.length === 1 ? basename(p.projects[0]) : `${p.projects.length} projects`;

  useEffect(() => {
    if (!searching) setSearchAll(false);
  }, [searching]);
  useEffect(() => setSearchAll(false), [p.projects]);

  const matchesQuery = useMemo(() => {
    const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    return (s: SessionRow) => {
      if (hits?.has(s.path)) return true;
      const n = notes.get(sessionKey(s));
      const hay = `${s.title} ${n?.components.join(" ") ?? ""} ${s.first_prompt ?? ""} ${s.cwd ?? ""} ${s.git_branch ?? ""} ${s.native_id}`.toLowerCase();
      return terms.every((t) => hay.includes(t));
    };
  }, [query, hits, notes]);

  const matchCounts = useMemo(() => {
    if (!searching) return { project: 0, all: 0 };
    const all = filtered.filter(matchesQuery);
    const inProject = p.projects.length ? base.filter((s) => inSelection(s) && matchesQuery(s)).length : all.length;
    return { all: all.length, project: inProject };
  }, [base, filtered, matchesQuery, searching, p.projects, inSelection]);

  const clearSearch = useCallback(() => {
    setQuery("");
    searchRef.current?.focus();
  }, []);

  const list = useMemo(() => {
    let out = scoped ? base.filter(inSelection) : filtered;
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
  }, [base, filtered, scoped, inSelection, p.sort, query, hits, matchesQuery]);

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
    // An imported copy is related only to sessions imported with it, not to the local original.
    const kin = (s: SessionRow) => s.source === selected.source && s.from_bundle === selected.from_bundle && s.path !== selected.path;
    const parent = selected.parent_id ? sessions.find((s) => kin(s) && s.native_id === selected.parent_id) : undefined;
    const children = [...branchChildren(selected, undefined), ...sessions.filter((s) => kin(s) && s.parent_id === selected.native_id)];
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

  // Keyboard: j/k or arrows move through the list, / or ⌘K focuses search,
  // ? lists the shortcuts. Keep ShortcutsSheet in step with this.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Also in the app menu; this covers the browser preview.
      if ((e.metaKey || e.ctrlKey) && e.key === ",") {
        e.preventDefault();
        setHelp(false);
        setSheet(true);
        return;
      }
      // The open sheet handles its own keys; nothing behind it should react.
      if (overlay) return;
      const typing = (e.target as HTMLElement)?.tagName === "INPUT";
      if (e.key === "?" && !typing) {
        e.preventDefault();
        setHelp(true);
        return;
      }
      if ((e.key === "/" && !typing) || ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k")) {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
        return;
      }
      if (typing && e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      // ⌘↑ / ⌘↓ and ⌥↑ / ⌥↓ scroll the transcript instead.
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const dir = e.key === "j" || e.key === "ArrowDown" ? 1 : e.key === "k" || e.key === "ArrowUp" ? -1 : 0;
      if (!dir || !rows.length) return;
      if (typing && (e.target as HTMLElement) !== searchRef.current) return;
      e.preventDefault();
      const i = rows.findIndex((r) => r.s.path === p.selected);
      // Wraps from the last session to the first and back.
      const next = rows[i < 0 ? 0 : (i + dir + rows.length) % rows.length];
      if (next) select(next.s.path);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [rows, p.selected, select, overlay]);

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

  // Sessions the errors filter would keep, under every other filter.
  const errorSessions = useMemo(
    () =>
      (sessions ?? []).filter(
        (s) => s.error_count > 0 && !hiddenSet.has(s.project) && p.sources.includes(s.source) && (p.showSub || !s.is_subagent),
      ).length,
    [sessions, hiddenSet, p.sources, p.showSub],
  );

  // Only kinds some session has are offered (and any still chosen, so they can be cleared).
  const kindOptions = useMemo(
    () =>
      KINDS.filter((k) => p.kindFilter.includes(k) || [...notes.values()].some((n) => n.kinds.includes(k))).map((k) => ({
        key: k,
        label: KIND_LABEL[k],
      })),
    [notes, p.kindFilter],
  );
  const showNoteFilters = notes.size > 0 || p.outcomeFilter.length > 0 || p.kindFilter.length > 0;

  // Changing what a list shows starts it at the top; changes that happen
  // under the user (expanding a row, new sessions arriving, hiding or dragging
  // a project) keep the scroll position. A new object marks a reset.
  const projectListKey = useMemo(
    () => ({}),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [filterText, p.sources, p.errorsOnly, p.showSub, p.countExits, p.projectSort, p.outcomeFilter, p.kindFilter],
  );
  // Search results are ranked, so new full-text hits re-rank them from the top too.
  const sessionListKey = useMemo(
    () => ({}),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [p.projects, filterText, p.sources, p.errorsOnly, p.showSub, p.countExits, p.sort, query, searchAll, hits, p.outcomeFilter, p.kindFilter],
  );

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
            placeholder={p.projects.length ? `Search ${selectionName}` : "Search all projects"}
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
              <SourceIcon source={s} />
              {SOURCE_LABEL[s]}
              <span className="n">{counts[s]}</span>
            </button>
          ))}
          <button
            className={`srcpill filter-pill ${p.errorsOnly ? "on" : ""}`}
            onClick={() => update({ errorsOnly: !p.errorsOnly })}
            aria-pressed={p.errorsOnly}
            title="Show only sessions with errors"
          >
            <span className="sdot err" />
            Only with errors
            <span className="n">{errorSessions}</span>
          </button>
        </div>

        <div className="rail-section">
          <span>Projects</span>
          <Menu
            className="proj-sort"
            label="Sort projects"
            title="Sort projects (drag a project to arrange by hand)"
            value={p.projectSort}
            options={PROJECT_SORTS}
            onChange={(projectSort) => update({ projectSort })}
          />
        </div>
        {projects.length > 8 && (
          <div className="proj-filter" onClick={() => projectFilterRef.current?.focus()}>
            <input
              ref={projectFilterRef}
              placeholder="Filter projects"
              value={projectFilter}
              onChange={(e) => setProjectFilter(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") setProjectFilter("");
              }}
            />
            {projectFilter && (
              <button
                className="search-clear"
                aria-label="Clear project filter"
                title="Clear filter (Esc)"
                onClick={() => {
                  setProjectFilter("");
                  projectFilterRef.current?.focus();
                }}
              >
                ×
              </button>
            )}
          </div>
        )}
        <ProjectNav
          projects={visibleProjects}
          total={filtered.length}
          selected={p.projects}
          onSelect={(projects) => update({ projects })}
          onHide={hideProject}
          // Dragging a filtered subset would leave the other projects' places ambiguous.
          onReorder={projectFilter.trim() ? undefined : reorderProjects}
          resetKey={projectListKey}
        />

        <footer className="rail-foot">
          <button className="ghost icon-button" title="Settings (⌘,)" aria-label="Settings" onClick={() => setSheet(true)}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
              <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
              <circle cx="12" cy="12" r="3" />
            </svg>
          </button>
          <button className="ghost import-button" title="Import a shared session (⌘⇧I)" onClick={() => void chooseBundle()}>
            Import…
          </button>
          <button className="ghost help-button" title="Keyboard shortcuts (?)" aria-label="Keyboard shortcuts" onClick={() => setHelp(true)}>
            ?
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
              {searching ? "Results" : p.projects.length ? selectionName : "All sessions"}
            </h2>
            <div className="mid-subrow" data-tauri-drag-region>
              <span className="mid-sub" data-tauri-drag-region>
                {searching
                  ? `${list.length} ${list.length === 1 ? "session matches" : "sessions match"} “${query.trim()}”`
                  : p.projects.length === 1
                    ? tildify(p.projects[0])
                    : p.projects.length > 1
                      ? p.projects.map(basename).join(", ")
                      : `${list.length} ${list.length === 1 ? "session" : "sessions"} across ${visibleProjects.length} ${visibleProjects.length === 1 ? "project" : "projects"}`}
              </span>
              {/* Search results are ranked by relevance, so sorting only applies while browsing. */}
              {!searching && (
                <Menu className="caps-sort" label="Sort sessions" value={p.sort} options={SORTS} onChange={(sort) => update({ sort })} />
              )}
            </div>
            {showNoteFilters && (
              <div className="mid-filters">
                <MultiMenu
                  className="caps-sort"
                  label="Outcome"
                  options={OUTCOMES.map((o) => ({ key: o, label: OUTCOME_LABEL[o] }))}
                  value={p.outcomeFilter}
                  onChange={(outcomeFilter) => update({ outcomeFilter })}
                />
                {kindOptions.length > 0 && (
                  <MultiMenu
                    className="caps-sort"
                    label="Annotation type"
                    options={kindOptions}
                    value={p.kindFilter}
                    onChange={(kindFilter) => update({ kindFilter })}
                  />
                )}
              </div>
            )}
          </div>
          {searching && (
            <div className="mid-controls">
              {p.projects.length > 0 && (
                <div className="segmented" role="radiogroup" aria-label="Search scope">
                  <button role="radio" aria-checked={!searchAll} className={!searchAll ? "on" : ""} onClick={() => setSearchAll(false)}>
                    {selectionName} <span className="n">{matchCounts.project}</span>
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
                  Nothing {scoped ? `in ${selectionName} ` : ""}matches “{query.trim()}”.
                </p>
                {scoped && matchCounts.all > 0 ? (
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
            showProject={!(scoped && p.projects.length === 1)}
            notes={notes}
            groupByDay={!query && (p.sort === "recent" || p.sort === "oldest")}
            resetKey={sessionListKey}
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
            keyboard={!overlay}
            countExits={p.countExits}
            openWith={p.openWith[selected.source]}
            onOpenWith={(choice) => update({ openWith: { ...p.openWith, [selected.source]: choice } })}
            authorName={p.authorName.trim() || null}
            collapsed={p.summaryCollapsed}
            onCollapse={(summaryCollapsed) => update({ summaryCollapsed })}
            confirmGenerate={async () => {
              if (p.generateConsent) return true;
              const ok = await ask(
                "Generating notes sends this session's transcript to Anthropic, through the claude command and your Claude Code sign-in, so Claude Haiku can suggest details and annotations. Nothing is sent until you generate notes for a session.",
                { title: "Send this transcript to Anthropic?", kind: "info", okLabel: "Generate", cancelLabel: "Cancel" },
              );
              if (ok) update({ generateConsent: true });
              return ok;
            }}
            onNotesChanged={() => {
              reloadNotes();
              // Re-runs the full-text search, which covers notes.
              setVersion((v) => v + 1);
            }}
            onExport={() => setExporting(selected)}
          />
        ) : (
          <div className="placeholder" data-tauri-drag-region>
            <div className="ph-mark">¶</div>
            <p>Pick a session to read it.</p>
            {sessions && sessions.length > 0 && (
              <p className="dim">
                Last activity {relTime(Math.max(...sessions.map((s) => s.updated_at ?? 0)))} · j / k to move · / to search · ? for shortcuts
              </p>
            )}
          </div>
        )}
      </main>

      {help && <ShortcutsSheet onClose={() => setHelp(false)} />}
      {exporting && <ExportSheet session={exporting} onClose={() => setExporting(null)} />}
      {importing && (
        <ImportSheet
          state={importing.state}
          onResolve={(choice) => void importBundle(importing.path, choice)}
          onClose={() => setImporting(null)}
        />
      )}
      {sheet && (
        <SettingsSheet
          onClose={() => setSheet(false)}
          stats={stats}
          indexing={!!progress}
          version={version}
          hidden={hiddenProjects}
          onUnhide={unhideProject}
          countExits={p.countExits}
          onCountExits={(countExits) => update({ countExits })}
          theme={p.theme}
          onTheme={(theme) => update({ theme })}
          authorName={p.authorName}
          onAuthorName={(authorName) => update({ authorName })}
          hideSubagents={!p.showSub}
          onHideSubagents={(hide) => update({ showSub: !hide })}
        />
      )}
    </div>
  );
}
