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
