use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
}

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub kind: EventKind,
    pub ts: Option<i64>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub sidechain: bool,
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
            model: None,
            images: Vec::new(),
            sidechain: false,
        }
    }
}

#[derive(Debug, Default, Clone, Serialize)]
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
    pub git_branch: Option<String>,
    /// An explicit title and where it came from ("custom", "ai", "thread", "name", ...).
    pub title: Option<(String, &'static str)>,
    pub models: Vec<String>,
    pub tokens: Tokens,
    pub events: Vec<Event>,
    pub is_subagent: bool,
    pub parent_id: Option<String>,
    pub started_at: Option<i64>,
}

impl Parsed {
    pub fn new(source: Source, native_id: String) -> Self {
        Parsed {
            source,
            native_id,
            cwd: None,
            git_branch: None,
            title: None,
            models: Vec::new(),
            tokens: Tokens::default(),
            events: Vec::new(),
            is_subagent: false,
            parent_id: None,
            started_at: None,
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
}
