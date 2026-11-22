//! `sp`: Session Player from the command line. It reads and writes the same
//! database as the app, so analysis and reports made here show up there.

mod backend;
mod markdown;
mod route;

use std::collections::{HashMap, HashSet};
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, anyhow, bail};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone};
use clap::{Args, Parser, Subcommand, ValueEnum};
use rusqlite::Connection;
use session_player_client::{
    Client, ConnectParams, LocalProbe, PullChange, PullCounts, PullRequest, PullResponse, ServersStore, check_compat, connect, probe_local,
};
use session_player_core::model::SessionRow;
use session_player_core::ops::{
    self, ACTION_RESPONSES, ACTION_VERDICTS, ANALYZE_SESSIONS, ActionResponseChange, ActionResponsesRequest, ActionVerdictsRequest, AnalyzeSessionsRequest,
    CANCEL_JOB, CancelJobRequest, GENERATE_REPORT, GET_REPORT, GET_ROOTS, GenerateReportRequest, GetReportRequest, GetRootsRequest, HELLO, HelloRequest, Job,
    JobId, JobState, LIST_REPORTS, LIST_SESSIONS, ListReportsRequest, ListSessionsRequest, NOTE_SUMMARIES, NoteSummariesRequest, Paths, PushedEvent,
    REINDEX, REPORT_ANNOTATIONS, ReindexRequest, ReportAnnotationsRequest, SESSION_NOTES, SET_ACTION_RESPONSE, SessionNotesRequest, SetActionResponseRequest,
};
use session_player_core::{annotations, generate, index, reports, sync};
use session_player_server::pairing::{DeviceInfo, DevicesRequest, PairRequest, PairResponse, UnpairRequest};
use session_player_server::service::{ServiceAction, ServiceReport, ServiceRequest};
use session_player_server::{ServeParams, ServerConfig, serve};

use backend::{Backend, Detached, RunParams};
use route::{Route, RouteParams};

/// The app's identifier, which names its data folder.
const APP_ID: &str = "com.danmactough.sessionplayer";

#[derive(Parser)]
#[command(name = "sp", version, about = "Session Player from the command line")]
struct Cli {
    /// Run in this process on this machine's database, even when a server is running here
    #[arg(long, global = true, conflicts_with = "server")]
    local: bool,
    /// Run on a server paired with `sp connect`, by the name it was saved under
    #[arg(long, global = true, value_name = "NAME")]
    server: Option<String>,
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
    /// The other machine needs sp too, installed with `sp install`.
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
    /// Put sp at ~/.local/bin/sp, where your shell and `sp pull` on another machine look for it
    ///
    /// The sp inside the Mac app is linked, so updating the app updates it. Anywhere else it is copied.
    Install,
    /// Run the Session Player server on this machine's data folder until stopped
    ///
    /// Settings are read from server.json in the data folder.
    Serve,
    /// Print a pairing string another machine passes to `sp connect`
    Pair,
    /// Pair with a server, using the string `sp pair` printed there
    Connect {
        /// The pairing string, starting with sp1.
        pairing: String,
        /// The name to save the server under, which `--server` and `sp pull` take
        #[arg(long)]
        name: String,
    },
    /// List the devices paired with this machine's server
    Devices,
    /// Revoke a paired device's access to this machine's server
    Unpair {
        /// The device's id, as `sp devices` lists it
        device: String,
    },
    /// Install, remove, start, stop or check the server as a per-user service (launchd or systemd)
    Service {
        #[arg(value_enum)]
        action: ServiceArg,
    },
    /// Stop a job running on the server
    Cancel {
        /// The job's id, as printed when it was started or detached from
        job: String,
    },
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
enum ServiceArg {
    Install,
    Uninstall,
    Start,
    Stop,
    Status,
}

#[derive(Clone, Copy, ValueEnum)]
enum Status {
    Done,
    InProgress,
    Declined,
    Considering,
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
    Ok(Paths { db: data.join("index.sqlite"), library: data.join("library"), logs, data })
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

/// Where the command runs (see `route`), and the paired server's name when it is one.
fn open_backend(cli: &Cli, paths: &Paths, index: bool) -> anyhow::Result<(Backend, Option<String>)> {
    let servers = match &cli.server {
        Some(_) => ServersStore::new(&paths.data).list()?,
        None => Vec::new(),
    };
    let probe = if cli.local || cli.server.is_some() { LocalProbe::NotLive } else { probe_local(&paths.data) };
    let route = route::resolve(RouteParams { local_flag: cli.local, server_flag: cli.server.as_deref(), probe, servers: &servers })?;
    let remote = match &route {
        Route::Remote { name } => Some(name.clone()),
        _ => None,
    };
    Ok((Backend::open(backend::OpenParams { route, paths, index })?, remote))
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

/// A time resolved here, sent as UTC, so the other machine's time zone doesn't change what "since" means.
fn utc(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.to_rfc3339())
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

/// The project's sessions and the name to report them under. A paired server keeps its checkout
/// in a folder of its own, so there they are the sessions of the same repository, without
/// imported copies, as `sp sessions <host>` lists them.
fn scope_rows(backend: &Backend, remote: Option<&str>, project: &ProjectArg, since: Option<i64>) -> anyhow::Result<(String, Vec<SessionRow>)> {
    let project = project_of(project)?;
    if remote.is_some() {
        let repo = sync::repo_name(Path::new(&project)).with_context(|| format!("Couldn't work out which repository {project} is"))?;
        let rows = backend.call(&LIST_SESSIONS, &ListSessionsRequest { repo: Some(repo.0.clone()), since: since.and_then(utc) })?;
        return Ok((repo.0, rows.into_iter().filter(|r| !r.is_subagent && !r.from_bundle).collect()));
    }
    let rows = backend.call(&LIST_SESSIONS, &ListSessionsRequest::default())?;
    let found = in_scope(&rows, &project, since).into_iter().cloned().collect();
    Ok((project, found))
}

fn scoped(backend: &Backend, remote: Option<&str>, scope: &Scope) -> anyhow::Result<(String, Vec<SessionRow>)> {
    let since = scope.since.as_deref().map(|s| parse_since(s, Local::now())).transpose()?;
    let (label, found) = scope_rows(backend, remote, &scope.project, since)?;
    if found.is_empty() {
        let range = scope.since.as_deref().map(|s| format!(" since {s}")).unwrap_or_default();
        bail!("No sessions found for {label}{range}.");
    }
    Ok((label, found))
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

fn find_claude() -> Option<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let on_path: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).map(|d| d.join("claude")).collect()).unwrap_or_default();
    on_path.into_iter().chain([home.join(".local/bin/claude"), home.join(".claude/local/claude")]).find(|p| p.is_file())
}

/// The focus text the flags give. `None` leaves it to wherever the command
/// runs: the prompt saved in the app there, else the built-in one.
fn focus(prompt: &Prompt) -> anyhow::Result<Option<String>> {
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
    if given.as_ref().is_some_and(|text| text.trim().is_empty()) {
        bail!("The prompt is empty.");
    }
    Ok(given)
}

/// A flag that Ctrl-C sets. In this process it stops the `claude` calls in
/// progress (they run in process groups of their own, so the terminal's
/// interrupt doesn't reach them); on a server it detaches. A second Ctrl-C
/// quits at once.
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

/// Ends the command the way the job ended.
fn job_outcome(job: &Job) -> anyhow::Result<()> {
    match job.state {
        JobState::Finished | JobState::Running => Ok(()),
        JobState::Cancelled => Err(Interrupted.into()),
        JobState::Failed => bail!("{}", job.error.as_deref().unwrap_or("The job failed.")),
    }
}

fn analyzed_keys(backend: &Backend) -> anyhow::Result<HashSet<String>> {
    Ok(backend.call(&NOTE_SUMMARIES, &NoteSummariesRequest {})?.into_iter().filter(|s| s.analyzed).map(|s| s.key).collect())
}

fn print_analysis(backend: &Backend, row: &SessionRow) -> anyhow::Result<()> {
    let notes = backend.call(&SESSION_NOTES, &SessionNotesRequest { key: annotations::session_key(row), path: row.path.clone(), branch: None })?;
    let meta = notes.meta.unwrap_or_default();
    println!("{}", markdown::one_line(&row.title));
    let note = meta.outcome_note.as_deref().map(|n| format!(": {}", markdown::one_line(n))).unwrap_or_default();
    println!("Outcome: {}{note}", markdown::outcome_label(meta.outcome));
    if !meta.components.is_empty() {
        println!("Components: {}", meta.components.join(", "));
    }
    if !notes.annotations.is_empty() {
        println!("Annotations:");
        for a in notes.annotations {
            println!("- {}: {}", markdown::kind_label(a.kind), markdown::one_line(&a.note));
        }
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

fn cmd_sessions(backend: &Backend, remote: Option<&str>, scope: &Scope, repo: Option<&str>) -> anyhow::Result<()> {
    let found: Vec<SessionRow> = match repo {
        Some(repo) => {
            let rows = backend.call(&LIST_SESSIONS, &ListSessionsRequest::default())?;
            in_repo(&rows, repo, scope.since.as_deref().map(|s| parse_since(s, Local::now())).transpose()?).into_iter().cloned().collect()
        }
        None => scoped(backend, remote, scope)?.1,
    };
    let analyzed = analyzed_keys(backend)?;
    for r in &found {
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

/// Runs an Analyze job on `paths`, reporting progress on standard error.
fn analyze(backend: &Backend, req: AnalyzeSessionsRequest, cancel: &AtomicBool) -> anyhow::Result<()> {
    let job = backend.run(RunParams {
        start: &|b| b.call(&ANALYZE_SESSIONS, &req),
        on_event: &mut |e| {
            if let PushedEvent::AnalyzeProgress(p) = &e.event
                && p.done > 0
            {
                eprintln!("[{}/{}] done", p.done, p.total);
            }
        },
        cancel,
    })?;
    job_outcome(&job)
}

fn cmd_analyze(backend: &Backend, remote: Option<&str>, session: Option<&str>, scope: &Scope, prompt: &Prompt, force: bool) -> anyhow::Result<()> {
    let focus = focus(prompt)?;
    let cancel = cancel_on_interrupt()?;
    if session.is_none() && scope.since.is_some() {
        let (_, found) = scoped(backend, remote, scope)?;
        let analyzed = analyzed_keys(backend)?;
        // A pi session's branches share one analysis.
        let mut seen = HashSet::new();
        let todo: Vec<String> = found
            .iter()
            .filter(|r| force || !analyzed.contains(&annotations::session_key(r)))
            .filter(|r| seen.insert(annotations::session_key(r)))
            .map(|r| r.path.clone())
            .collect();
        if todo.is_empty() {
            eprintln!("Every session in range is already analyzed. Use --force to analyze them again.");
            return Ok(());
        }
        return analyze(backend, AnalyzeSessionsRequest { paths: todo, force, focus }, &cancel);
    }
    let row = match session {
        Some(arg) => find_session(&backend.call(&LIST_SESSIONS, &ListSessionsRequest::default())?, arg)?.clone(),
        None => {
            let (label, rows) = scope_rows(backend, remote, &scope.project, None)?;
            rows.into_iter().next().with_context(|| format!("No sessions found for {label}."))?
        }
    };
    eprintln!("Analyzing {}…", markdown::one_line(&row.title));
    analyze(backend, AnalyzeSessionsRequest { paths: vec![row.path.clone()], force: true, focus }, &cancel)?;
    print_analysis(backend, &row)
}

fn render(backend: &Backend, report: &reports::Report) -> anyhow::Result<String> {
    let id = report.id.clone();
    let cited = backend.call(&REPORT_ANNOTATIONS, &ReportAnnotationsRequest { id: id.clone() })?;
    let responses = markdown::Responses {
        responses: backend.call(&ACTION_RESPONSES, &ActionResponsesRequest { id: id.clone() })?,
        verdicts: backend.call(&ACTION_VERDICTS, &ActionVerdictsRequest { id })?,
    };
    Ok(markdown::report_markdown(report, &cited, &responses))
}

fn cmd_report(backend: &Backend, remote: Option<&str>, scope: &Scope, prompt: &Prompt, model: &str, no_analyze: bool, out: Option<&Path>) -> anyhow::Result<()> {
    let (label, found) = scoped(backend, remote, scope)?;
    let focus = focus(prompt)?;
    let cancel = cancel_on_interrupt()?;

    let mut seen = HashSet::new();
    let found: Vec<SessionRow> = found.into_iter().filter(|r| seen.insert(annotations::session_key(r))).collect();
    let analyzed = analyzed_keys(backend)?;
    let unanalyzed = found.iter().filter(|r| !analyzed.contains(&annotations::session_key(r))).count();
    let writing = if no_analyze {
        let picks = found.len() - unanalyzed;
        if picks == 0 {
            bail!("None of the {} sessions for {label} has an analysis to report on.", found.len());
        }
        picks
    } else {
        if unanalyzed > 0 {
            eprintln!("Analyzing {unanalyzed} of {} sessions first…", found.len());
        }
        found.len()
    };

    eprintln!("Writing a report on {writing} sessions with {model}…");
    let req = GenerateReportRequest { paths: found.iter().map(|r| r.path.clone()).collect(), model: Some(model.to_string()), focus, analyze_first: !no_analyze };
    let mut saved = None;
    let job = backend.run(RunParams {
        start: &|b| b.call(&GENERATE_REPORT, &req),
        on_event: &mut |e| match &e.event {
            PushedEvent::ReportProgress(p) if p.total > 1 => eprintln!("Part {} of {} done", p.done, p.total),
            PushedEvent::ReportDone(d) => saved = d.report.clone(),
            _ => {}
        },
        cancel: &cancel,
    })?;
    job_outcome(&job)?;
    let id = saved.context("The report was written, but its id didn't arrive. Find it with: sp reports")?;
    let report = backend.call(&GET_REPORT, &GetReportRequest { id: id.clone() })?.context("That report no longer exists.")?;

    let text = render(backend, &report)?;
    match out {
        Some(file) => std::fs::write(file, text).with_context(|| format!("Couldn't write {}", file.display()))?,
        None => std::io::stdout().write_all(text.as_bytes())?,
    }
    eprintln!("Saved as report {id}. Work through it with: sp doctor {id}");
    Ok(())
}

fn cmd_reports(backend: &Backend) -> anyhow::Result<()> {
    for r in backend.call(&LIST_REPORTS, &ListReportsRequest {})? {
        let when = Local.timestamp_millis_opt(r.created_at).single().map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
        println!("{}  {when}  {} sessions, {} actions  {}", r.id, r.sessions, r.actions, markdown::one_line(&r.title));
    }
    Ok(())
}

/// The report `arg` names by id or its start. With no `arg`, the newest
/// report that covers one of the sessions `project` gives (with its name).
fn find_report(backend: &Backend, arg: Option<&str>, project: &dyn Fn() -> anyhow::Result<(String, HashSet<String>)>) -> anyhow::Result<reports::Report> {
    let all = backend.call(&LIST_REPORTS, &ListReportsRequest {})?;
    let load = |id: &str| backend.call(&GET_REPORT, &GetReportRequest { id: id.to_string() })?.context("That report no longer exists.");
    if let Some(arg) = arg {
        let matches: Vec<&reports::ReportSummary> = all.iter().filter(|r| r.id.starts_with(arg)).collect();
        return match matches.len() {
            0 => bail!("No report has an id starting with {arg}. List them with: sp reports"),
            1 => load(&matches[0].id),
            n => bail!("{n} reports have an id starting with {arg}. Give more of the id."),
        };
    }
    let (project, keys) = project()?;
    for summary in &all {
        let report = load(&summary.id)?;
        if report.sessions.iter().any(|s| keys.contains(&s.key)) {
            return Ok(report);
        }
    }
    bail!("No report covers {project} yet. Write one with: sp report --since 7d")
}

/// The project's name and its sessions' keys, for `find_report`.
fn project_keys(backend: &Backend, remote: Option<&str>, project: &ProjectArg) -> anyhow::Result<(String, HashSet<String>)> {
    let (label, rows) = scope_rows(backend, remote, project, None)?;
    Ok((label, rows.iter().map(annotations::session_key).collect()))
}

fn cmd_show(backend: &Backend, remote: Option<&str>, report: Option<&str>, project: &ProjectArg) -> anyhow::Result<()> {
    let report = find_report(backend, report, &|| project_keys(backend, remote, project))?;
    std::io::stdout().write_all(render(backend, &report)?.as_bytes())?;
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

fn cmd_doctor(backend: &Backend, remote: Option<&str>, paths: &Paths, report: Option<&str>, project: &ProjectArg, print: bool) -> anyhow::Result<()> {
    let report = find_report(backend, report, &|| project_keys(backend, remote, project))?;
    if report.actions.is_empty() {
        bail!("Report {} recommends no actions.", report.id);
    }
    let dir = paths.data.join("doctor");
    std::fs::create_dir_all(&dir)?;
    let report_file = dir.join(format!("{}.md", report.id));
    std::fs::write(&report_file, render(backend, &report)?)?;

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

fn cmd_action(backend: &Backend, report: &str, number: usize, status: Option<Status>, note: Option<String>) -> anyhow::Result<()> {
    let report = find_report(backend, Some(report), &|| bail!("unused"))?;
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
    let before = backend.call(&ACTION_RESPONSES, &ActionResponsesRequest { id: report.id.clone() })?.into_iter().find(|r| r.action_index == number - 1);
    if status.is_none() && note.is_none() {
        match before {
            Some(r) => println!("{number}. {title}: {}{}", r.status.map(markdown::status_label).unwrap_or("No status"), r.note.map(|n| format!(". Note: {n}")).unwrap_or_default()),
            None => println!("{number}. {title}: no response recorded"),
        }
        return Ok(());
    }
    let input = ActionResponseChange {
        report_id: report.id.clone(),
        action_index: number - 1,
        status: status.or(before.as_ref().and_then(|r| r.status)),
        note: note.or(before.as_ref().and_then(|r| r.note.clone())),
        user_impact: before.as_ref().and_then(|r| r.user_impact),
    };
    let saved = backend.call(&SET_ACTION_RESPONSE, &SetActionResponseRequest { input })?;
    let label = saved.and_then(|r| r.status).map(markdown::status_label).unwrap_or("No status");
    println!("{number}. {title}: {label}");
    Ok(())
}

fn home() -> anyhow::Result<PathBuf> {
    dirs::home_dir().context("Couldn't find the home folder")
}

fn cmd_index(backend: &Backend, full: bool) -> anyhow::Result<()> {
    let roots = backend.call(&GET_ROOTS, &GetRootsRequest {})?;
    if roots.is_empty() {
        bail!("There are no session folders to read: none of ~/.claude/projects, ~/.codex/sessions and ~/.pi/agent/sessions exists.");
    }
    let show = std::io::stderr().is_terminal();
    let mut stats = None;
    // An index pass can't be cancelled, so Ctrl-C just ends sp.
    let never = AtomicBool::new(false);
    let job = backend.run(RunParams {
        start: &|b| b.call(&REINDEX, &ReindexRequest { full }),
        on_event: &mut |e| match &e.event {
            PushedEvent::IndexProgress(p) if show && p.total > 0 && (p.done % 50 == 0 || p.done == p.total) => {
                eprint!("\rReading sessions {}/{}{}", p.done, p.total, if p.done == p.total { "\n" } else { "" })
            }
            PushedEvent::IndexDone(s) => stats = Some(s.clone()),
            _ => {}
        },
        cancel: &never,
    })?;
    job_outcome(&job)?;
    let listed = backend.call(&LIST_SESSIONS, &ListSessionsRequest::default())?.len();
    match stats {
        Some(stats) => {
            println!("{} files found, {} new or changed, in {:.1}s. {} sessions listed.", stats.scanned, stats.updated + stats.failed, stats.elapsed_ms as f64 / 1000.0, listed);
            if stats.removed > 0 {
                println!("Dropped {} whose files are gone.", stats.removed);
            }
            if stats.failed > 0 {
                println!("{} files couldn't be read.", stats.failed);
            }
        }
        None => println!("{listed} sessions listed."),
    }
    println!("Session folders:");
    for root in roots {
        println!("  {}", root.path);
    }
    Ok(())
}

/// Whether a program at `path` is inside a Mac app bundle. The app's updater
/// replaces the whole bundle, so a link to the sp inside stays current where a copy would go stale.
fn in_app_bundle(path: &Path) -> bool {
    path.parent().is_some_and(|dir| dir.ends_with("Contents/MacOS") && dir.ancestors().any(|a| a.extension().is_some_and(|e| e == "app")))
}

fn cmd_install() -> anyhow::Result<()> {
    let dir = home()?.join(".local/bin");
    let dest = dir.join("sp");
    let running = std::env::current_exe()?;
    let link = in_app_bundle(&running);
    let already = if link { std::fs::read_link(&dest).is_ok_and(|to| to == running) } else { std::fs::canonicalize(&dest).ok() == std::fs::canonicalize(&running).ok() };
    if already {
        println!("{} is already this sp.", dest.display());
        return Ok(());
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("Couldn't create {}", dir.display()))?;
    // Made beside the destination and renamed: an sp running from there can't be written over.
    let new = dir.join(".sp-installing");
    let _ = std::fs::remove_file(&new);
    if link {
        #[cfg(unix)]
        std::os::unix::fs::symlink(&running, &new).with_context(|| format!("Couldn't write to {}", dir.display()))?;
    } else {
        std::fs::copy(&running, &new).with_context(|| format!("Couldn't write to {}", dir.display()))?;
    }
    std::fs::rename(&new, &dest)?;
    if link {
        println!("Linked {} to {}", dest.display(), running.display());
    } else {
        println!("Installed {}", dest.display());
    }
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
        Some(127) => bail!("{host} has no sp. Install it there; see \"Use the command line\" in the README."),
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
    if let Some(at) = since.and_then(utc) {
        args.extend(["--since".into(), at]);
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

/// `sync::PullSummary` from the `pull` operation's answer.
fn pull_summary(pulled: PullResponse) -> sync::PullSummary {
    let counts = |c: PullCounts| sync::Counts { added: c.added, replaced: c.replaced, kept_local: c.kept_local, unchanged: c.unchanged };
    sync::PullSummary {
        exporter: pulled.exporter,
        files: pulled
            .files
            .into_iter()
            .map(|f| sync::PulledFile {
                path: PathBuf::from(f.path),
                change: match f.change {
                    PullChange::Added => sync::Change::Added,
                    PullChange::Replaced => sync::Change::Replaced,
                    PullChange::KeptLocal => sync::Change::KeptLocal,
                    PullChange::Unchanged => sync::Change::Unchanged,
                },
                is_session: f.is_session,
            })
            .collect(),
        sessions: counts(pulled.sessions),
        analyses: counts(pulled.analyses),
        annotations: counts(pulled.annotations),
        responses: counts(pulled.responses),
        reports_added: pulled.reports_added,
    }
}

/// A server paired with `sp connect` is pulled from over HTTPS; any other name is a host for ssh.
fn cmd_pull(cli: &Cli, paths: &Paths, host: &str, scope: &Scope, dry_run: bool) -> anyhow::Result<()> {
    let (project, repo, since) = remote_scope(scope)?;
    eprintln!("{} {} from {host}…", if dry_run { "Checking" } else { "Pulling" }, repo.0);
    let summary = if ServersStore::new(&paths.data).get(host)?.is_some() {
        let (backend, _) = open_backend(cli, paths, true)?;
        let pulled: PullResponse = backend.call_named("pull", &PullRequest { server: host.to_string(), project, since: since.and_then(utc), dry_run })?;
        pull_summary(pulled)
    } else {
        let mut args = vec!["export-project".to_string(), "--repo".into(), repo.0.clone()];
        if let Some(since) = since {
            args.extend(["--since".into(), since.to_string()]);
        }
        let mut child = remote_sp(host, &args)?;
        let mut stdout = child.stdout.take().expect("stdout is piped");
        let imported = import_project(paths, &project, &mut stdout, dry_run);
        drop(stdout);
        // When the other side failed, its exit status says more than the import's complaint about an empty stream.
        remote_result(host, &mut child, "pull")?;
        imported?
    };
    if dry_run {
        print_plan(&summary);
    }
    print_pull(&summary);
    Ok(())
}

fn cmd_serve(paths: &Paths) -> anyhow::Result<()> {
    let file = paths.data.join("server.json");
    let config: ServerConfig = match std::fs::read(&file) {
        Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| format!("Couldn't read {}; fix or remove it", file.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ServerConfig::default(),
        Err(e) => return Err(e).with_context(|| format!("Couldn't read {}", file.display())),
    };
    let ctx = backend::open_ctx(paths)?;
    // Ctrl-C and SIGTERM (a service manager stopping the server) both shut it down cleanly.
    let (stop, stopped) = std::sync::mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = stop.send(());
    })?;
    let server = serve(ServeParams { ctx: ctx.clone(), data: paths.data.clone(), config })?;
    // Sessions written while no server ran; the watcher picks up the ones written from now on.
    ops::call(&ctx, &REINDEX, &ReindexRequest { full: false }).map_err(backend::op_error)?;
    eprintln!("Serving {} on http://{}", paths.data.display(), server.addrs().loopback);
    let _ = stopped.recv();
    server.shutdown();
    Ok(())
}

fn cmd_pair(backend: &Backend) -> anyhow::Result<()> {
    let offer: PairResponse = backend.call_named("pair", &PairRequest {})?;
    println!("{}", offer.pairing);
    eprintln!("On the other machine, run: sp connect <the string above> --name <a name for this machine>");
    eprintln!("The string works once, within 5 minutes.");
    Ok(())
}

/// This machine's name, which the server lists it under.
fn hostname() -> anyhow::Result<String> {
    let output = std::process::Command::new("hostname").output().context("Couldn't run hostname")?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() {
        bail!("hostname printed no name");
    }
    Ok(name)
}

fn cmd_connect(paths: &Paths, pairing: &str, name: &str) -> anyhow::Result<()> {
    let mut server = connect(ConnectParams { pairing, name: &hostname()? }).map_err(|e| anyhow!(e.message))?;
    server.name = name.to_string();
    let hello = Client::paired(&server)?.call(&HELLO, &HelloRequest {}).map_err(|e| anyhow!(e.message))?;
    check_compat(&hello).map_err(|e| anyhow!(e))?;
    println!("Paired with {}:{} as {name}. Use it with: sp --server {name} <command>", server.address, server.port);
    ServersStore::new(&paths.data).save(server)
}

fn cmd_devices(backend: &Backend) -> anyhow::Result<()> {
    let devices: Vec<DeviceInfo> = backend.call_named("devices", &DevicesRequest {})?;
    if devices.is_empty() {
        println!("No devices are paired.");
    }
    for d in devices {
        let when = Local.timestamp_millis_opt(d.paired_at as i64).single().map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
        println!("{}  {when}  {}", d.id.0, d.name);
    }
    Ok(())
}

fn cmd_unpair(backend: &Backend, device: &str) -> anyhow::Result<()> {
    let () = backend.call_named("unpair", &UnpairRequest { id: ops::DeviceId(device.to_string()) })?;
    println!("Unpaired {device}.");
    Ok(())
}

fn cmd_service(cli: &Cli, paths: &Paths, action: ServiceArg) -> anyhow::Result<()> {
    if cli.server.is_some() {
        bail!("sp service manages the server on this machine only. Run it on that machine, without --server.");
    }
    let action = match action {
        ServiceArg::Install => ServiceAction::Install,
        ServiceArg::Uninstall => ServiceAction::Uninstall,
        ServiceArg::Start => ServiceAction::Start,
        ServiceArg::Stop => ServiceAction::Stop,
        ServiceArg::Status => ServiceAction::Status,
    };
    let backend = Backend::open(backend::OpenParams { route: Route::Local, paths, index: false })?;
    let report: ServiceReport = backend.call_named("service", &ServiceRequest { action })?;
    println!("{}: {}", if report.installed { "Installed" } else { "Not installed" }, report.unit_file.display());
    println!("{}", if report.running { "Running" } else { "Not running" });
    Ok(())
}

fn cmd_cancel(backend: &Backend, job: &str) -> anyhow::Result<()> {
    backend.call(&CANCEL_JOB, &CancelJobRequest { id: JobId(job.to_string()) })?;
    println!("Cancelling {job}.");
    Ok(())
}

fn run(cli: &Cli, paths: &Paths) -> anyhow::Result<()> {
    let open = |index: bool| open_backend(cli, paths, index);
    match &cli.command {
        Command::Sessions { host: Some(host), scope, .. } => remote_sessions(host, scope),
        Command::Sessions { host: None, scope, repo } => {
            let (b, remote) = open(true)?;
            cmd_sessions(&b, remote.as_deref(), scope, repo.as_deref())
        }
        Command::Analyze { session, scope, prompt, force } => {
            let (b, remote) = open(true)?;
            cmd_analyze(&b, remote.as_deref(), session.as_deref(), scope, prompt, *force)
        }
        Command::Report { scope, prompt, model, no_analyze, out } => {
            let (b, remote) = open(true)?;
            cmd_report(&b, remote.as_deref(), scope, prompt, model, *no_analyze, out.as_deref())
        }
        Command::Reports => cmd_reports(&open(false)?.0),
        Command::Show { report, project } => {
            let (b, remote) = open(false)?;
            cmd_show(&b, remote.as_deref(), report.as_deref(), project)
        }
        Command::Doctor { report, project, print } => {
            let (b, remote) = open(true)?;
            cmd_doctor(&b, remote.as_deref(), paths, report.as_deref(), project, *print)
        }
        Command::Action { report, number, status, note } => cmd_action(&open(false)?.0, report, *number, *status, note.clone()),
        Command::Pull { host, scope, dry_run } => cmd_pull(cli, paths, host, scope, *dry_run),
        // The job is the index pass, so none runs first.
        Command::Index { full } => cmd_index(&open(false)?.0, *full),
        Command::Install => cmd_install(),
        Command::Serve => cmd_serve(paths),
        Command::Pair => cmd_pair(&open(false)?.0),
        Command::Connect { pairing, name } => cmd_connect(paths, pairing, name),
        Command::Devices => cmd_devices(&open(false)?.0),
        Command::Unpair { device } => cmd_unpair(&open(false)?.0, device),
        Command::Service { action } => cmd_service(cli, paths, *action),
        Command::Cancel { job } => cmd_cancel(&open(false)?.0, job),
        Command::ExportProject { repo, since } => cmd_export_project(paths, repo, *since),
        Command::ImportProject { project, dry_run } => cmd_import_project(paths, project, *dry_run),
    }
}

fn main() {
    let cli = Cli::parse();
    let result = paths().and_then(|paths| run(&cli, &paths));
    if let Err(e) = result {
        if let Some(detached) = e.downcast_ref::<Detached>() {
            eprintln!("{detached}");
            std::process::exit(130);
        }
        if generate::is_cancelled(&e) || e.is::<Interrupted>() {
            eprintln!("Cancelled");
            std::process::exit(130);
        }
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
#[path = "../../testutil/tempdir.rs"]
mod tempdir;

#[cfg(test)]
mod tests {
    use super::*;
    use super::tempdir::TempDir;

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
    fn only_the_sp_inside_the_mac_app_is_linked_instead_of_copied() {
        assert!(in_app_bundle(Path::new("/Applications/Session Player.app/Contents/MacOS/sp")));
        assert!(!in_app_bundle(Path::new("/home/me/.local/bin/sp")));
        assert!(!in_app_bundle(Path::new("/Users/me/code/session-player/src-tauri/target/release/sp")));
        assert!(!in_app_bundle(Path::new("/Users/me/my.app/notes/sp")));
    }

    #[test]
    fn another_machine_is_asked_for_a_repository_by_name_wherever_it_keeps_the_checkout() {
        let dir = TempDir::new("in-repo");
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
        let home = TempDir::new("remote-command");
        let installed = home.join(".local/bin/sp");
        std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
        std::fs::write(&installed, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o755)).unwrap();

        let command = remote_command(&["export-project".into(), "--repo".into(), "github.com/it's/a repo".into(), "--since".into(), "42".into()]);

        // sshd hands the command to the account's shell as one string, without ~/.local/bin on the PATH.
        let run = std::process::Command::new("sh").arg("-c").arg(&command).env("HOME", home.path()).env("PATH", "/usr/bin:/bin").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&run.stdout), "export-project\n--repo\ngithub.com/it's/a repo\n--since\n42\n");

        std::fs::remove_file(&installed).unwrap();
        let run = std::process::Command::new("sh").arg("-c").arg(&command).env("HOME", home.path()).env("PATH", "/usr/bin:/bin").output().unwrap();
        assert_eq!(run.status.code(), Some(127), "with no sp anywhere the shell reports the command as not found");
    }
}
