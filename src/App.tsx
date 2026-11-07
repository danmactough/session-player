import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { message, open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import type { AnnotationKind, IndexStats, NoteSummary, OnConflict, Outcome, SearchHit, SessionRow, SortKey, Source } from "./types";
import { KINDS, KIND_LABEL, OUTCOME_LABEL, sessionKey } from "./lib/notes";
import { MultiMenu } from "./components/MultiMenu";

const OUTCOMES = Object.keys(OUTCOME_LABEL) as Outcome[];
import { basename, relTime, setHome, SOURCE_LABEL, tildify } from "./lib/format";
import { SessionList, type ClickHow, type ListMemo } from "./components/SessionList";
import { QueueStatus } from "./components/QueueStatus";
import { AnalyzeNotice } from "./components/AnalyzeNotice";
import { ReportList } from "./components/ReportList";
import { ReportView } from "./components/ReportView";
import { useReports } from "./lib/useReports";
import { useAnalyzeQueue } from "./lib/useAnalyzeQueue";
import { AnalysisView } from "./components/AnalysisView";
import { Transcript } from "./components/Transcript";
import { SettingsSheet } from "./components/SettingsSheet";
import { ShortcutsSheet } from "./components/ShortcutsSheet";
import { ExportSheet } from "./components/ExportSheet";
import { ImportSheet, type ImportState } from "./components/ImportSheet";
import { Menu } from "./components/Menu";
import { SourceIcon } from "./components/AppIcon";
import { applyTheme, storedTheme, type Theme } from "./lib/theme";
import { ProjectNav, type ProjectSummary } from "./components/ProjectNav";
import { UpdateBanner } from "./components/UpdateBanner";
import { useUpdater } from "./lib/useUpdater";
import { ancestors, branchChildren, branchRows, branchSourcePath, buildTree, type ListRow } from "./lib/tree";

const ALL_SOURCES: Source[] = ["claude", "codex", "pi"];
const NO_PICKS = new Set<string>();
const REPORT_SORTS: { key: "recent" | "oldest"; label: string }[] = [
  { key: "recent", label: "Recent" },
  { key: "oldest", label: "Oldest" },
];
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
  /** The user ticked "Don't show this again" on the notice Analyze shows before sending transcripts. */
  hideAnalyzeNotice: boolean;
  /** The summary under a transcript's title is hidden. */
  summaryCollapsed: boolean;
  /** Ask GitHub for a newer release when the app opens and periodically after that. */
  autoCheckUpdates: boolean;
  /** What the right-hand pane shows: the selected session, or the Analysis view. */
  view: View;
  /** Each view's own sidebar and list choices (the keys in LIST_KEYS), so
   * switching views and back finds everything as it was. */
  lists: Record<View, ListPrefs>;
  /** What the Analysis view shows: statistics on the sessions, or saved reports. */
  analysisTab: "statistics" | "reports";
  /** The report open on the Reports tab. */
  openReport: string | null;
  reportSort: "recent" | "oldest";
}

type View = "session" | "analysis";
// Read and written through `p` and `update` as if they were top-level
// prefs; they belong to the view showing.
const LIST_KEYS = ["projects", "sort", "sources", "errorsOnly", "selected", "outcomeFilter", "kindFilter"] as const;
type ListPrefs = Pick<Prefs, (typeof LIST_KEYS)[number]>;
const pickList = (from: Prefs): ListPrefs => ({
  projects: from.projects,
  sort: from.sort,
  sources: from.sources,
  errorsOnly: from.errorsOnly,
  selected: from.selected,
  outcomeFilter: from.outcomeFilter,
  kindFilter: from.kindFilter,
});

/** State that lasts only while the app runs, kept apart for each view. */
function usePerView<T>(view: View, init: () => T): [T, (next: T | ((prev: T) => T)) => void] {
  const [all, setAll] = useState<Record<View, T>>(() => ({ session: init(), analysis: init() }));
  const set = useCallback(
    (next: T | ((prev: T) => T)) =>
      setAll((a) => ({ ...a, [view]: typeof next === "function" ? (next as (prev: T) => T)(a[view]) : next })),
    [view],
  );
  return [all[view], set];
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
  hideAnalyzeNotice: false,
  summaryCollapsed: false,
  autoCheckUpdates: true,
  view: "session",
  lists: { session: {} as ListPrefs, analysis: {} as ListPrefs },
  analysisTab: "statistics",
  openReport: null,
  reportSort: "recent",
};
DEFAULT_PREFS.lists = { session: pickList(DEFAULT_PREFS), analysis: pickList(DEFAULT_PREFS) };

export default function App() {
  const [rawSessions, setSessions] = useState<SessionRow[] | null>(null);
  const [prefs, setPrefs] = useState<Prefs | null>(null);
  const view: View = prefs?.view ?? "session";
  const [query, setQuery] = usePerView(view, () => "");
  // Like Slack: a search started inside a project is scoped to it until the
  // user widens it to all projects.
  const [searchAll, setSearchAll] = usePerView(view, () => false);
  const [hits, setHits] = useState<Map<string, SearchHit> | null>(null);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [stats, setStats] = useState<IndexStats | null>(null);
  // A short-lived line over the bottom of the project list, like a browser's link preview.
  const [notice, setNotice] = useState<{ text: string; shown: boolean }>({ text: "", shown: false });
  const noticeTimer = useRef<number | undefined>(undefined);
  const showNotice = useCallback((text: string, ms: number) => {
    window.clearTimeout(noticeTimer.current);
    setNotice({ text, shown: true });
    // Keep the text while it fades out.
    noticeTimer.current = window.setTimeout(() => setNotice((n) => ({ ...n, shown: false })), ms);
  }, []);
  const [sheet, setSheet] = useState(false);
  const [help, setHelp] = useState(false);
  const [exporting, setExporting] = useState<SessionRow | null>(null);
  const [importing, setImporting] = useState<{ path: string; state: ImportState } | null>(null);
  // Resolves the promise of an Analyze notice waiting for OK or Cancel.
  const [noticeAsk, setNoticeAsk] = useState<{ count: number; resolve: (ok: boolean) => void } | null>(null);
  const overlay = sheet || help || !!exporting || !!importing || !!noticeAsk;
  const [version, setVersion] = useState(0);
  const [projectFilter, setProjectFilter] = usePerView(view, () => "");
  const projectFilterRef = useRef<HTMLInputElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  // Sessions whose subagents, forks and branches are shown in the list.
  const [expanded, setExpanded] = usePerView<Set<string>>(view, () => new Set());
  // Where each view's session list and project list were scrolled to, restored on switching back.
  const scrollMemo = useRef<Record<View, { list: ListMemo; projects?: number }>>({ session: { list: {} }, analysis: { list: {} } });

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
        showNotice(`Index error: ${e}`, 8000);
        api.log("error", `list_sessions failed: ${e}`);
      });
    setVersion((v) => v + 1);
  }, [reloadNotes, showNotice]);

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
        // The Analyze notice used to be agreed to once, with no way to see it again.
        delete saved.generateConsent;
        const merged: Prefs = { ...DEFAULT_PREFS, ...saved };
        // Lists were shared by both views before each view kept its own.
        if (!saved.lists) merged.lists = { session: pickList(merged), analysis: { ...pickList(merged), selected: null } };
        setPrefs(merged);
      })
      .catch(() => setPrefs(DEFAULT_PREFS));
    reload();
    const un = [
      listen<{ done: number; total: number }>("index-progress", (e) => setProgress(e.payload)),
      listen<IndexStats>("index-done", (e) => {
        setProgress(null);
        setStats(e.payload);
        const s = e.payload;
        showNotice(`Indexed · ${s.elapsed_ms} ms`, 2500);
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
        showNotice(`Index error: ${e.payload}`, 8000);
      }),
    ];
    return () => un.forEach((p) => p.then((f) => f()));
  }, [reload, chooseBundle, reloadNotes, showNotice]);

  const update = useCallback((patch: Partial<Prefs>) => {
    setPrefs((prev) => {
      const cur = prev ?? DEFAULT_PREFS;
      const listPatch: Partial<ListPrefs> = {};
      const rest: Partial<Prefs> = {};
      for (const [k, v] of Object.entries(patch)) {
        if ((LIST_KEYS as readonly string[]).includes(k)) (listPatch as Record<string, unknown>)[k] = v;
        else (rest as Record<string, unknown>)[k] = v;
      }
      const next: Prefs = { ...cur, ...rest, lists: { ...cur.lists, [cur.view]: { ...cur.lists[cur.view], ...listPatch } } };
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

  const stored = prefs ?? DEFAULT_PREFS;
  const p: Prefs = { ...stored, ...stored.lists[stored.view] };
  const updater = useUpdater({ autoCheck: prefs !== null && p.autoCheckUpdates, onNotice: showNotice });
  const { checkNow } = updater;
  useEffect(() => {
    const un = listen("check-for-updates", () => void checkNow());
    return () => void un.then((f) => f());
  }, [checkNow]);

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
  }, [searching, setSearchAll]);
  const lastView = useRef(view);
  useEffect(() => {
    // Switching views brings back that view's projects; only a new choice of projects resets the scope.
    if (lastView.current !== view) {
      lastView.current = view;
      return;
    }
    setSearchAll(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [p.projects]);

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
  }, [setQuery]);

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
  // The transcript always shows the Sessions view's session, even while Analysis covers it.
  const sessionSelected = stored.lists.session.selected;
  const selected = sessionSelected ? byPath.get(sessionSelected) : undefined;

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
  }, [setExpanded]);

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

  // The Sessions view always has a session open: the first in the list when
  // none is chosen yet, or the chosen one is gone.
  useEffect(() => {
    if (view !== "session" || selected || !rows.length) return;
    select(rows[0].s.path);
  }, [view, selected, rows, select]);

  // Sessions picked in the Analysis view, by path. The Sessions view has only
  // its one open session.
  const [picked, setPicked] = useState<Set<string>>(() => new Set());
  const pickAnchor = useRef<string | null>(null);
  const clickRow = useCallback(
    (path: string, how: ClickHow) => {
      if (view === "session") {
        select(path);
        return;
      }
      // Like Finder: a click selects one session, ⌘-click adds or removes one,
      // and ⇧-click selects the run from the last session clicked.
      const paths = rows.map((r) => r.s.path);
      const from = paths.indexOf(pickAnchor.current ?? "");
      if (how.range && from >= 0) {
        const to = paths.indexOf(path);
        setPicked(new Set(paths.slice(Math.min(from, to), Math.max(from, to) + 1)));
        return;
      }
      pickAnchor.current = path;
      if (how.toggle) {
        setPicked((prev) => {
          const next = new Set(prev);
          if (next.has(path)) next.delete(path);
          else next.add(path);
          return next;
        });
        return;
      }
      setPicked(new Set([path]));
    },
    [view, rows, select],
  );
  const clearPicked = useCallback(() => {
    pickAnchor.current = null;
    setPicked(new Set());
  }, []);
  const pickedRows = useMemo(() => [...picked].map((path) => byPath.get(path)).filter((s): s is SessionRow => !!s), [picked, byPath]);
  const queue = useAnalyzeQueue();
  // A report that finishes opens on the Reports tab.
  const reports = useReports((id) => update({ analysisTab: "reports", openReport: id }));
  const reportsTab = view === "analysis" && p.analysisTab === "reports";
  // The backend lists reports newest first.
  const reportList = useMemo(() => (p.reportSort === "oldest" ? [...reports.list].reverse() : reports.list), [reports.list, p.reportSort]);
  // With no reports (and none being written) there's nothing to show on the Reports tab.
  const noReports = reports.loaded && !reports.list.length && !reports.jobs.length;
  useEffect(() => {
    if (noReports && p.analysisTab === "reports") update({ analysisTab: "statistics" });
  }, [noReports, p.analysisTab, update]);
  const reportPicked = useCallback(async () => {
    if (!pickedRows.length) return;
    await reports.start(pickedRows.map((s) => ({ key: sessionKey(s), title: s.title })));
    clearPicked();
    update({ analysisTab: "reports" });
  }, [pickedRows, reports, clearPicked, update]);
  // A report cites sessions by key; the Sessions view opens the session's own row, not a pi branch.
  const rowByKey = useMemo(() => {
    const m = new Map<string, SessionRow>();
    for (const s of sessions ?? []) if (!m.has(sessionKey(s))) m.set(sessionKey(s), s);
    return m;
  }, [sessions]);
  const openSessionByKey = useCallback(
    (key: string) => {
      const row = rowByKey.get(key);
      if (!row) return;
      update({ view: "session" });
      update({ selected: row.path });
    },
    [rowByKey, update],
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
      if (reportsTab) {
        const dir = typing || e.metaKey || e.ctrlKey || e.altKey ? 0 : e.key === "j" || e.key === "ArrowDown" ? 1 : e.key === "k" || e.key === "ArrowUp" ? -1 : 0;
        const list = reportList;
        if (!dir || !list.length) return;
        e.preventDefault();
        const i = list.findIndex((r) => r.id === p.openReport);
        update({ openReport: list[i < 0 ? 0 : (i + dir + list.length) % list.length].id });
        return;
      }
      if (view === "analysis" && !typing && (e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "a") {
        e.preventDefault();
        // The same sessions as "Select all" in the bar, including subagents folded under their session.
        setPicked(new Set(list.map((s) => s.path)));
        return;
      }
      if (e.key === "Escape" && !typing && view === "analysis" && picked.size > 0) {
        clearPicked();
        return;
      }
      if (typing && e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      // ⌘↑ / ⌘↓ and ⌥↑ / ⌥↓ scroll the transcript instead.
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const dir = e.key === "j" || e.key === "ArrowDown" ? 1 : e.key === "k" || e.key === "ArrowUp" ? -1 : 0;
      if (!dir || !rows.length) return;
      if (typing && (e.target as HTMLElement) !== searchRef.current) return;
      e.preventDefault();
      // In Analysis, j/k move a single selection from the last session clicked.
      const current = view === "analysis" ? pickAnchor.current : p.selected;
      const i = rows.findIndex((r) => r.s.path === current);
      // Wraps from the last session to the first and back.
      const next = rows[i < 0 ? 0 : (i + dir + rows.length) % rows.length];
      if (!next) return;
      if (view === "analysis") clickRow(next.s.path, { toggle: false, range: false });
      else select(next.s.path);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [rows, list, p.selected, select, overlay, picked.size, view, clickRow, clearPicked, reportsTab, reportList, p.openReport, update]);

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

  const confirmGenerate = useCallback(
    (count = 1) => (p.hideAnalyzeNotice ? Promise.resolve(true) : new Promise<boolean>((resolve) => setNoticeAsk({ count, resolve }))),
    [p.hideAnalyzeNotice],
  );
  const analyzePicked = useCallback(async () => {
    if (!pickedRows.length || !(await confirmGenerate(pickedRows.length))) return;
    let reanalyze = false;
    const analyzed = pickedRows.filter((s) => notes.get(sessionKey(s))?.analyzed).length;
    if (analyzed > 0) {
      const again = "Analyze Again";
      const changed = "Only If Changed";
      const choice = await message(
        `Analyze has already run on ${analyzed === pickedRows.length ? (analyzed === 1 ? "this session" : analyzed === 2 ? "both of these sessions" : `all ${analyzed} of these sessions`) : `${analyzed} of these ${pickedRows.length} sessions`}. Analyze ${analyzed === 1 ? "it" : "them"} again, or only if ${analyzed === 1 ? "its transcript has" : "a transcript has"} changed since? Only what Analyze wrote and no one edited is replaced.`,
        { title: "Analyze sessions again?", kind: "info", buttons: { yes: again, no: changed, cancel: "Cancel" } },
      );
      if (choice !== again && choice !== changed) return;
      reanalyze = choice === again;
    }
    queue.enqueue(pickedRows, reanalyze);
    clearPicked();
  }, [pickedRows, confirmGenerate, queue, notes, clearPicked]);
  const unanalyzed = useMemo(() => list.filter((s) => !notes.get(sessionKey(s))?.analyzed), [list, notes]);

  // On the heading's line in the Analysis view's middle column, level with the sidebar's switch.
  const analysisSwitch = view === "analysis" && (
    <div className={`view-switch ${reportsTab ? "right" : ""}`} role="radiogroup" aria-label="Analysis shows">
      <span className="view-thumb" aria-hidden="true" />
      <button role="radio" aria-checked={!reportsTab} className={!reportsTab ? "on" : ""} onClick={() => update({ analysisTab: "statistics" })}>
        Statistics
      </button>
      <button
        role="radio"
        aria-checked={reportsTab}
        className={reportsTab ? "on" : ""}
        onClick={() => update({ analysisTab: "reports" })}
        disabled={noReports}
        title={noReports ? "No reports yet: select sessions and click Report" : undefined}
      >
        Reports
      </button>
    </div>
  );

  return (
    <div className="app">
      <aside className="rail">
        <div className="rail-top" data-tauri-drag-region>
          <div
            className={`view-switch ${p.view === "analysis" ? "right" : ""}`}
            role="radiogroup"
            aria-label="View"
            title="Switch between reading sessions and the Analysis view"
          >
            <span className="view-thumb" aria-hidden="true" />
            <button role="radio" aria-checked={p.view === "session"} className={p.view === "session" ? "on" : ""} onClick={() => update({ view: "session" })}>
              Sessions
            </button>
            <button role="radio" aria-checked={p.view === "analysis"} className={p.view === "analysis" ? "on" : ""} onClick={() => update({ view: "analysis" })}>
              Analysis
            </button>
          </div>
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
          {ALL_SOURCES.map((s) => {
            const on = p.sources.includes(s);
            const alone = on && p.sources.length === 1;
            const label = SOURCE_LABEL[s];
            return (
              <div key={s} className={`srcpill-wrap ${alone ? "" : "can-only"}`}>
                <button
                  className={`srcpill src-${s} ${on ? "on" : ""}`}
                  onClick={(e) => (e.altKey ? update({ sources: [s] }) : toggleSource(s))}
                  title={alone ? "Show all agents" : `${on ? "Hide" : "Show"} ${label} sessions (⌥-click to show only ${label})`}
                >
                  <SourceIcon source={s} />
                  {label}
                  <span className="n">{counts[s]}</span>
                </button>
                {!alone && (
                  <button className="srcpill-only" onClick={() => update({ sources: [s] })} title={`Show only ${label}`} aria-label={`Show only ${label}`}>
                    only
                  </button>
                )}
              </div>
            );
          })}
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
          // One per view, so each comes back scrolled where it was left.
          key={view}
          initialScroll={scrollMemo.current[view].projects}
          onScrollTop={(top) => (scrollMemo.current[view].projects = top)}
          projects={visibleProjects}
          total={filtered.length}
          selected={p.projects}
          onSelect={(projects) => update({ projects })}
          onHide={hideProject}
          // Dragging a filtered subset would leave the other projects' places ambiguous.
          onReorder={projectFilter.trim() ? undefined : reorderProjects}
          resetKey={projectListKey}
        />

        {!updater.bannerHidden && (
          <UpdateBanner
            status={updater.status}
            onInstall={() => void updater.install()}
            onRestart={() => void updater.restart()}
            onLater={updater.later}
          />
        )}
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
          {queue.items.length > 0 && (
            <QueueStatus
              items={queue.items}
              onOpen={(path) => (view === "analysis" ? clickRow(path, { toggle: false, range: false }) : select(path))}
              onCancel={queue.cancel} onCancelAll={queue.cancelAll} onClear={queue.clearFinished} />
          )}
          <div className={`rail-status ${notice.shown ? "on" : ""}`} role="status" title={notice.text}>
            {notice.text}
          </div>
        </footer>
      </aside>

      <section className="middle">
        {reportsTab ? (
          <>
            <div className="mid-head" data-tauri-drag-region>
              <div className="mid-title" data-tauri-drag-region>
                <div className="mid-title-row" data-tauri-drag-region>
                  <h2 data-tauri-drag-region>Reports</h2>
                  {analysisSwitch}
                </div>
                <div className="mid-subrow" data-tauri-drag-region>
                  <span className="mid-sub" data-tauri-drag-region>
                    {reports.list.length ? `${reports.list.length} ${reports.list.length === 1 ? "report" : "reports"}` : "None yet"}
                  </span>
                  <Menu
                    className="caps-sort"
                    label="Sort reports"
                    value={p.reportSort}
                    options={REPORT_SORTS}
                    onChange={(reportSort) => update({ reportSort })}
                  />
                </div>
              </div>
            </div>
            <ReportList
              reports={reportList}
              jobs={reports.jobs}
              selected={p.openReport}
              onSelect={(openReport) => update({ openReport })}
              onCancel={reports.cancel}
              onDismiss={reports.dismiss}
            />
          </>
        ) : (
          <>
          <div className="mid-head" data-tauri-drag-region>
            <div className="mid-title" data-tauri-drag-region>
              <div className="mid-title-row" data-tauri-drag-region>
                <h2 data-tauri-drag-region>
                  {searching ? "Results" : p.projects.length ? selectionName : "All sessions"}
                </h2>
                {analysisSwitch}
              </div>
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
                    label="Outcome"
                    options={OUTCOMES.map((o) => ({ key: o, label: OUTCOME_LABEL[o] }))}
                    value={p.outcomeFilter}
                    onChange={(outcomeFilter) => update({ outcomeFilter })}
                  />
                  {kindOptions.length > 0 && (
                    <MultiMenu
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
              key={view}
              memo={scrollMemo.current[view].list}
              rows={rows}
              selected={view === "session" ? p.selected : null}
              picked={view === "analysis" ? picked : NO_PICKS}
              onBlankClick={view === "analysis" ? clearPicked : undefined}
              onSelect={clickRow}
              onToggle={toggleExpanded}
              hits={hits}
              showProject={!(scoped && p.projects.length === 1)}
              notes={notes}
              groupByDay={!query && (p.sort === "recent" || p.sort === "oldest")}
              resetKey={sessionListKey}
            />
          )}
          {/* At the bottom, so showing it doesn't move the rows being clicked. */}
          {view === "analysis" && picked.size > 0 && (
            <div className="pick-bar" role="toolbar" aria-label="Selected sessions">
              <div className="pick-row">
                <span className="pick-count">{picked.size} selected</span>
                <button className="primary pick-analyze" onClick={() => void analyzePicked()} disabled={!pickedRows.length}>
                  Analyze {pickedRows.length}
                </button>
                <button className="ghost pick-analyze" onClick={() => void reportPicked()} disabled={!pickedRows.length} title="Write a report on these sessions' outcomes and annotations">
                  Report
                </button>
                <button className="clear-link" onClick={clearPicked} title="Clear the selection (Esc)">
                  Clear
                </button>
              </div>
              <div className="pick-row">
                <button className="pick-link" onClick={() => setPicked(new Set(list.map((s) => s.path)))}>
                  Select all {list.length}
                </button>
                <button className="pick-link" onClick={() => setPicked(new Set(unanalyzed.map((s) => s.path)))} disabled={!unanalyzed.length}>
                  Select {unanalyzed.length} not analyzed
                </button>
              </div>
            </div>
          )}
          </>
        )}
      </section>

      <main className="detail">
        {reportsTab &&
          (p.openReport ? (
            <ReportView
              key={p.openReport}
              id={p.openReport}
              onOpenSession={openSessionByKey}
              canOpenSession={(key) => rowByKey.has(key)}
              onDeleted={() => {
                update({ openReport: null });
                reports.reload();
              }}
            />
          ) : (
            <div className="placeholder" data-tauri-drag-region>
              <div className="ph-mark">¶</div>
              <p>{reports.list.length ? "Pick a report to read it." : "No reports yet."}</p>
            </div>
          ))}
        {view === "analysis" && !reportsTab && (
          <AnalysisView
            // Picked sessions narrow the counts the way picking projects narrows the list.
            sessions={pickedRows.length ? pickedRows : list}
            scope={pickedRows.length ? "selection" : "list"}
            notes={notes}
            outcomeFilter={p.outcomeFilter}
            kindFilter={p.kindFilter}
            // Clicking the only value chosen clears it; anything else narrows the list to that value.
            onOutcome={(o) => update({ outcomeFilter: p.outcomeFilter.length === 1 && p.outcomeFilter[0] === o ? [] : [o] })}
            onKind={(k) => update({ kindFilter: p.kindFilter.length === 1 && p.kindFilter[0] === k ? [] : [k] })}
            onPickUnanalyzed={() => setPicked(new Set(unanalyzed.map((s) => s.path)))}
          />
        )}
        {/* Stays mounted under Analysis, so switching back finds the transcript exactly as it was left. */}
        <div className={`detail-session ${view === "analysis" ? "offstage" : ""}`} inert={view === "analysis"}>
          {selected ? (
            <Transcript
              key={selected.path}
              session={selected}
              parent={family.parent}
              children={family.children}
              highlight={query.trim()}
              onOpen={open}
              focus={focus}
              keyboard={!overlay && view === "session"}
              countExits={p.countExits}
              openWith={p.openWith[selected.source]}
              onOpenWith={(choice) => update({ openWith: { ...p.openWith, [selected.source]: choice } })}
              authorName={p.authorName.trim() || null}
              collapsed={p.summaryCollapsed}
              onCollapse={(summaryCollapsed) => update({ summaryCollapsed })}
              confirmGenerate={confirmGenerate}
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
        </div>
      </main>

      {help && <ShortcutsSheet onClose={() => setHelp(false)} />}
      {noticeAsk && (
        <AnalyzeNotice
          count={noticeAsk.count}
          onOk={(dontShowAgain) => {
            if (dontShowAgain) update({ hideAnalyzeNotice: true });
            noticeAsk.resolve(true);
            setNoticeAsk(null);
          }}
          onCancel={() => {
            noticeAsk.resolve(false);
            setNoticeAsk(null);
          }}
        />
      )}
      {exporting && (
        <ExportSheet
          session={exporting}
          authorName={p.authorName}
          onAuthorName={(authorName) => update({ authorName })}
          onClose={() => setExporting(null)}
        />
      )}
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
          showAnalyzeNotice={!p.hideAnalyzeNotice}
          onShowAnalyzeNotice={(show) => update({ hideAnalyzeNotice: !show })}
          hideSubagents={!p.showSub}
          onHideSubagents={(hide) => update({ showSub: !hide })}
          autoCheckUpdates={p.autoCheckUpdates}
          onAutoCheckUpdates={(autoCheckUpdates) => update({ autoCheckUpdates })}
          checkingForUpdates={updater.checking}
          updateCheckResult={updater.checkResult}
          onCheckForUpdates={() => void updater.checkNow()}
        />
      )}
    </div>
  );
}
