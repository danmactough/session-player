use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Claude,
    Codex,
    Pi,
}

impl Source {
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::Claude => "claude",
            Source::Codex => "codex",
            Source::Pi => "pi",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    User,
    Assistant,
    Thinking,
    ToolCall,
    ToolResult,
    /// A slash command or shell command the user ran directly.
    Command,
    /// Harness notices: interruptions, hook output, notifications.
    System,
    Compaction,
    /// Something the harness put in the prompt for the agent to follow: an
    /// instruction file, a skill, an agent definition, MCP server instructions.
    Context,
    /// Earlier conversation handed to the session as context, such as the
    /// ChatGPT chat a Codex task was started from. `title` names it.
    Reference,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub kind: EventKind,
    pub ts: Option<i64>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    /// The tool couldn't do what was asked, or the harness or model failed.
    /// A shell command that ran and exited non-zero is not this; see `failed_exit`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
    /// Non-zero exit status of a shell command that ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    /// `exit_code` signals a failure rather than an answer such as grep's
    /// "no match" (see `parse::benign_exit`). Counted as an error only when
    /// the user turns that on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub failed_exit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sidechain: bool,
    /// The pi entry this event came from, used to place fork markers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// For a `Context` event: what kind of document it is and where it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextSource>,
    /// Identifies the event within an exported bundle ("e<n>"); unchanged by
    /// import, so references to it survive a round trip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
}

impl Event {
    pub fn new(kind: EventKind, ts: Option<i64>, text: impl Into<String>) -> Self {
        Event {
            kind,
            ts,
            text: text.into(),
            tool_name: None,
            call_id: None,
            is_error: false,
            exit_code: None,
            failed_exit: false,
            model: None,
            images: Vec::new(),
            sidechain: false,
            entry: None,
            title: None,
            anchor: None,
            context: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    /// CLAUDE.md, CLAUDE.local.md, AGENTS.md and the like.
    Instructions,
    Skill,
    /// A subagent's definition.
    Agent,
    /// Instructions an MCP server gives the agent.
    Mcp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextSource {
    pub kind: ContextKind,
    /// The file's path, or the skill, agent or server's name.
    pub name: String,
    /// The same document was given earlier in the session.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub repeat: bool,
    /// Whose document it is: "user", "repo", "local" or "managed". Unset for
    /// one that isn't a file, such as an MCP server's instructions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tokens {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
}

/// Everything a format parser extracts from one session file.
#[derive(Debug)]
pub struct Parsed {
    pub source: Source,
    pub native_id: String,
    pub cwd: Option<String>,
    /// Overrides the project derived from `cwd` (Codex chats outside a project).
    pub project: Option<String>,
    pub git_branch: Option<String>,
    /// An explicit title and where it came from ("custom", "ai", "thread", "name", ...).
    pub title: Option<(String, &'static str)>,
    pub models: Vec<String>,
    pub tokens: Tokens,
    pub events: Vec<Event>,
    pub is_subagent: bool,
    pub parent_id: Option<String>,
    pub started_at: Option<i64>,
    /// The agent a session was imported from (Codex can import Claude Code sessions).
    pub imported_from: Option<String>,
    /// Alternate branches in a pi session tree, other than the active one.
    pub branches: Vec<Branch>,
    /// For a pi fork file: the last entry it shares with the session it was
    /// forked from, and how many prompts precede that point.
    pub fork_entry: Option<String>,
    pub fork_turn: Option<i64>,
}

/// A pi branch that is not the active one.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Branch {
    /// Id of the branch's last entry; pass it back to load the branch.
    pub leaf: String,
    pub label: String,
    pub started_at: Option<i64>,
    pub updated_at: Option<i64>,
    /// Prompts made on the branch after it split off.
    pub user_turns: i64,
    /// How many prompts precede the split.
    pub fork_turn: i64,
    /// The last entry shared with the source branch.
    pub fork_entry: Option<String>,
    /// The branch this one split from; None means the active branch.
    pub parent_leaf: Option<String>,
}

impl Parsed {
    pub fn new(source: Source, native_id: String) -> Self {
        Parsed {
            source,
            native_id,
            cwd: None,
            project: None,
            git_branch: None,
            title: None,
            models: Vec::new(),
            tokens: Tokens::default(),
            events: Vec::new(),
            is_subagent: false,
            parent_id: None,
            started_at: None,
            imported_from: None,
            branches: Vec::new(),
            fork_entry: None,
            fork_turn: None,
        }
    }

    pub fn add_model(&mut self, model: &str) {
        if model.is_empty() || model.starts_with('<') {
            return;
        }
        if !self.models.iter().any(|m| m == model) {
            self.models.push(model.to_string());
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionRow {
    pub path: String,
    pub source: String,
    pub native_id: String,
    pub cwd: Option<String>,
    pub project: String,
    pub title: String,
    pub title_source: String,
    pub first_prompt: Option<String>,
    pub started_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub user_turns: i64,
    pub assistant_msgs: i64,
    pub tool_calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub models: Vec<String>,
    pub git_branch: Option<String>,
    pub is_subagent: bool,
    pub parent_id: Option<String>,
    pub size: i64,
    pub imported_from: Option<String>,
    pub branches: Vec<Branch>,
    pub fork_entry: Option<String>,
    pub fork_turn: Option<i64>,
    /// Failed tool calls plus model/API errors.
    pub error_count: i64,
    /// Shell commands that exited with a failing status.
    pub exit_count: i64,
    /// Imported from a bundle into the app's library; there is no agent file to resume or open.
    pub from_bundle: bool,
}
