import type { SessionRow } from "../types";

export type ChildKind = "subagent" | "fork" | "branch";

export interface ListRow {
  s: SessionRow;
  depth: number;
  kind?: ChildKind;
  /** Direct children (subagents, forks, branches), shown or not. */
  childCount: number;
  childSummary: string;
  expanded: boolean;
}

// An imported copy of a session sits beside the local original, so each
// nests only under its own kind of parent.
const origin = (s: SessionRow) => (s.from_bundle ? "bundle" : "local");
const idKey = (s: SessionRow) => `${origin(s)}|${s.source}|${s.native_id}`;
const parentKey = (s: SessionRow) => (s.parent_id ? `${origin(s)}|${s.source}|${s.parent_id}` : null);

/** One list row per alternate pi branch, opened through the parent's file. */
export function branchRows(s: SessionRow): SessionRow[] {
  return (s.branches ?? []).map((b) => ({
    ...s,
    path: `${s.path}#${b.leaf}`,
    title: b.label,
    title_source: "branch",
    first_prompt: null,
    started_at: b.started_at,
    updated_at: b.updated_at,
    user_turns: b.user_turns,
    assistant_msgs: 0,
    tool_calls: 0,
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    // Counted for the whole file; a branch has no count of its own.
    error_count: 0,
    exit_count: 0,
    branches: [],
    fork_entry: b.fork_entry,
    fork_turn: b.fork_turn,
    branch_of: s.path,
    branch_leaf: b.leaf,
    branch_parent: b.parent_leaf,
  }));
}

/** Branch rows that split directly from `s` (a session's active branch, or another branch). */
export function branchChildren(s: SessionRow, root: SessionRow | undefined): SessionRow[] {
  if (s.branch_of) return root ? branchRows(root).filter((b) => b.branch_parent === s.branch_leaf) : [];
  return branchRows(s).filter((b) => !b.branch_parent);
}

/** The path of the row a branch split from. */
export function branchSourcePath(b: SessionRow): string | undefined {
  if (!b.branch_of) return undefined;
  return b.branch_parent ? `${b.branch_of}#${b.branch_parent}` : b.branch_of;
}

export function childKind(s: SessionRow): ChildKind {
  return s.branch_of ? "branch" : s.is_subagent ? "subagent" : "fork";
}

function summarize(children: SessionRow[]): string {
  const n: Record<ChildKind, number> = { subagent: 0, fork: 0, branch: 0 };
  for (const c of children) n[childKind(c)]++;
  const part = (k: ChildKind, one: string, many: string) => (n[k] ? `${n[k]} ${n[k] === 1 ? one : many}` : "");
  return [part("branch", "branch", "branches"), part("subagent", "subagent", "subagents"), part("fork", "fork", "forks")]
    .filter(Boolean)
    .join(" · ");
}

/**
 * Nests subagent and fork sessions under the session that started them, and
 * pi branches under their session, keeping the order of `sessions` for the
 * top level. A child whose parent is not in `sessions` stays at the top level.
 */
export function buildTree(sessions: SessionRow[], expanded: Set<string>): ListRow[] {
  const byId = new Map(sessions.map((s) => [idKey(s), s]));
  const kids = new Map<string, SessionRow[]>();
  const roots: SessionRow[] = [];
  for (const s of sessions) {
    const pk = parentKey(s);
    const parent = pk ? byId.get(pk) : undefined;
    if (parent && parent !== s) {
      const list = kids.get(pk!) ?? [];
      list.push(s);
      kids.set(pk!, list);
    } else {
      roots.push(s);
    }
  }
  const byPath = new Map(sessions.map((s) => [s.path, s]));
  const out: ListRow[] = [];
  const seen = new Set<string>();
  const visit = (s: SessionRow, depth: number) => {
    if (seen.has(s.path)) return;
    seen.add(s.path);
    const children = [
      ...branchChildren(s, s.branch_of ? byPath.get(s.branch_of) : undefined),
      ...(s.branch_of ? [] : (kids.get(idKey(s)) ?? []).sort((a, b) => (a.started_at ?? 0) - (b.started_at ?? 0))),
    ];
    const open = expanded.has(s.path);
    out.push({
      s,
      depth,
      kind: depth ? childKind(s) : undefined,
      childCount: children.length,
      childSummary: summarize(children),
      expanded: open,
    });
    if (!open) return;
    for (const c of children) visit(c, depth + 1);
  };
  for (const r of roots) visit(r, 0);
  return out;
}

/** Paths of the sessions above `path` in the tree, nearest first. */
export function ancestors(path: string, all: SessionRow[]): string[] {
  const byPath = new Map(all.map((s) => [s.path, s]));
  const byId = new Map(all.map((s) => [idKey(s), s]));
  const out: string[] = [];
  let cur = byPath.get(path);
  while (cur) {
    const pk = parentKey(cur);
    const parent = cur.branch_of ? byPath.get(branchSourcePath(cur)!) : pk ? byId.get(pk) : undefined;
    if (!parent || out.includes(parent.path)) break;
    out.push(parent.path);
    cur = parent;
  }
  return out;
}
