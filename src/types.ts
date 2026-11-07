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
  | "context"
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
  /** For a "context" event: what the harness put in the prompt, and where it came from. */
  context?: {
    kind: "instructions" | "skill" | "agent" | "mcp";
    /** The file's path, or the skill, agent or server's name. */
    name: string;
    /** The same document was given earlier in the session. */
    repeat?: boolean;
    /** Whose document it is: "user", "repo", "local" or "managed". */
    scope?: string;
  };
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
    sessions: {
      id: string;
      title: string;
      is_subagent: boolean;
      prompts: number;
      events: number;
      has_details: boolean;
      annotations: number;
      /** Instruction files, skills, agent definitions and MCP instructions in the transcript, each once. */
      context: NonNullable<SessionEvent["context"]>[];
    }[];
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

/** The analysis of a session: how it turned out and the parts of the repository it worked on. */
export interface SessionMeta {
  key: string;
  outcome: Outcome;
  /** Why it succeeded or didn't. */
  outcome_note: string | null;
  /** Repository paths or packages worked on, such as "packages/api". */
  components: string[];
  updated_at: number;
  /** Fields ("outcome", "components") still holding what a model generated. */
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
  | "ignored_instruction"
  | "other";

export interface EventRef {
  anchor: string;
  kind: EventKind;
  ts: number | null;
  /** The pi entry the event came from; pi events are found by entry. */
  entry?: string;
  /** For a context document: its name, which tells apart documents sent together. */
  name?: string;
  /** Whether the event is in the transcript as loaded. */
  found?: boolean;
  /** The event is on another branch of the pi session. */
  elsewhere?: boolean;
}

export type Author = { type: "person"; name: string | null } | { type: "model"; model: string };

export interface Annotation {
  /** Empty until saved. */
  id: string;
  /** Missing until saved. Each change saves a new version; the backend numbers them. */
  version?: number;
  key: string;
  kind: AnnotationKind;
  note: string;
  /** The events it is about; at least one. */
  anchors: EventRef[];
  author: Author;
  created_at: number;
  updated_at: number;
  /** A model's annotation someone edited; regenerating keeps it. */
  edited?: boolean;
  rationale?: string;
  confidence?: string;
  /** The transcript revision a model's annotation was made from. */
  revision?: string;
}

export interface NoteSummary {
  key: string;
  outcome: Outcome;
  components: string[];
  kinds: AnnotationKind[];
  /** How many of the session's annotations have each kind. */
  kind_counts: Partial<Record<AnnotationKind, number>>;
  person_annotations: number;
  model_annotations: number;
  /** The model's annotations by confidence: "low", "medium", "high". */
  confidence: Record<string, number>;
  /** Analyze has run on the session. */
  analyzed: boolean;
}

export interface OpenTarget {
  id: string;
  label: string;
  url: string;
}

/** What a report's takeaway or action rests on: an annotation as the report saw it, or a session's outcome. */
export type Evidence = { type: "annotation"; id: string; version: number; key: string } | { type: "session"; key: string };

export type ActionCategory = "codebase" | "skill" | "instructions" | "environment" | "automation";

export interface ReportSession {
  key: string;
  title: string;
  outcome: Outcome;
  outcome_note: string | null;
  components: string[];
  /** The annotations the report was given, as (id, version). */
  annotations: { id: string; version: number }[];
}

/** A saved report on a set of sessions; it never changes after it's written. */
export interface Report {
  id: string;
  created_at: number;
  title: string;
  model: string;
  prompt_version: number;
  /** The user's own focus text, when the report wasn't written with the built-in one. */
  focus?: string;
  sessions: ReportSession[];
  stats: {
    sessions: number;
    outcomes: Partial<Record<Outcome, number>>;
    kinds: Partial<Record<AnnotationKind, number>>;
    annotations: number;
  };
  summary: string;
  takeaways: { text: string; evidence: Evidence[] }[];
  actions: { title: string; category: ActionCategory; priority: "high" | "medium" | "low"; change: string; evidence: Evidence[] }[];
  /** Takeaways and actions dropped because they cited nothing the report was given. */
  rejected: number;
}

/** An annotation as a report was given it, and its version now (null once deleted). */
export interface CitedAnnotation {
  annotation: Annotation;
  current_version: number | null;
}

export interface ReportSummary {
  id: string;
  created_at: number;
  title: string;
  model: string;
  sessions: number;
  actions: number;
}
