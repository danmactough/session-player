export type Source = "claude" | "codex" | "pi";

export interface SessionRow {
  path: string;
  source: Source;
  native_id: string;
  cwd: string | null;
  project: string;
  title: string;
  title_source: string;
  first_prompt: string | null;
  started_at: number | null;
  updated_at: number | null;
  user_turns: number;
  assistant_msgs: number;
  tool_calls: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  models: string[];
  git_branch: string | null;
  is_subagent: boolean;
  parent_id: string | null;
  size: number;
  imported_from: string | null;
  branches: Branch[];
  /** pi forks and branches: the last entry shared with the source, and how many prompts precede it. */
  fork_entry: string | null;
  fork_turn: number | null;
  /** Set on rows the UI makes for pi branches: the real session file. */
  branch_of?: string;
  branch_leaf?: string;
  /** The branch this branch split from; null means the session's active branch. */
  branch_parent?: string | null;
}

/** A pi branch other than the active one. */
export interface Branch {
  leaf: string;
  label: string;
  started_at: number | null;
  updated_at: number | null;
  user_turns: number;
  fork_turn: number;
  fork_entry: string | null;
  parent_leaf: string | null;
}

export type EventKind =
  | "user"
  | "assistant"
  | "thinking"
  | "tool_call"
  | "tool_result"
  | "command"
  | "system"
  | "compaction";

export interface SessionEvent {
  kind: EventKind;
  ts: number | null;
  text: string;
  tool_name?: string;
  call_id?: string;
  is_error?: boolean;
  model?: string;
  images?: string[];
  sidechain?: boolean;
  entry?: string;
}

export interface RootInfo {
  path: string;
  exists: boolean;
  sessions: number;
}

export interface SearchHit {
  path: string;
  snippet: string;
  rank: number;
}

export interface IndexStats {
  scanned: number;
  updated: number;
  removed: number;
  failed: number;
  elapsed_ms: number;
}

export type SortKey = "recent" | "oldest" | "longest" | "tokens";
