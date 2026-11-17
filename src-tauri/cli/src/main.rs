//! `sp`: Session Player from the command line. It reads and writes the same
//! database as the app, so analysis and reports made here show up there.

mod markdown;

use std::collections::{HashMap, HashSet};
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anyhow::{Context, bail};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone};
use clap::{Args, Parser, Subcommand, ValueEnum};
use rusqlite::Connection;
use session_player_core::model::{Event, SessionRow, Source};
use session_player_core::{annotations, bundle, generate, index, parse, reports, runlog, sync};

/// The app's identifier, which names its data folder.
const APP_ID: &str = "com.danmactough.sessionplayer";

/// How many sessions are analyzed at the same time.
const ANALYZE_AT_ONCE: usize = 3;

#[derive(Parser)]
#[command(name = "sp", version, about = "Session Player from the command line")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List a project's sessions, here or on another machine
    Sessions {
        /// Another machine, as you would name it to ssh: list the sessions it has for this repository
        host: Option<String>,
        #[command(flatten)]
        scope: Scope,
        /// What `sp sessions <host>` passes to the other machine: the repository's name, in place of a folder
        #[arg(long, hide = true, conflicts_with = "project")]
        repo: Option<String>,
    },
    /// Have Claude fill in the outcome, components and annotations of a session
    ///
    /// With no SESSION and no --since, analyzes the project's most recently
    /// active session. With --since, analyzes every session in range that
    /// hasn't been analyzed.
    Analyze {
        /// A session id (or the start of one) or the path of a session file
        session: Option<String>,
        #[command(flatten)]
        scope: Scope,
        #[command(flatten)]
        prompt: Prompt,
        /// With --since: analyze sessions again that already have an analysis
        #[arg(long)]
        force: bool,
    },
    /// Write a report on a project's sessions and print it as Markdown
    Report {
        #[command(flatten)]
        scope: Scope,
        #[command(flatten)]
        prompt: Prompt,
        /// The model that writes the report
        #[arg(long, default_value = reports::DEFAULT_MODEL)]
        model: String,
        /// Report only on sessions already analyzed; don't analyze the others first
        #[arg(long)]
        no_analyze: bool,
        /// Write the Markdown to this file instead of standard output
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// List saved reports, newest first
    Reports,
    /// Print a saved report as Markdown
    Show {
        /// A report id (or the start of one). Defaults to the project's newest report.
        report: Option<String>,
        #[command(flatten)]
        project: ProjectArg,
    },
    /// Start a Claude Code session that works through a report's recommended actions
    ///
    /// Claude reads the report, proposes a concrete change for each open
    /// action, asks which to make, makes those, and records what you chose.
    Doctor {
        /// A report id (or the start of one). Defaults to the project's newest report.
        report: Option<String>,
        #[command(flatten)]
        project: ProjectArg,
        /// Print the prompt instead of starting Claude
        #[arg(long)]
        print: bool,
    },
    /// Record what you did about one of a report's recommended actions
    Action {
        /// A report id (or the start of one)
        report: String,
        /// The action's number in the report, starting at 1
        number: usize,
        #[arg(long, value_enum)]
        status: Option<Status>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Copy a project's sessions, analysis and reports from another machine over SSH
    ///
    /// The other machine needs sp too: build it there and run `sp install`.
    /// Run this inside a checkout of the same repository. Sessions land in
    /// the agents' own folders, so they can be resumed here.
    Pull {
        /// The other machine, as you would name it to ssh
        host: String,
        #[command(flatten)]
        scope: Scope,
        /// Show what would be copied, where it would be written and what would be replaced, without writing anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Read new and changed session files into the database, as the app does when it opens
    ///
    /// Every other command does this first, so it is only needed to prepare a
    /// machine ahead of time, such as one that others pull from, or to rebuild
    /// the index with --full.
    Index {
        /// Read every session file again, not only the new and changed ones
        #[arg(long)]
        full: bool,
    },
    /// Copy this program to ~/.local/bin/sp, where `sp pull` on another machine looks for it
    Install,
    /// What `sp pull` runs on the other machine: writes the repository's sessions to standard output
    #[command(hide = true)]
    ExportProject {
        /// The repository's name, as `sp pull` worked it out
        #[arg(long)]
        repo: String,
        /// Only sessions started at or after this many milliseconds since 1970
        #[arg(long)]
        since: Option<i64>,
    },
    /// Stores what `export-project` wrote, read from standard input
    #[command(hide = true)]
    ImportProject {
        #[command(flatten)]
        project: ProjectArg,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Args)]
struct ProjectArg {
    /// The project's folder. Defaults to the repository the current folder is in.
    #[arg(long)]
    project: Option<PathBuf>,
}

#[derive(Args)]
struct Scope {
    #[command(flatten)]
    project: ProjectArg,
    /// Only sessions started at or after this: a date (2026-11-01), a date
    /// and time (2026-11-01T09:30), or an age (7d, 12h, 2w)
    #[arg(long)]
    since: Option<String>,
}

#[derive(Args)]
struct Prompt {
    /// What to focus on, in place of the prompt saved in the app or the built-in one
    #[arg(long, conflicts_with = "prompt_file")]
    prompt: Option<String>,
    /// A file holding that text; `-` reads standard input
    #[arg(long)]
    prompt_file: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
enum Status {
    Done,
    InProgress,
    Declined,
    Considering,
}

struct Paths {
    data: PathBuf,
    db: PathBuf,
    library: PathBuf,
    log: PathBuf,
}

/// The folders the app uses (Tauri's `app_data_dir` and `app_log_dir`), so
/// both work on one database. `SESSION_PLAYER_DATA_DIR` points both elsewhere.
fn paths() -> anyhow::Result<Paths> {
    let (data, logs) = match std::env::var_os("SESSION_PLAYER_DATA_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => (PathBuf::from(&dir), PathBuf::from(&dir).join("logs")),
        None => {
            let data = dirs::data_dir().context("Couldn't find the folder for application data")?.join(APP_ID);
            let logs = if cfg!(target_os = "macos") {
                dirs::home_dir().context("Couldn't find the home folder")?.join("Library/Logs").join(APP_ID)
            } else {
                dirs::data_local_dir().context("Couldn't find the folder for application data")?.join(APP_ID).join("logs")
            };
            (data, logs)
        }
    };
    std::fs::create_dir_all(&data).with_context(|| format!("Couldn't create {}", data.display()))?;
    Ok(Paths { db: data.join("index.sqlite"), library: data.join("library"), log: logs.join("analyze.log"), data })
}

/// Registers the agents' session folders. They are added once, the first time
/// the database is used, so that a folder removed in the app's settings stays
/// removed. If sp first ran before any agent had, nothing was added then; with
/// no folders registered at all, the ones that exist now are.
fn register_session_folders(conn: &Connection) -> anyhow::Result<()> {
    index::seed_roots(conn)?;
    if index::roots(conn)?.is_empty() {
        for root in index::default_roots() {
            index::add_root(conn, &root.to_string_lossy())?;
        }
    }
    Ok(())
}

/// Opens the database and indexes sessions written since the last run.
fn open_indexed(paths: &Paths) -> anyhow::Result<Connection> {
    let mut conn = index::open(&paths.db)?;
    register_session_folders(&conn)?;
    let show = std::io::stderr().is_terminal();
    index::run(&mut conn, false, Some(&paths.library), &move |done, total| {
        // A first run reads every session; later runs only what changed.
        if show && total >= 200 && (done % 50 == 0 || done == total) {
            eprint!("\rIndexing sessions {done}/{total}{}", if done == total { "\n" } else { "" });
        }
    })?;
    Ok(conn)
}

/// The project a folder belongs to, named as the index names it.
fn project_of(arg: &ProjectArg) -> anyhow::Result<String> {
    let dir = match &arg.project {
        Some(p) => std::fs::canonicalize(p).with_context(|| format!("No such folder: {}", p.display()))?,
        None => std::env::current_dir()?,
    };
    Ok(index::project_of(Some(&dir.to_string_lossy())))
}

/// When `--since` starts, in milliseconds.
fn parse_since(text: &str, now: DateTime<Local>) -> anyhow::Result<i64> {
    let text = text.trim();
    let local = |t: NaiveDateTime| {
        Local.from_local_datetime(&t).earliest().map(|t| t.timestamp_millis()).with_context(|| format!("{text} isn't a time in this time zone"))
    };
    if let Some(unit) = text.chars().last().filter(|c| "hdw".contains(*c))
        && let Ok(n) = text[..text.len() - 1].parse::<i64>()
    {
        let hours = match unit {
            'h' => 1,
            'd' => 24,
            _ => 24 * 7,
        };
        return Ok((now - chrono::Duration::hours(n * hours)).timestamp_millis());
    }
    if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return local(date.and_hms_opt(0, 0, 0).expect("midnight"));
    }
    if let Ok(t) = DateTime::parse_from_rfc3339(text) {
        return Ok(t.timestamp_millis());
    }
    for format in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(t) = NaiveDateTime::parse_from_str(text, format) {
            return local(t);
        }
    }
    bail!("Couldn't read --since {text}. Use a date (2026-11-01), a date and time (2026-11-01T09:30) or an age (7d, 12h, 2w).")
}

/// A project's sessions, most recently active first, optionally only those
/// started at or after `since`. Subagents are left out: their parent's
/// transcript is the one a person reads and analyzes.
fn in_scope<'a>(rows: &'a [SessionRow], project: &str, since: Option<i64>) -> Vec<&'a SessionRow> {
    rows.iter()
        .filter(|r| !r.is_subagent && r.project == project)
        .filter(|r| since.is_none_or(|s| r.started_at.or(r.updated_at).is_some_and(|t| t >= s)))
        .collect()
}

fn scoped<'a>(rows: &'a [SessionRow], scope: &Scope) -> anyhow::Result<(String, Vec<&'a SessionRow>)> {
    let project = project_of(&scope.project)?;
    let since = scope.since.as_deref().map(|s| parse_since(s, Local::now())).transpose()?;
    let found = in_scope(rows, &project, since);
    if found.is_empty() {
        let range = scope.since.as_deref().map(|s| format!(" since {s}")).unwrap_or_default();
        bail!("No sessions found for {project}{range}.");
    }
    Ok((project, found))
}

/// The session `arg` names: a file's path, or a session id or its start.
fn find_session<'a>(rows: &'a [SessionRow], arg: &str) -> anyhow::Result<&'a SessionRow> {
    if let Ok(path) = std::fs::canonicalize(arg) {
        let path = path.to_string_lossy();
        return rows.iter().find(|r| r.path == path).with_context(|| format!("{path} isn't an indexed session. Is it under one of the session folders?"));
    }
    let matches: Vec<&SessionRow> = rows.iter().filter(|r| r.native_id.starts_with(arg)).collect();
    // A pi session's branches, and an imported copy, share an id; the first is the most recently active.
    let ids: HashSet<&str> = matches.iter().map(|r| r.native_id.as_str()).collect();
    match ids.len() {
        0 => bail!("No session has an id starting with {arg}."),
        1 => Ok(matches[0]),
        n => bail!("{n} sessions have an id starting with {arg}. Give more of the id."),
    }
}

/// The transcript as the app shows it: a library bundle or a session file.
fn session_events(path: &str) -> anyhow::Result<Vec<Event>> {
    let p = Path::new(path);
    if bundle::is_bundle_path(p) {
        return bundle::load_events(p);
    }
    let source: Source = parse::detect(p).context("Unrecognized session format")?;
    Ok(parse::parse_file(p, source)?.events)
}

/// The `claude` command with this shell's environment and the Claude settings.
fn claude_command(paths: &Paths) -> anyhow::Result<generate::Claude> {
    let bin = find_claude().context("Couldn't find the claude command. Install Claude Code and sign in, then try again.")?;
    let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude"));
    // `Claude` decides how to authenticate from the variables it is given, so it gets this process's.
    let mut claude = generate::Claude::new(bin, std::env::vars().collect(), &config_dir)?;
    claude.log = Some(paths.log.clone());
    Ok(claude)
}

fn find_claude() -> Option<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let on_path: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).map(|d| d.join("claude")).collect()).unwrap_or_default();
    on_path.into_iter().chain([home.join(".local/bin/claude"), home.join(".claude/local/claude")]).find(|p| p.is_file())
}

/// The focus text for a run: what the flags give, else the prompt saved in
/// the app's settings under `field`. `None` uses the built-in text.
fn focus(conn: &Connection, prompt: &Prompt, field: &str) -> anyhow::Result<Option<String>> {
    let given = match (&prompt.prompt, &prompt.prompt_file) {
        (Some(text), _) => Some(text.clone()),
        (None, Some(file)) if file == Path::new("-") => {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text)?;
            Some(text)
        }
        (None, Some(file)) => Some(std::fs::read_to_string(file).with_context(|| format!("Couldn't read {}", file.display()))?),
        (None, None) => None,
    };
    if let Some(text) = given {
        if text.trim().is_empty() {
            bail!("The prompt is empty.");
        }
        return Ok(Some(text));
    }
    let saved: Option<String> = conn.query_row("SELECT value FROM settings WHERE key='ui.prefs'", [], |r| r.get(0)).ok();
    Ok(saved
        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
        .and_then(|prefs| prefs.get(field).and_then(|v| v.as_str()).map(str::to_string))
        .filter(|text| !text.trim().is_empty()))
}

fn first_line(e: &anyhow::Error) -> String {
    format!("{e:#}").lines().next().unwrap_or_default().to_string()
}

/// A flag that Ctrl-C sets, which stops the `claude` calls in progress. They
/// run in process groups of their own, so the terminal's interrupt doesn't
/// reach them. A second Ctrl-C quits at once.
fn cancel_on_interrupt() -> anyhow::Result<Arc<AtomicBool>> {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    ctrlc::set_handler(move || {
        if flag.swap(true, Ordering::SeqCst) {
            std::process::exit(130);
        }
    })?;
    Ok(cancel)
}

/// Analyzes one session and saves the result. Returns a one-line summary.
fn analyze_one(paths: &Paths, claude: &generate::Claude, row: &SessionRow, focus: Option<&str>, cancel: &Arc<AtomicBool>) -> anyhow::Result<String> {
    let key = annotations::session_key(row);
    runlog::append(&paths.log, &format!("Analyze {key} started from the command line\n{}", row.path));
    let result = (|| {
        let events = session_events(&row.path)?;
        let mut ask = |system: &str, input: &str, schema: &serde_json::Value| generate::run_claude(claude, system, input, schema, cancel);
        let generated = generate::generate(&events, focus, &mut ask, &mut |_, _| {})?;
        let conn = index::open(&paths.db)?;
        generate::apply(&conn, &key, &generated, &events)?;
        Ok(format!(
            "{}, {} components, {} annotations",
            markdown::outcome_label(generated.outcome).to_lowercase(),
            generated.components.len(),
            generated.annotations.len()
        ))
    })();
    match &result {
        Ok(summary) => runlog::append(&paths.log, &format!("Analyze {key} done: {summary}")),
        Err(e) if generate::is_cancelled(e) => runlog::append(&paths.log, &format!("Analyze {key} cancelled")),
        Err(e) => runlog::append(&paths.log, &format!("Analyze {key} FAILED: {e:#}")),
    }
    result
}

/// Analyzes `rows`, a few at a time, reporting each on standard error.
/// Returns how many failed. Stops early once `cancel` is set.
fn analyze_many(paths: &Paths, claude: &generate::Claude, rows: &[&SessionRow], focus: Option<&str>, cancel: &Arc<AtomicBool>) -> usize {
    let next = AtomicUsize::new(0);
    let finished = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..ANALYZE_AT_ONCE.min(rows.len()) {
            scope.spawn(|| {
                while !cancel.load(Ordering::SeqCst)
                    && let Some(row) = rows.get(next.fetch_add(1, Ordering::SeqCst))
                {
                    let result = analyze_one(paths, claude, row, focus, cancel);
                    let n = finished.fetch_add(1, Ordering::SeqCst) + 1;
                    let title = markdown::one_line(&row.title);
                    match result {
                        Ok(summary) => eprintln!("[{n}/{}] {title}: {summary}", rows.len()),
                        Err(e) if generate::is_cancelled(&e) => {}
                        Err(e) => {
                            failed.fetch_add(1, Ordering::SeqCst);
                            eprintln!("[{n}/{}] {title}: failed: {}", rows.len(), first_line(&e));
                        }
                    }
                }
            });
        }
    });
    failed.into_inner()
}

fn analyzed_keys(conn: &Connection) -> anyhow::Result<HashSet<String>> {
    Ok(annotations::summaries(conn)?.into_iter().filter(|s| s.analyzed).map(|s| s.key).collect())
}

fn print_analysis(conn: &Connection, row: &SessionRow) -> anyhow::Result<()> {
    let key = annotations::session_key(row);
    let meta = annotations::meta(conn, &key)?.unwrap_or_default();
    println!("{}", markdown::one_line(&row.title));
    let note = meta.outcome_note.as_deref().map(|n| format!(": {}", markdown::one_line(n))).unwrap_or_default();
    println!("Outcome: {}{note}", markdown::outcome_label(meta.outcome));
    if !meta.components.is_empty() {
        println!("Components: {}", meta.components.join(", "));
    }
    let list = annotations::annotations(conn, &key)?;
    if !list.is_empty() {
        println!("Annotations:");
        for a in list {
            println!("- {}: {}", markdown::kind_label(a.kind), markdown::one_line(&a.note));
        }
    }
    Ok(())
}

/// Ends the command after Ctrl-C stopped a batch partway.
fn stop_if_cancelled(cancel: &AtomicBool) -> anyhow::Result<()> {
    if cancel.load(Ordering::SeqCst) {
        return Err(Interrupted.into());
    }
    Ok(())
}

#[derive(Debug)]
struct Interrupted;

impl std::fmt::Display for Interrupted {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("Cancelled")
    }
}

impl std::error::Error for Interrupted {}

fn log_file_hint(paths: &Paths) -> String {
    format!("Details are in {}.", paths.log.display())
}

/// Sessions of the repository named `repo`, whichever folder its checkout is in
/// here, most recently active first. This is what another machine asks for: it
/// knows the repository's name but not where this machine keeps it.
fn in_repo<'a>(rows: &'a [SessionRow], repo: &str, since: Option<i64>) -> Vec<&'a SessionRow> {
    let mut names: HashMap<&str, Option<sync::RepoName>> = HashMap::new();
    rows.iter()
        .filter(|r| !r.is_subagent && !r.from_bundle)
        .filter(|r| names.entry(r.project.as_str()).or_insert_with(|| sync::repo_name(Path::new(&r.project))).as_ref().is_some_and(|name| name.0 == repo))
        .filter(|r| since.is_none_or(|s| r.started_at.or(r.updated_at).is_some_and(|t| t >= s)))
        .collect()
}

fn cmd_sessions(paths: &Paths, host: Option<&str>, scope: &Scope, repo: Option<&str>) -> anyhow::Result<()> {
    if let Some(host) = host {
        return remote_sessions(host, scope);
    }
    let conn = open_indexed(paths)?;
    let rows = index::list(&conn)?;
    let found = match repo {
        Some(repo) => in_repo(&rows, repo, scope.since.as_deref().map(|s| parse_since(s, Local::now())).transpose()?),
        None => scoped(&rows, scope)?.1,
    };
    let analyzed = analyzed_keys(&conn)?;
    for r in found {
        let when = r.started_at.or(r.updated_at).and_then(|t| Local.timestamp_millis_opt(t).single());
        println!(
            "{:<8}  {}  {:<6}  {}  {}",
            r.native_id.chars().take(8).collect::<String>(),
            when.map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_else(|| " ".repeat(16)),
            r.source,
            if analyzed.contains(&annotations::session_key(r)) { "analyzed" } else { "        " },
            markdown::one_line(&r.title)
        );
    }
    Ok(())
}

fn cmd_analyze(paths: &Paths, session: Option<&str>, scope: &Scope, prompt: &Prompt, force: bool) -> anyhow::Result<()> {
    let conn = open_indexed(paths)?;
    let rows = index::list(&conn)?;
    let focus = focus(&conn, prompt, "analyzePrompt")?;
    let claude = claude_command(paths)?;
    let cancel = cancel_on_interrupt()?;
    if session.is_none() && scope.since.is_some() {
        let (_, found) = scoped(&rows, scope)?;
        let analyzed = analyzed_keys(&conn)?;
        // A pi session's branches share one analysis.
        let mut seen = HashSet::new();
        let todo: Vec<&SessionRow> = found
            .into_iter()
            .filter(|r| force || !analyzed.contains(&annotations::session_key(r)))
            .filter(|r| seen.insert(annotations::session_key(r)))
            .collect();
        if todo.is_empty() {
            eprintln!("Every session in range is already analyzed. Use --force to analyze them again.");
            return Ok(());
        }
        let failed = analyze_many(paths, &claude, &todo, focus.as_deref(), &cancel);
        stop_if_cancelled(&cancel)?;
        if failed > 0 {
            bail!("{failed} of {} sessions couldn't be analyzed. {}", todo.len(), log_file_hint(paths));
        }
        return Ok(());
    }
    let row = match session {
        Some(arg) => find_session(&rows, arg)?,
        None => {
            let project = project_of(&scope.project)?;
            *in_scope(&rows, &project, None).first().with_context(|| format!("No sessions found for {project}."))?
        }
    };
    eprintln!("Analyzing {}…", markdown::one_line(&row.title));
    analyze_one(paths, &claude, row, focus.as_deref(), &cancel).map_err(|e| if generate::is_cancelled(&e) { e } else { e.context(log_file_hint(paths)) })?;
    print_analysis(&conn, row)
}

fn render(conn: &Connection, report: &reports::Report) -> anyhow::Result<String> {
    let cited = reports::cited_annotations(conn, report)?;
    let responses = markdown::Responses { responses: reports::action_responses(conn, &report.id)?, verdicts: reports::action_verdicts(conn, &report.id)? };
    Ok(markdown::report_markdown(report, &cited, &responses))
}

fn cmd_report(paths: &Paths, scope: &Scope, prompt: &Prompt, model: &str, no_analyze: bool, out: Option<&Path>) -> anyhow::Result<()> {
    let conn = open_indexed(paths)?;
    let rows = index::list(&conn)?;
    let (project, found) = scoped(&rows, scope)?;
    let focus = focus(&conn, prompt, "reportPrompt")?;
    let claude = claude_command(paths)?;
    let cancel = cancel_on_interrupt()?;

    let mut seen = HashSet::new();
    let found: Vec<&SessionRow> = found.into_iter().filter(|r| seen.insert(annotations::session_key(r))).collect();
    let analyzed = analyzed_keys(&conn)?;
    let unanalyzed: Vec<&SessionRow> = found.iter().copied().filter(|r| !analyzed.contains(&annotations::session_key(r))).collect();
    if !no_analyze && !unanalyzed.is_empty() {
        eprintln!("Analyzing {} of {} sessions first…", unanalyzed.len(), found.len());
        // The saved Analyze prompt applies here; --prompt is for the report.
        let analyze_focus = self::focus(&conn, &Prompt { prompt: None, prompt_file: None }, "analyzePrompt")?;
        let failed = analyze_many(paths, &claude, &unanalyzed, analyze_focus.as_deref(), &cancel);
        stop_if_cancelled(&cancel)?;
        if failed > 0 {
            eprintln!("{failed} sessions couldn't be analyzed and are left out of the report. {}", log_file_hint(paths));
        }
    }
    let analyzed = analyzed_keys(&conn)?;
    let picks: Vec<reports::SessionPick> = found
        .iter()
        .filter(|r| analyzed.contains(&annotations::session_key(r)))
        .map(|r| reports::SessionPick { key: annotations::session_key(r), title: r.title.clone() })
        .collect();
    if picks.is_empty() {
        bail!("None of the {} sessions for {project} has an analysis to report on.", found.len());
    }

    eprintln!("Writing a report on {} sessions with {model}…", picks.len());
    runlog::append(&paths.log, &format!("Report started from the command line on {} sessions with {model}", picks.len()));
    let mut ask = |system: &str, input: &str, schema: &serde_json::Value| generate::run_claude_model(&claude, model, system, input, schema, &cancel);
    let report = reports::generate(&conn, &picks, focus.as_deref(), &mut ask, &mut |done, total| {
        if total > 1 {
            eprintln!("Part {done} of {total} done");
        }
    })
    .map_err(|e| {
        if generate::is_cancelled(&e) {
            runlog::append(&paths.log, "Report cancelled");
            return e;
        }
        runlog::append(&paths.log, &format!("Report FAILED: {e:#}"));
        e.context(log_file_hint(paths))
    })?;
    reports::save_report(&conn, &report)?;
    runlog::append(&paths.log, &format!("Report done, saved as {}", report.id));

    let text = render(&conn, &report)?;
    match out {
        Some(file) => std::fs::write(file, text).with_context(|| format!("Couldn't write {}", file.display()))?,
        None => std::io::stdout().write_all(text.as_bytes())?,
    }
    eprintln!("Saved as report {}. Work through it with: sp doctor {}", report.id, report.id);
    Ok(())
}

fn cmd_reports(paths: &Paths) -> anyhow::Result<()> {
    let conn = index::open(&paths.db)?;
    for r in reports::reports(&conn)? {
        let when = Local.timestamp_millis_opt(r.created_at).single().map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
        println!("{}  {when}  {} sessions, {} actions  {}", r.id, r.sessions, r.actions, markdown::one_line(&r.title));
    }
    Ok(())
}

/// The report `arg` names by id or its start. With no `arg`, the newest
/// report that covers a session of `project`.
fn find_report(conn: &Connection, arg: Option<&str>, project: &dyn Fn() -> anyhow::Result<String>) -> anyhow::Result<reports::Report> {
    let all = reports::reports(conn)?;
    let load = |id: &str| reports::report(conn, id)?.context("That report no longer exists.");
    if let Some(arg) = arg {
        let matches: Vec<&reports::ReportSummary> = all.iter().filter(|r| r.id.starts_with(arg)).collect();
        return match matches.len() {
            0 => bail!("No report has an id starting with {arg}. List them with: sp reports"),
            1 => load(&matches[0].id),
            n => bail!("{n} reports have an id starting with {arg}. Give more of the id."),
        };
    }
    let project = project()?;
    let projects: HashMap<String, String> = index::list(conn)?.into_iter().map(|r| (annotations::session_key(&r), r.project)).collect();
    for summary in &all {
        let report = load(&summary.id)?;
        if report.sessions.iter().any(|s| projects.get(&s.key) == Some(&project)) {
            return Ok(report);
        }
    }
    bail!("No report covers {project} yet. Write one with: sp report --since 7d")
}

fn cmd_show(paths: &Paths, report: Option<&str>, project: &ProjectArg) -> anyhow::Result<()> {
    let conn = index::open(&paths.db)?;
    let report = find_report(&conn, report, &|| project_of(project))?;
    std::io::stdout().write_all(render(&conn, &report)?.as_bytes())?;
    Ok(())
}

fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// What the doctor session is told. The report is in a file, not in the
/// prompt, because a long report doesn't fit in a command-line argument.
fn doctor_prompt(report: &reports::Report, report_file: &Path, sp: &str) -> String {
    format!(
        "Session Player wrote a report on recent coding-agent sessions in this project. It is in {file}. Read it.

Work through its \"Recommended actions\":

1. Skip an action whose Status line says Done or Declined. Take every other action, including ones marked In progress or Thinking about it.
2. For each, look at the files it concerns (this repository, and agent instructions, skills, hooks or settings in the home folder when the action is about those) and work out the concrete change: which files, and what changes in each. If the action no longer applies because the change is already in place, say so instead.
3. Show me the list of proposed changes, numbered as in the report, each with the files it touches and a one-or-two-sentence description. Then ask me which ones to make, as one multiple-choice question that lets me pick several. Change nothing before I answer.
4. Make the changes I picked, and only those. Don't commit.
5. Record what happened to each action you proposed, using the action's number from the report:
   - made: `{sp} action {id} <number> --status done`
   - I didn't pick it: `{sp} action {id} <number> --status declined`
   - already in place or can't be done: `{sp} action {id} <number> --status declined --note \"<why>\"`
   Add `--note \"<text>\"` whenever I gave a reason or you changed something different from what the report asked for.
6. Finish with the list of what you changed and what you recorded.",
        file = report_file.display(),
        id = report.id,
    )
}

fn cmd_doctor(paths: &Paths, report: Option<&str>, project: &ProjectArg, print: bool) -> anyhow::Result<()> {
    let conn = open_indexed(paths)?;
    let report = find_report(&conn, report, &|| project_of(project))?;
    if report.actions.is_empty() {
        bail!("Report {} recommends no actions.", report.id);
    }
    let dir = paths.data.join("doctor");
    std::fs::create_dir_all(&dir)?;
    let report_file = dir.join(format!("{}.md", report.id));
    std::fs::write(&report_file, render(&conn, &report)?)?;
    drop(conn);

    // The full path, so the session can run it even when `sp` isn't on its PATH.
    let sp = std::env::current_exe().map(|p| shell_quote(&p.to_string_lossy())).unwrap_or_else(|_| "sp".into());
    let prompt = doctor_prompt(&report, &report_file, &sp);
    if print {
        println!("{prompt}");
        return Ok(());
    }
    let claude = find_claude().context("Couldn't find the claude command. Install Claude Code and sign in, then try again.")?;
    let project = project_of(project)?;
    let mut command = std::process::Command::new(&claude);
    if Path::new(&project).is_dir() {
        command.current_dir(&project);
    }
    eprintln!("Starting Claude on “{}”…", markdown::one_line(&report.title));
    use std::os::unix::process::CommandExt;
    // Replaces this process, so Claude owns the terminal.
    let failed = command.arg(prompt).exec();
    Err(failed).with_context(|| format!("Couldn't run {}", claude.display()))
}

fn cmd_action(paths: &Paths, report: &str, number: usize, status: Option<Status>, note: Option<String>) -> anyhow::Result<()> {
    let conn = index::open(&paths.db)?;
    let report = find_report(&conn, Some(report), &|| bail!("unused"))?;
    if number == 0 || number > report.actions.len() {
        bail!("Report {} has actions 1 to {}.", report.id, report.actions.len());
    }
    let status = status.map(|s| match s {
        Status::Done => reports::ActionStatus::Done,
        Status::InProgress => reports::ActionStatus::InProgress,
        Status::Declined => reports::ActionStatus::Declined,
        Status::Considering => reports::ActionStatus::Considering,
    });
    let title = markdown::one_line(&report.actions[number - 1].title);
    // A response is replaced whole, so a flag left out keeps what was recorded before.
    let before = reports::action_responses(&conn, &report.id)?.into_iter().find(|r| r.action_index == number - 1);
    if status.is_none() && note.is_none() {
        match before {
            Some(r) => println!("{number}. {title}: {}{}", r.status.map(markdown::status_label).unwrap_or("No status"), r.note.map(|n| format!(". Note: {n}")).unwrap_or_default()),
            None => println!("{number}. {title}: no response recorded"),
        }
        return Ok(());
    }
    let input = reports::ActionResponseInput {
        report_id: report.id.clone(),
        action_index: number - 1,
        status: status.or(before.as_ref().and_then(|r| r.status)),
        note: note.or(before.as_ref().and_then(|r| r.note.clone())),
        user_impact: before.as_ref().and_then(|r| r.user_impact),
    };
    let saved = reports::set_action_response(&conn, &input)?;
    let label = saved.and_then(|r| r.status).map(markdown::status_label).unwrap_or("No status");
    println!("{number}. {title}: {label}");
    Ok(())
}

fn home() -> anyhow::Result<PathBuf> {
    dirs::home_dir().context("Couldn't find the home folder")
}

fn cmd_index(paths: &Paths, full: bool) -> anyhow::Result<()> {
    let mut conn = index::open(&paths.db)?;
    register_session_folders(&conn)?;
    let roots = index::roots(&conn)?;
    if roots.is_empty() {
        bail!("There are no session folders to read: none of ~/.claude/projects, ~/.codex/sessions and ~/.pi/agent/sessions exists.");
    }
    let show = std::io::stderr().is_terminal();
    let stats = index::run(&mut conn, full, Some(&paths.library), &move |done, total| {
        if show && total > 0 && (done % 50 == 0 || done == total) {
            eprint!("\rReading sessions {done}/{total}{}", if done == total { "\n" } else { "" });
        }
    })?;
    let listed = index::list(&conn)?.len();
    println!("{} files found, {} new or changed, in {:.1}s. {} sessions listed.", stats.scanned, stats.updated + stats.failed, stats.elapsed_ms as f64 / 1000.0, listed);
    if stats.removed > 0 {
        println!("Dropped {} whose files are gone.", stats.removed);
    }
    if stats.failed > 0 {
        println!("{} files couldn't be read.", stats.failed);
    }
    println!("Session folders:");
    for root in roots {
        println!("  {root}");
    }
    Ok(())
}

fn cmd_install() -> anyhow::Result<()> {
    let dir = home()?.join(".local/bin");
    let dest = dir.join("sp");
    let running = std::env::current_exe()?;
    if std::fs::canonicalize(&dest).ok() == std::fs::canonicalize(&running).ok() {
        println!("This is already the sp at {}.", dest.display());
        return Ok(());
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("Couldn't create {}", dir.display()))?;
    // Copied beside the destination and renamed: an sp running from there can't be written over.
    let copy = dir.join(".sp-installing");
    std::fs::copy(&running, &copy).with_context(|| format!("Couldn't write to {}", dir.display()))?;
    std::fs::rename(&copy, &dest)?;
    println!("Installed {}", dest.display());
    let on_path = std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d == dir));
    if !on_path {
        println!("{} isn't on your PATH, so add it to run `sp` by name. `sp pull` from another machine finds it either way.", dir.display());
    }
    Ok(())
}

fn cmd_export_project(paths: &Paths, repo: &str, since: Option<i64>) -> anyhow::Result<()> {
    if std::io::stdout().is_terminal() {
        bail!("export-project writes compressed data for `sp pull` to read. Run `sp pull <host>` on the machine that should receive the sessions.");
    }
    let conn = open_indexed(paths)?;
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    sync::export(&conn, &home()?, &sync::ExportRequest { repo: sync::RepoName(repo.into()), since }, &mut out)?;
    out.flush()?;
    Ok(())
}

/// Stores an export for the checkout at `project`, then indexes the sessions it brought.
/// A dry run stores nothing and reports what would have been.
fn import_project(paths: &Paths, project: &str, input: &mut dyn Read, dry_run: bool) -> anyhow::Result<sync::PullSummary> {
    let conn = index::open(&paths.db)?;
    register_session_folders(&conn)?;
    let summary = sync::import(&conn, &sync::ImportTarget { project: PathBuf::from(project), home: home()?, dry_run }, input)?;
    drop(conn);
    if !dry_run {
        open_indexed(paths)?;
    }
    Ok(summary)
}

/// What a dry run found, one line per file that a pull would write or leave as
/// it is here. Files already identical aren't listed.
fn print_plan(summary: &sync::PullSummary) {
    let home = home().unwrap_or_default();
    let short = |path: &Path| match path.strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    };
    println!("Dry run: nothing was written. A pull from {} would:", summary.exporter);
    for file in &summary.files {
        let what = match file.change {
            sync::Change::Added => "add     ",
            sync::Change::Replaced => "replace ",
            sync::Change::KeptLocal => "keep    ",
            sync::Change::Unchanged => continue,
        };
        let why = if file.change == sync::Change::KeptLocal { "  (the copy here is newer)" } else { "" };
        println!("  {what}{}{why}", short(&file.path));
    }
    let same = summary.files.iter().filter(|f| f.change == sync::Change::Unchanged).count();
    if same > 0 {
        println!("  leave {same} files that are already the same here");
    }
    println!("In total, it would have this result:");
}

fn print_pull(summary: &sync::PullSummary) {
    let line = |what: &str, c: &sync::Counts| {
        let parts: Vec<String> = [(c.added, "added"), (c.replaced, "replaced with a newer copy"), (c.kept_local, "kept as they are here"), (c.unchanged, "already here")]
            .iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, label)| format!("{n} {label}"))
            .collect();
        if !parts.is_empty() {
            println!("{what}: {}", parts.join(", "));
        }
    };
    println!("From {}", summary.exporter);
    line("Sessions", &summary.sessions);
    line("Analyses", &summary.analyses);
    line("Annotations", &summary.annotations);
    line("Responses to actions", &summary.responses);
    if summary.reports_added > 0 {
        println!("Reports: {} added", summary.reports_added);
    }
    if summary.sessions == sync::Counts::default() {
        println!("The other machine has no sessions for this repository in that range.");
    }
}

fn cmd_import_project(paths: &Paths, project: &ProjectArg, dry_run: bool) -> anyhow::Result<()> {
    let project = project_of(project)?;
    let summary = import_project(paths, &project, &mut std::io::stdin().lock(), dry_run)?;
    if dry_run {
        print_plan(&summary);
    }
    print_pull(&summary);
    Ok(())
}

/// The command line that runs sp with `args` on another machine. A command
/// run over SSH doesn't get the PATH an interactive shell sets up, so the
/// place `sp install` copies to is tried before the PATH.
fn remote_command(args: &[String]) -> String {
    let args = args.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ");
    let script = format!("if [ -x \"$HOME/.local/bin/sp\" ]; then exec \"$HOME/.local/bin/sp\" {args}; else exec sp {args}; fi");
    // Wrapped in sh so it runs the same whatever the account's shell is.
    format!("sh -c {}", shell_quote(&script))
}

/// Starts sp with `args` on `host` over SSH, with its output piped back.
fn remote_sp(host: &str, args: &[String]) -> anyhow::Result<std::process::Child> {
    std::process::Command::new("ssh")
        // Without "--" a host name starting with "-" would be read as an ssh option.
        .arg("--")
        .arg(host)
        .arg(remote_command(args))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .context("Couldn't run ssh")
}

/// Waits for the sp started on `host` and explains a failure. Its own message, if any, has already been printed.
fn remote_result(host: &str, child: &mut std::process::Child, command: &str) -> anyhow::Result<()> {
    match child.wait()?.code() {
        Some(0) => Ok(()),
        Some(255) => bail!("ssh couldn't reach {host}. Check that `ssh {host}` works."),
        Some(127) => bail!("{host} has no sp. Build sp on {host} and run `sp install` there."),
        Some(2) => bail!("The sp on {host} is too old for `sp {command}`. Update it there and run `sp install` again."),
        _ => bail!("sp failed on {host}; its message is above."),
    }
}

/// The repository the scope's folder is a checkout of, and when `--since` starts.
/// The time is resolved here and sent as a number or a UTC time, so the other
/// machine's time zone doesn't change what "since" means.
fn remote_scope(scope: &Scope) -> anyhow::Result<(String, sync::RepoName, Option<i64>)> {
    let project = project_of(&scope.project)?;
    let repo = sync::repo_name(Path::new(&project)).with_context(|| format!("Couldn't work out which repository {project} is"))?;
    let since = scope.since.as_deref().map(|s| parse_since(s, Local::now())).transpose()?;
    Ok((project, repo, since))
}

fn remote_sessions(host: &str, scope: &Scope) -> anyhow::Result<()> {
    let (_, repo, since) = remote_scope(scope)?;
    let mut args = vec!["sessions".to_string(), "--repo".into(), repo.0.clone()];
    if let Some(at) = since.and_then(|ms| chrono::DateTime::from_timestamp_millis(ms)) {
        args.extend(["--since".into(), at.to_rfc3339()]);
    }
    let mut child = remote_sp(host, &args)?;
    let mut listing = String::new();
    child.stdout.take().expect("stdout is piped").read_to_string(&mut listing)?;
    remote_result(host, &mut child, "sessions <host>")?;
    if listing.trim().is_empty() {
        let range = scope.since.as_deref().map(|s| format!(" since {s}")).unwrap_or_default();
        bail!("{host} has no sessions for {}{range}.", repo.0);
    }
    print!("{listing}");
    Ok(())
}

fn cmd_pull(paths: &Paths, host: &str, scope: &Scope, dry_run: bool) -> anyhow::Result<()> {
    let (project, repo, since) = remote_scope(scope)?;
    let mut args = vec!["export-project".to_string(), "--repo".into(), repo.0.clone()];
    if let Some(since) = since {
        args.extend(["--since".into(), since.to_string()]);
    }
    eprintln!("{} {} from {host}…", if dry_run { "Checking" } else { "Pulling" }, repo.0);
    let mut child = remote_sp(host, &args)?;
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let imported = import_project(paths, &project, &mut stdout, dry_run);
    drop(stdout);
    // When the other side failed, its exit status says more than the import's complaint about an empty stream.
    remote_result(host, &mut child, "pull")?;
    let summary = imported?;
    if dry_run {
        print_plan(&summary);
    }
    print_pull(&summary);
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    let result = paths().and_then(|paths| match &cli.command {
        Command::Sessions { host, scope, repo } => cmd_sessions(&paths, host.as_deref(), scope, repo.as_deref()),
        Command::Analyze { session, scope, prompt, force } => cmd_analyze(&paths, session.as_deref(), scope, prompt, *force),
        Command::Report { scope, prompt, model, no_analyze, out } => cmd_report(&paths, scope, prompt, model, *no_analyze, out.as_deref()),
        Command::Reports => cmd_reports(&paths),
        Command::Show { report, project } => cmd_show(&paths, report.as_deref(), project),
        Command::Doctor { report, project, print } => cmd_doctor(&paths, report.as_deref(), project, *print),
        Command::Action { report, number, status, note } => cmd_action(&paths, report, *number, *status, note.clone()),
        Command::Pull { host, scope, dry_run } => cmd_pull(&paths, host, scope, *dry_run),
        Command::Index { full } => cmd_index(&paths, *full),
        Command::Install => cmd_install(),
        Command::ExportProject { repo, since } => cmd_export_project(&paths, repo, *since),
        Command::ImportProject { project, dry_run } => cmd_import_project(&paths, project, *dry_run),
    });
    if let Err(e) = result {
        if generate::is_cancelled(&e) || e.is::<Interrupted>() {
            eprintln!("Cancelled");
            std::process::exit(130);
        }
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, project: &str, started_at: i64) -> SessionRow {
        SessionRow {
            path: format!("/sessions/{id}.jsonl"),
            source: "claude".into(),
            native_id: id.into(),
            cwd: Some(project.into()),
            project: project.into(),
            title: id.into(),
            title_source: "prompt".into(),
            first_prompt: None,
            started_at: Some(started_at),
            updated_at: Some(started_at),
            user_turns: 1,
            assistant_msgs: 1,
            tool_calls: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            models: vec![],
            git_branch: None,
            is_subagent: false,
            parent_id: None,
            size: 0,
            imported_from: None,
            branches: vec![],
            fork_entry: None,
            fork_turn: None,
            error_count: 0,
            exit_count: 0,
            from_bundle: false,
        }
    }

    fn at(text: &str) -> DateTime<Local> {
        Local.from_local_datetime(&NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap()).unwrap()
    }

    #[test]
    fn since_reads_dates_times_and_ages() {
        let now = at("2026-11-14 12:00");
        assert_eq!(parse_since("2026-11-01", now).unwrap(), at("2026-11-01 00:00").timestamp_millis());
        assert_eq!(parse_since("2026-11-01T09:30", now).unwrap(), at("2026-11-01 09:30").timestamp_millis());
        assert_eq!(parse_since("2026-11-01 09:30", now).unwrap(), at("2026-11-01 09:30").timestamp_millis());
        assert_eq!(parse_since("2026-11-01T09:30:00Z", now).unwrap(), 1793525400000);
        assert_eq!(parse_since("12h", now).unwrap(), at("2026-11-14 00:00").timestamp_millis());
        assert_eq!(parse_since("7d", now).unwrap(), now.timestamp_millis() - 7 * 24 * 3_600_000);
        assert_eq!(parse_since("2w", now).unwrap(), now.timestamp_millis() - 14 * 24 * 3_600_000);
        assert!(parse_since("last tuesday", now).is_err());
        assert!(parse_since("d", now).is_err());
    }

    #[test]
    fn scope_keeps_the_projects_own_sessions_from_the_start_time_on() {
        let mut subagent = row("sub", "/repo", 300);
        subagent.is_subagent = true;
        let rows = vec![row("new", "/repo", 300), row("old", "/repo", 100), row("other", "/elsewhere", 300), subagent];
        let ids = |since| in_scope(&rows, "/repo", since).iter().map(|r| r.native_id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(None), ["new", "old"]);
        assert_eq!(ids(Some(300)), ["new"]);
        assert_eq!(ids(Some(301)), [] as [&str; 0]);
    }

    #[test]
    fn a_session_is_found_by_the_start_of_its_id() {
        let rows = vec![row("abc123", "/repo", 1), row("abd456", "/repo", 1)];
        assert_eq!(find_session(&rows, "abc").unwrap().native_id, "abc123");
        assert!(find_session(&rows, "ab").unwrap_err().to_string().contains("2 sessions"));
        assert!(find_session(&rows, "zzz").unwrap_err().to_string().contains("No session"));
    }

    #[test]
    fn shell_quoting_leaves_plain_paths_alone() {
        assert_eq!(shell_quote("/usr/local/bin/sp"), "/usr/local/bin/sp");
        assert_eq!(shell_quote("/Users/me/My Tools/sp"), "'/Users/me/My Tools/sp'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn another_machine_is_asked_for_a_repository_by_name_wherever_it_keeps_the_checkout() {
        let dir = std::env::temp_dir().join(format!("sp-in-repo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let checkout = dir.join("somewhere/Widgets");
        std::fs::create_dir_all(checkout.join(".git")).unwrap();
        std::fs::write(checkout.join(".git/config"), "[remote \"origin\"]\n\turl = git@github.com:dan/widgets.git\n").unwrap();
        let here = checkout.to_string_lossy().into_owned();
        let mut subagent = row("sub", &here, 300);
        subagent.is_subagent = true;
        let mut shared = row("shared", &here, 300);
        shared.from_bundle = true;
        let rows = vec![row("new", &here, 300), row("old", &here, 100), row("other", "/elsewhere/widgets", 300), subagent, shared];
        let ids = |repo, since| in_repo(&rows, repo, since).iter().map(|r| r.native_id.clone()).collect::<Vec<_>>();
        assert_eq!(ids("github.com/dan/widgets", None), ["new", "old"]);
        assert_eq!(ids("github.com/dan/widgets", Some(200)), ["new"]);
        assert_eq!(ids("widgets", None), ["other"], "a checkout with no origin goes by its folder's name");
    }

    #[cfg(unix)]
    #[test]
    fn the_command_for_the_other_machine_runs_the_installed_sp_with_the_repository_name_intact() {
        use std::os::unix::fs::PermissionsExt;
        let home = std::env::temp_dir().join(format!("sp-remote-command-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let installed = home.join(".local/bin/sp");
        std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
        std::fs::write(&installed, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o755)).unwrap();

        let command = remote_command(&["export-project".into(), "--repo".into(), "github.com/it's/a repo".into(), "--since".into(), "42".into()]);

        // sshd hands the command to the account's shell as one string, without ~/.local/bin on the PATH.
        let run = std::process::Command::new("sh").arg("-c").arg(&command).env("HOME", &home).env("PATH", "/usr/bin:/bin").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&run.stdout), "export-project\n--repo\ngithub.com/it's/a repo\n--since\n42\n");

        std::fs::remove_file(&installed).unwrap();
        let run = std::process::Command::new("sh").arg("-c").arg(&command).env("HOME", &home).env("PATH", "/usr/bin:/bin").output().unwrap();
        assert_eq!(run.status.code(), Some(127), "with no sp anywhere the shell reports the command as not found");
    }
}
