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
  /** Tool calls that couldn't run, plus harness and model/API errors. */
  error_count: number;
  /** Shell commands that ran and exited with a failing status. */
  exit_count: number;
  /** Imported from a shared bundle: replayable here, but there is no agent file to resume or open. */
  from_bundle: boolean;
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
  | "compaction"
  /** Earlier conversation handed to the session as context; `title` names it. */
  | "reference";

export interface SessionEvent {
  kind: EventKind;
  ts: number | null;
  text: string;
  tool_name?: string;
  call_id?: string;
  is_error?: boolean;
  /** Non-zero exit status of a shell command that ran. */
  exit_code?: number;
  /** The exit status means failure (not grep's "no match"); an error only if the user counts them. */
  failed_exit?: boolean;
  model?: string;
  images?: string[];
  sidechain?: boolean;
  entry?: string;
  title?: string;
  /** Set on events from an imported bundle; stable across export and import. */
  anchor?: string;
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

/** A desktop app that can open a session, and the link that opens it there. */
export interface TerminalApp {
  id: string;
  label: string;
}

/** A session left out of an export, listed so the recipient knows it exists. */
export interface RelatedSession {
  relation: "parent" | "subagent" | "branch";
  id: string;
  title: string;
}

export interface BundleAttachment {
  id: string;
  session: string;
  anchor: string;
  kind: "image" | "file";
  name: string | null;
  mime: string | null;
  bytes: number | null;
  status: "included" | "omitted";
  reason: string | null;
}

export interface ExportPreview {
  token: number;
  summary: {
    bytes: number;
    file_name: string;
    sessions: { id: string; title: string; is_subagent: boolean; prompts: number; events: number; has_details: boolean; annotations: number }[];
    omitted: RelatedSession[];
    attachments: BundleAttachment[];
  };
  subagents: { path: string; title: string; included: boolean }[];
}

export type OnConflict = "replace" | "keep";

export interface ImportReport {
  imported: string[];
  unchanged: string[];
  kept: string[];
  conflicts: { id: string; existing_title: string; incoming_title: string }[];
  /** Sessions with no conversation, which were not imported. */
  empty: string[];
  /** Details and annotations you changed since an earlier import, kept rather than replaced. */
  kept_notes: number;
}

export interface LibraryError {
  path: string;
  error: string;
}

export type Outcome = "unknown" | "succeeded" | "partial" | "failed";

/** What a person has written about a session; the transcript's own details stay on SessionRow. */
export interface SessionMeta {
  key: string;
  title: string | null;
  goal: string | null;
  context: string | null;
  outcome: Outcome;
  outcome_note: string | null;
  tags: string[];
  links: { url: string; label: string | null }[];
  updated_at: number;
  /** Fields ("title", "goal", "context", "outcome", "tags") holding a model's suggestion no one has edited or accepted. */
  generated: string[];
  generated_by: { model: string; at: number; revision: string } | null;
}

export type AnnotationKind =
  | "tool_error"
  | "human_intervention"
  | "agent_stuck"
  | "human_frustration"
  | "wrong_assumption"
  | "wasted_work"
  | "environment"
  | "missing_context"
  | "automatable"
  | "key_decision"
  | "other";

export interface EventRef {
  anchor: string;
  kind: EventKind;
  ts: number | null;
  /** The pi entry the event came from; pi events are found by entry. */
  entry?: string;
  /** Whether the event is in the transcript as loaded. */
  found?: boolean;
  /** The event is on another branch of the pi session. */
  elsewhere?: boolean;
}

export type Author = { type: "person"; name: string | null } | { type: "model"; model: string };

export interface Annotation {
  /** Empty until saved. */
  id: string;
  key: string;
  kind: AnnotationKind;
  note: string;
  /** None means the annotation is about the whole session. */
  anchors: EventRef[];
  author: Author;
  created_at: number;
  updated_at: number;
  /** A model's annotation someone accepted or edited; regenerating keeps it. */
  accepted?: boolean;
  rationale?: string;
  confidence?: string;
  /** The transcript revision a model's annotation was made from. */
  revision?: string;
}

export interface NoteSummary {
  key: string;
  title: string | null;
  outcome: Outcome;
  tags: string[];
  kinds: AnnotationKind[];
}

export interface OpenTarget {
  id: string;
  label: string;
  url: string;
}
