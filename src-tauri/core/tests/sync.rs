// Copying a repository's sessions, analysis and reports from one machine to
// another: what travels, where it lands, and which copy wins when both
// machines have one.
use std::path::{Path, PathBuf};

use serde_json::json;
use session_player_core::test_support::{annotations as notes, generate, index, reports, sync};

/// One machine: a home folder, a checkout of the repository, and the app's database.
struct Machine {
    home: PathBuf,
    project: PathBuf,
    conn: rusqlite::Connection,
}

/// A machine whose checkout is at `checkout` below its own temporary folder.
fn machine(test: &str, name: &str, checkout: &str) -> Machine {
    let dir = std::env::temp_dir().join(format!("sp-sync-{test}-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let home = dir.join("home");
    let project = dir.join(checkout);
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(project.join(".git")).unwrap();
    let conn = index::open(&dir.join("index.sqlite")).unwrap();
    Machine { home, project, conn }
}

/// The folder Claude Code keeps a working directory's sessions in.
fn claude_folder(m: &Machine, cwd: &Path) -> PathBuf {
    let name: String = cwd.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    m.home.join(".claude/projects").join(name)
}

fn claude_file(m: &Machine, id: &str) -> PathBuf {
    claude_folder(m, &m.project).join(format!("{id}.jsonl"))
}

fn turn(m: &Machine, id: &str, day: &str, minute: u32, prompt: &str) -> String {
    let user = json!({ "type": "user", "sessionId": id, "cwd": m.project, "timestamp": format!("{day}T10:{minute:02}:00Z"),
                       "message": { "role": "user", "content": prompt } });
    let assistant = json!({ "type": "assistant", "sessionId": id, "timestamp": format!("{day}T10:{minute:02}:30Z"),
                            "message": { "id": format!("m{minute}"), "model": "claude-opus-5-5", "role": "assistant",
                                         "content": [{ "type": "text", "text": format!("Done: {prompt}") }] } });
    format!("{user}\n{assistant}\n")
}

/// Writes a Claude Code session held on 2026-03-01, with one turn per prompt a minute apart.
fn claude_session(m: &Machine, id: &str, prompts: &[&str]) {
    claude_session_on(m, id, "2026-03-01", prompts);
}

fn claude_session_on(m: &Machine, id: &str, day: &str, prompts: &[&str]) {
    let text: String = prompts.iter().enumerate().map(|(i, p)| turn(m, id, day, i as u32, p)).collect();
    let file = claude_file(m, id);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

fn reindex(m: &mut Machine) {
    for root in [".claude/projects", ".codex/sessions", ".pi/agent/sessions"] {
        let root = m.home.join(root);
        if root.is_dir() {
            index::add_root(&m.conn, &root.to_string_lossy()).unwrap();
        }
    }
    index::run(&mut m.conn, false, None, &|_, _| {}).unwrap();
}

fn export(from: &mut Machine, since: Option<i64>) -> Vec<u8> {
    let repo = sync::repo_name(&from.project).unwrap();
    export_repo(from, repo, since)
}

fn export_repo(from: &mut Machine, repo: sync::RepoName, since: Option<i64>) -> Vec<u8> {
    reindex(from);
    let mut out = Vec::new();
    sync::export(&from.conn, &from.home, &sync::ExportRequest { repo, since }, &mut out).unwrap();
    out
}

fn import(to: &mut Machine, bytes: &[u8]) -> anyhow::Result<sync::PullSummary> {
    let target = sync::ImportTarget { project: to.project.clone(), home: to.home.clone() };
    let summary = sync::import(&to.conn, &target, &mut &bytes[..])?;
    index::run(&mut to.conn, false, None, &|_, _| {}).unwrap();
    Ok(summary)
}

/// What `sp pull` does: the receiving machine names the repository its checkout
/// is of, and the other machine exports what it has of that repository.
fn pull(from: &mut Machine, to: &mut Machine) -> sync::PullSummary {
    let bytes = export_repo(from, sync::repo_name(&to.project).unwrap(), None);
    import(to, &bytes).unwrap()
}

/// (native id, project) of every listed session, ordered by id.
fn listed(m: &Machine) -> Vec<(String, String)> {
    let mut rows: Vec<_> = index::list(&m.conn).unwrap().into_iter().map(|r| (r.native_id, r.project)).collect();
    rows.sort();
    rows
}

#[test]
fn a_pulled_session_lists_under_the_local_checkout_whatever_path_the_other_machine_recorded() {
    let mut laptop = machine("lists", "laptop", "Users/dan/dev/widgets");
    let mut desktop = machine("lists", "desktop", "code/widgets");
    claude_session(&laptop, "s1", &["Fix the login test"]);

    let summary = pull(&mut laptop, &mut desktop);

    assert_eq!(summary.sessions, sync::Counts { added: 1, ..Default::default() });
    assert_eq!(listed(&desktop), [("s1".to_string(), desktop.project.to_string_lossy().into_owned())]);
    assert_eq!(std::fs::read(claude_file(&desktop, "s1")).unwrap(), std::fs::read(claude_file(&laptop, "s1")).unwrap());
    let row = index::list(&desktop.conn).unwrap().remove(0);
    assert_eq!(row.cwd.as_deref(), Some(&*desktop.project.to_string_lossy()), "resuming runs in a folder that exists here, where the agent now keeps the session");
}

fn key(id: &str) -> String {
    format!("local:claude:{id}")
}

fn at() -> Vec<notes::EventRef> {
    vec![serde_json::from_value(json!({ "anchor": "e0", "kind": "user", "ts": null })).unwrap()]
}

/// An annotation a person wrote on the session.
fn annotate(m: &Machine, id: &str, note: &str) -> notes::Annotation {
    notes::save_annotation(&m.conn, notes::Annotation::new(&key(id), notes::Kind::KeyDecision, note, at(), notes::Author::Person { name: None })).unwrap()
}

/// Stores an analysis of the session as Analyze would: an outcome and one
/// annotation from the model, in place of the model's earlier annotations.
fn analyze(m: &Machine, id: &str, made_at: i64, note: &str) -> notes::Annotation {
    for old in notes::annotations(&m.conn, &key(id)).unwrap() {
        if matches!(old.author, notes::Author::Model { .. }) && !old.edited {
            notes::delete_annotation(&m.conn, &old.id).unwrap();
        }
    }
    let generated_by = Some(notes::Generation { model: "sonnet".into(), at: made_at, revision: "r1".into() });
    let meta = notes::SessionMeta { key: key(id), outcome: notes::Outcome::Succeeded, outcome_note: Some(note.into()), generated: vec!["outcome".into()], generated_by, ..Default::default() };
    notes::store_meta(&m.conn, &meta).unwrap();
    notes::save_annotation(&m.conn, notes::Annotation::new(&key(id), notes::Kind::WastedWork, note, at(), notes::Author::Model { model: "sonnet".into() })).unwrap()
}

#[test]
fn a_sessions_analysis_and_annotations_arrive_as_they_were_stored() {
    let mut laptop = machine("notes", "laptop", "dev/widgets");
    let mut desktop = machine("notes", "desktop", "code/widgets");
    claude_session(&laptop, "s1", &["Fix the login test"]);
    analyze(&laptop, "s1", 5_000, "Reran the suite three times");
    annotate(&laptop, "s1", "Chose to mock the clock");

    let summary = pull(&mut laptop, &mut desktop);

    assert_eq!(summary.analyses, sync::Counts { added: 1, ..Default::default() });
    assert_eq!(summary.annotations, sync::Counts { added: 2, ..Default::default() });
    assert_eq!(notes::meta(&desktop.conn, &key("s1")).unwrap(), notes::meta(&laptop.conn, &key("s1")).unwrap());
    assert_eq!(notes::annotations(&desktop.conn, &key("s1")).unwrap(), notes::annotations(&laptop.conn, &key("s1")).unwrap());
    let found = index::search(&desktop.conn, "clock").unwrap();
    assert_eq!(found.len(), 1, "a pulled annotation is searchable");
}

/// Writes and saves a report on the session with one recommended action.
fn report_on(m: &Machine, id: &str) -> reports::Report {
    let mut ask = |_: &str, _: &str, _: &serde_json::Value| {
        Ok(generate::ClaudeOutput {
            structured: json!({
                "title": "Login tests", "summary": "s", "takeaways": [], "impact": [],
                "actions": [{ "title": "Document the clock", "category": "instructions", "priority": "high", "change": "Add a Clock section",
                              "success_check": "No more reruns", "refs": ["s1"] }]
            }),
            model: "claude-sonnet-5-5".into(),
        })
    };
    let pick: reports::SessionPick = serde_json::from_value(json!({ "key": key(id), "title": "Login" })).unwrap();
    let report = reports::generate(&m.conn, &[pick], None, &mut ask, &mut |_, _| {}).unwrap();
    reports::save_report(&m.conn, &report).unwrap();
    report
}

fn respond(m: &Machine, report: &reports::Report, status: &str, note: &str) -> reports::ActionResponse {
    let input = json!({ "report_id": report.id, "action_index": 0, "status": status, "note": note, "user_impact": null });
    reports::set_action_response(&m.conn, &serde_json::from_value(input).unwrap()).unwrap().unwrap()
}

#[test]
fn a_report_arrives_with_the_annotation_versions_it_cites_and_the_responses_to_its_actions() {
    let mut laptop = machine("report", "laptop", "dev/widgets");
    let mut desktop = machine("report", "desktop", "code/widgets");
    claude_session(&laptop, "s1", &["Fix the login test"]);
    reindex(&mut laptop);
    let cited = annotate(&laptop, "s1", "Chose to mock the clock");
    let report = report_on(&laptop, "s1");
    let edited = notes::save_annotation(&laptop.conn, notes::Annotation { note: "Chose to freeze the clock".into(), ..cited.clone() }).unwrap();
    respond(&laptop, &report, "done", "Added the section");

    let summary = pull(&mut laptop, &mut desktop);

    assert_eq!(summary.reports_added, 1);
    assert_eq!(summary.responses, sync::Counts { added: 1, ..Default::default() });
    let pulled = reports::report(&desktop.conn, &report.id).unwrap().unwrap();
    assert_eq!(pulled, report);
    let evidence = reports::cited_annotations(&desktop.conn, &pulled).unwrap();
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].annotation, cited, "the report shows the text it was written from");
    assert_eq!(evidence[0].current_version, Some(edited.version));
    assert_eq!(notes::annotations(&desktop.conn, &key("s1")).unwrap(), [edited]);
    assert_eq!(reports::action_responses(&desktop.conn, &report.id).unwrap(), reports::action_responses(&laptop.conn, &report.id).unwrap());
}

#[test]
fn pulling_again_changes_nothing() {
    let mut laptop = machine("again", "laptop", "dev/widgets");
    let mut desktop = machine("again", "desktop", "code/widgets");
    claude_session(&laptop, "s1", &["Fix the login test"]);
    reindex(&mut laptop);
    analyze(&laptop, "s1", 5_000, "Reran the suite three times");
    let report = report_on(&laptop, "s1");
    respond(&laptop, &report, "considering", "Maybe");
    pull(&mut laptop, &mut desktop);
    let file = claude_file(&desktop, "s1");
    let written = std::fs::metadata(&file).unwrap().modified().unwrap();

    let again = pull(&mut laptop, &mut desktop);

    let unchanged = |n| sync::Counts { unchanged: n, ..Default::default() };
    assert_eq!(again.sessions, unchanged(1));
    assert_eq!(again.analyses, unchanged(1));
    assert_eq!(again.annotations, unchanged(1));
    assert_eq!(again.responses, unchanged(1));
    assert_eq!(again.reports_added, 0);
    assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), written, "the file isn't rewritten");
    assert_eq!(listed(&desktop).len(), 1);
}

fn ids(m: &Machine) -> Vec<String> {
    listed(m).into_iter().map(|(id, _)| id).collect()
}

#[test]
fn since_leaves_out_earlier_sessions_and_what_belongs_only_to_them() {
    let mut laptop = machine("since", "laptop", "dev/widgets");
    let mut desktop = machine("since", "desktop", "code/widgets");
    claude_session_on(&laptop, "old", "2026-02-01", &["Fix the login test"]);
    claude_session_on(&laptop, "new", "2026-03-01", &["Fix the logout test"]);
    reindex(&mut laptop);
    analyze(&laptop, "old", 5_000, "Reran the suite three times");
    analyze(&laptop, "new", 6_000, "Went straight to the cause");
    let about_old = report_on(&laptop, "old");
    let about_new = report_on(&laptop, "new");
    let mid_february = chrono::DateTime::parse_from_rfc3339("2026-02-15T00:00:00Z").unwrap().timestamp_millis();

    let bytes = export(&mut laptop, Some(mid_february));
    import(&mut desktop, &bytes).unwrap();

    assert_eq!(ids(&desktop), ["new"]);
    assert_eq!(notes::meta(&desktop.conn, &key("old")).unwrap(), None);
    assert!(notes::meta(&desktop.conn, &key("new")).unwrap().is_some());
    assert!(reports::report(&desktop.conn, &about_old.id).unwrap().is_none());
    assert!(reports::report(&desktop.conn, &about_new.id).unwrap().is_some());
}

/// Continues the session on this machine with one more turn.
fn continue_session(m: &Machine, id: &str, minute: u32, prompt: &str) {
    use std::io::Write;
    let mut file = std::fs::File::options().append(true).open(claude_file(m, id)).unwrap();
    file.write_all(turn(m, id, "2026-03-01", minute, prompt).as_bytes()).unwrap();
}

fn prompts(m: &Machine, id: &str) -> Vec<String> {
    let text = std::fs::read_to_string(claude_file(m, id)).unwrap();
    text.lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|v| v["type"] == "user")
        .map(|v| v["message"]["content"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn the_copy_of_a_session_that_continues_the_other_is_the_one_kept() {
    let mut laptop = machine("longer", "laptop", "dev/widgets");
    let mut desktop = machine("longer", "desktop", "code/widgets");
    claude_session(&laptop, "s1", &["one"]);
    pull(&mut laptop, &mut desktop);

    continue_session(&laptop, "s1", 5, "two");
    let summary = pull(&mut laptop, &mut desktop);
    assert_eq!(summary.sessions, sync::Counts { replaced: 1, ..Default::default() });
    assert_eq!(prompts(&desktop, "s1"), ["one", "two"]);

    // Resumed on the desktop, in the desktop's checkout. The laptop's copy is now the shorter one.
    continue_session(&desktop, "s1", 9, "three");
    let summary = pull(&mut laptop, &mut desktop);
    assert_eq!(summary.sessions, sync::Counts { kept_local: 1, ..Default::default() });
    assert_eq!(prompts(&desktop, "s1"), ["one", "two", "three"]);
    assert_eq!(listed(&desktop)[0].1, desktop.project.to_string_lossy());
}

#[test]
fn when_a_session_was_continued_on_both_machines_the_copy_with_the_later_last_event_is_kept() {
    let mut laptop = machine("diverged", "laptop", "dev/widgets");
    let mut desktop = machine("diverged", "desktop", "code/widgets");
    claude_session(&laptop, "s1", &["one"]);
    pull(&mut laptop, &mut desktop);

    continue_session(&laptop, "s1", 20, "laptop went on");
    continue_session(&desktop, "s1", 10, "desktop went on");
    let summary = pull(&mut laptop, &mut desktop);
    assert_eq!(summary.sessions, sync::Counts { replaced: 1, ..Default::default() });
    assert_eq!(prompts(&desktop, "s1"), ["one", "laptop went on"]);

    continue_session(&laptop, "s1", 30, "laptop again");
    continue_session(&desktop, "s1", 40, "desktop again, later");
    let summary = pull(&mut laptop, &mut desktop);
    assert_eq!(summary.sessions, sync::Counts { kept_local: 1, ..Default::default() });
    assert_eq!(prompts(&desktop, "s1"), ["one", "laptop went on", "desktop again, later"]);
}

/// Later edits get a later `updated_at`, which is in milliseconds.
fn tick() {
    std::thread::sleep(std::time::Duration::from_millis(3));
}

fn edit_note(m: &Machine, a: &notes::Annotation, note: &str) -> notes::Annotation {
    tick();
    notes::save_annotation(&m.conn, notes::Annotation { note: note.into(), ..a.clone() }).unwrap()
}

fn edit_outcome(m: &Machine, id: &str, note: &str) {
    tick();
    let meta = notes::SessionMeta { outcome_note: Some(note.into()), ..notes::meta(&m.conn, &key(id)).unwrap().unwrap() };
    notes::save_meta(&m.conn, &meta).unwrap();
}

fn outcome_note(m: &Machine, id: &str) -> String {
    notes::meta(&m.conn, &key(id)).unwrap().unwrap().outcome_note.unwrap()
}

#[test]
fn a_newer_edit_replaces_an_older_one_and_an_older_one_never_replaces_a_newer_one() {
    let mut laptop = machine("edits", "laptop", "dev/widgets");
    let mut desktop = machine("edits", "desktop", "code/widgets");
    claude_session(&laptop, "s1", &["Fix the login test"]);
    reindex(&mut laptop);
    analyze(&laptop, "s1", 5_000, "Reran the suite three times");
    let note = annotate(&laptop, "s1", "Chose to mock the clock");
    let report = report_on(&laptop, "s1");
    respond(&laptop, &report, "considering", "Maybe");
    pull(&mut laptop, &mut desktop);

    // Edited on the desktop after the pull: the laptop's copies are now the older ones.
    edit_outcome(&desktop, "s1", "Desktop's view");
    let on_desktop = edit_note(&desktop, &note, "Desktop's wording");
    tick();
    respond(&desktop, &report, "done", "Desktop did it");
    let summary = pull(&mut laptop, &mut desktop);
    assert_eq!(summary.analyses.kept_local, 1);
    assert_eq!(summary.annotations.kept_local, 1);
    assert_eq!(summary.responses, sync::Counts { kept_local: 1, ..Default::default() });
    assert_eq!(outcome_note(&desktop, "s1"), "Desktop's view");
    assert_eq!(notes::annotation(&desktop.conn, &note.id).unwrap().unwrap(), on_desktop);
    assert_eq!(reports::action_responses(&desktop.conn, &report.id).unwrap()[0].note.as_deref(), Some("Desktop did it"));

    // Edited on the laptop later still.
    edit_outcome(&laptop, "s1", "Laptop's later view");
    let on_laptop = edit_note(&laptop, &note, "Laptop's later wording");
    tick();
    respond(&laptop, &report, "declined", "Laptop changed its mind");
    let summary = pull(&mut laptop, &mut desktop);
    assert_eq!(summary.analyses.replaced, 1);
    assert_eq!(summary.annotations.replaced, 1);
    assert_eq!(summary.responses, sync::Counts { replaced: 1, ..Default::default() });
    assert_eq!(outcome_note(&desktop, "s1"), "Laptop's later view");
    assert_eq!(notes::annotation(&desktop.conn, &note.id).unwrap().unwrap().note, on_laptop.note);
    assert_eq!(reports::action_responses(&desktop.conn, &report.id).unwrap(), reports::action_responses(&laptop.conn, &report.id).unwrap());
    let evidence = reports::cited_annotations(&desktop.conn, &reports::report(&desktop.conn, &report.id).unwrap().unwrap()).unwrap();
    let cited = evidence.iter().find(|e| e.annotation.id == note.id).unwrap();
    assert_eq!(cited.annotation.note, "Chose to mock the clock", "the report still shows the text it was written from");
}

fn notes_on(m: &Machine, id: &str) -> Vec<String> {
    let mut list: Vec<String> = notes::annotations(&m.conn, &key(id)).unwrap().into_iter().map(|a| a.note).collect();
    list.sort();
    list
}

#[test]
fn the_newer_analysis_of_a_session_replaces_the_older_one_instead_of_adding_to_it() {
    let mut laptop = machine("analysis", "laptop", "dev/widgets");
    let mut desktop = machine("analysis", "desktop", "code/widgets");
    claude_session(&laptop, "s1", &["Fix the login test"]);
    pull(&mut laptop, &mut desktop);

    analyze(&desktop, "s1", 5_000, "Desktop's first analysis");
    annotate(&desktop, "s1", "Written by hand on the desktop");
    let kept = analyze(&desktop, "s1", 5_000, "Desktop's analysis, then edited by hand");
    edit_note(&desktop, &kept, "Edited by hand on the desktop");
    notes::save_annotation(&desktop.conn, notes::Annotation::new(&key("s1"), notes::Kind::WastedWork, "Desktop's first analysis", at(), notes::Author::Model { model: "sonnet".into() })).unwrap();
    tick();
    analyze(&laptop, "s1", 9_000, "Laptop's later analysis");
    pull(&mut laptop, &mut desktop);
    assert_eq!(notes_on(&desktop, "s1"), ["Edited by hand on the desktop", "Laptop's later analysis", "Written by hand on the desktop"]);
    assert_eq!(outcome_note(&desktop, "s1"), "Laptop's later analysis");

    tick();
    analyze(&desktop, "s1", 20_000, "Desktop's latest analysis");
    let summary = pull(&mut laptop, &mut desktop);
    assert_eq!(summary.annotations.added, 0);
    assert_eq!(notes_on(&desktop, "s1"), ["Desktop's latest analysis", "Edited by hand on the desktop", "Written by hand on the desktop"]);
    assert_eq!(outcome_note(&desktop, "s1"), "Desktop's latest analysis");
}

#[test]
fn what_claude_code_keeps_beside_a_session_travels_with_it() {
    let mut laptop = machine("beside", "laptop", "dev/widgets");
    let mut desktop = machine("beside", "desktop", "code/widgets");
    claude_session(&laptop, "s1", &["Fix the login test"]);
    let beside = ["s1/subagents/agent-a1.jsonl", "s1/subagents/agent-a1.meta.json", "s1/tool-results/toolu_1.txt"];
    let contents = [turn(&laptop, "s1", "2026-03-01", 2, "Look through the tests"), r#"{"agentType":"Explore"}"#.to_string(), "A very long test log".to_string()];
    for (rel, text) in beside.iter().zip(&contents) {
        let file = claude_folder(&laptop, &laptop.project).join(rel);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    let summary = pull(&mut laptop, &mut desktop);

    assert_eq!(summary.sessions, sync::Counts { added: 2, ..Default::default() }, "the session and its subagent");
    for (rel, text) in beside.iter().zip(&contents) {
        assert_eq!(std::fs::read_to_string(claude_folder(&desktop, &desktop.project).join(rel)).unwrap(), *text, "{rel}");
    }
    let rows = index::list(&desktop.conn).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.project == desktop.project.to_string_lossy()));
    let subagent = rows.iter().find(|r| r.is_subagent).unwrap();
    assert_eq!(subagent.parent_id.as_deref(), Some("s1"));

    let again = pull(&mut laptop, &mut desktop);
    assert_eq!(again.sessions, sync::Counts { unchanged: 2, ..Default::default() });
}

fn set_origin(project: &Path, url: &str) {
    std::fs::write(project.join(".git/config"), format!("[core]\n\tbare = false\n[remote \"origin\"]\n\turl = {url}\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n[branch \"main\"]\n\tremote = origin\n")).unwrap();
}

#[test]
fn a_repository_is_named_by_its_origin_however_the_url_is_written_or_else_by_its_folder() {
    let m = machine("naming", "one", "Dev/Widgets");
    let name = |url: &str| {
        set_origin(&m.project, url);
        sync::repo_name(&m.project).unwrap().0
    };
    assert_eq!(name("git@github.com:Dan/Widgets.git"), "github.com/dan/widgets");
    assert_eq!(name("https://github.com/dan/widgets"), "github.com/dan/widgets");
    assert_eq!(name("https://dan@github.com/dan/widgets.git/"), "github.com/dan/widgets");
    assert_eq!(name("ssh://git@github.com:22/dan/widgets.git"), "github.com/dan/widgets");
    assert_eq!(name("/srv/git/widgets.git"), "widgets", "an origin on this machine's disk names nothing another machine shares");

    std::fs::remove_file(m.project.join(".git/config")).unwrap();
    assert_eq!(sync::repo_name(&m.project).unwrap().0, "widgets");
    assert_eq!(sync::repo_name(Path::new("/gone/from/disk/Widgets")).unwrap().0, "widgets");
}

#[test]
fn checkouts_in_differently_named_folders_are_one_repository_when_their_origins_match() {
    let mut laptop = machine("origin", "laptop", "dev/app");
    let mut desktop = machine("origin", "desktop", "code/widgets-main");
    set_origin(&laptop.project, "git@github.com:dan/widgets.git");
    set_origin(&desktop.project, "https://github.com/dan/widgets");
    claude_session(&laptop, "s1", &["Fix the login test"]);
    // Another repository on the laptop, in a folder with the desktop checkout's name.
    let other = laptop.project.parent().unwrap().join("widgets-main");
    std::fs::create_dir_all(other.join(".git")).unwrap();
    let elsewhere = Machine { home: laptop.home.clone(), project: other, conn: rusqlite::Connection::open_in_memory().unwrap() };
    claude_session(&elsewhere, "unrelated", &["Something else"]);

    pull(&mut laptop, &mut desktop);

    assert_eq!(ids(&desktop), ["s1"]);
}

fn unpacked(bytes: &[u8]) -> Vec<u8> {
    zstd::stream::decode_all(bytes).unwrap()
}

fn packed(bytes: &[u8]) -> Vec<u8> {
    zstd::stream::encode_all(bytes, 3).unwrap()
}

/// Every file below the machine's home folder.
fn files(m: &Machine) -> Vec<PathBuf> {
    walkdir::WalkDir::new(&m.home).into_iter().filter_map(Result::ok).filter(|e| e.file_type().is_file()).map(|e| e.into_path()).collect()
}

fn refusal(to: &mut Machine, bytes: &[u8]) -> String {
    let error = import(to, bytes).unwrap_err().to_string();
    assert_eq!(files(to), Vec::<PathBuf>::new(), "nothing is written");
    assert_eq!(listed(to), []);
    error
}

#[test]
fn an_import_refuses_what_it_cannot_use_before_writing_anything() {
    let mut laptop = machine("refuse", "laptop", "dev/widgets");
    let mut desktop = machine("refuse", "desktop", "code/widgets");
    claude_session(&laptop, "s1", &["Fix the login test"]);
    let good = unpacked(&export(&mut laptop, None));
    let text = String::from_utf8(good.clone()).unwrap();

    assert!(refusal(&mut desktop, b"hello").contains("isn't a Session Player export"));
    assert!(refusal(&mut desktop, &packed(b"{\"some\":\"json\"}\n")).contains("isn't a Session Player export"));

    let newer = text.replacen("\"version\":1", "\"version\":2", 1);
    assert!(refusal(&mut desktop, &packed(newer.as_bytes())).contains("newer"));

    let mut gadgets = machine("refuse", "gadgets", "code/gadgets");
    let error = refusal(&mut gadgets, &packed(&good));
    assert!(error.contains("widgets") && error.contains("gadgets"), "{error}");

    let escaping = text.replacen("\"rel_path\":\"s1.jsonl\"", "\"rel_path\":\"../../../s1.jsonl\"", 1);
    assert!(refusal(&mut desktop, &packed(escaping.as_bytes())).contains("outside"));
    assert!(!desktop.home.parent().unwrap().join("s1.jsonl").exists());
}

#[test]
fn a_stream_cut_short_stores_no_records_and_pulling_again_finishes_the_job() {
    let mut laptop = machine("short", "laptop", "dev/widgets");
    let mut desktop = machine("short", "desktop", "code/widgets");
    claude_session(&laptop, "s1", &["Fix the login test"]);
    reindex(&mut laptop);
    analyze(&laptop, "s1", 5_000, "Reran the suite three times");
    let report = report_on(&laptop, "s1");
    let whole = unpacked(&export(&mut laptop, None));

    // Cut after the analysis, before the report and the end marker.
    let text = String::from_utf8(whole).unwrap();
    let cut = text.find("{\"record\":\"report\"").unwrap();
    let error = import(&mut desktop, &packed(text[..cut].as_bytes())).unwrap_err().to_string();

    assert!(error.contains("ended early"), "{error}");
    assert_eq!(notes::meta(&desktop.conn, &key("s1")).unwrap(), None);
    assert_eq!(notes_on(&desktop, "s1"), Vec::<String>::new());
    assert!(claude_file(&desktop, "s1").exists(), "a session file copied whole is kept");

    // Cut in the middle of the session file.
    let mut other = machine("short", "other", "code/widgets");
    let middle = text.find("Fix the login test").unwrap();
    let error = import(&mut other, &packed(text[..middle].as_bytes())).unwrap_err().to_string();
    assert!(error.contains("ended early"), "{error}");
    assert_eq!(files(&other), Vec::<PathBuf>::new(), "no partial session file is left");

    let summary = pull(&mut laptop, &mut desktop);
    assert_eq!(summary.sessions, sync::Counts { unchanged: 1, ..Default::default() });
    assert_eq!(summary.analyses.added, 1);
    assert!(reports::report(&desktop.conn, &report.id).unwrap().is_some());
}

/// A fixture session rewritten as if it had been held in the machine's checkout.
fn fixture_in(m: &Machine, name: &str, recorded_cwd: &str) -> String {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap();
    text.replace(recorded_cwd, &m.project.to_string_lossy())
}

fn pi_folder(m: &Machine) -> PathBuf {
    let path = m.project.to_string_lossy();
    m.home.join(".pi/agent/sessions").join(format!("--{}--", path.trim_start_matches('/').replace('/', "-")))
}

#[test]
fn codex_and_pi_sessions_land_where_each_agent_keeps_them() {
    let mut laptop = machine("agents", "laptop", "dev/widgets");
    let mut desktop = machine("agents", "desktop", "code/widgets");
    let codex = "2026/01/02/rollout-2026-01-02T10-00-00-019d-codex.jsonl";
    let pi = "2026-01-03T10-00-00-000Z_01a-pi.jsonl";
    let codex_text = fixture_in(&laptop, "codex.jsonl", "/work/other");
    let pi_text = fixture_in(&laptop, "pi.jsonl", "/work/pi");
    for (file, text) in [(laptop.home.join(".codex/sessions").join(codex), &codex_text), (pi_folder(&laptop).join(pi), &pi_text)] {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    let summary = pull(&mut laptop, &mut desktop);

    assert_eq!(summary.sessions, sync::Counts { added: 2, ..Default::default() });
    assert_eq!(std::fs::read_to_string(desktop.home.join(".codex/sessions").join(codex)).unwrap(), codex_text);
    assert_eq!(std::fs::read_to_string(pi_folder(&desktop).join(pi)).unwrap(), pi_text);
    let here = desktop.project.to_string_lossy().into_owned();
    assert_eq!(listed(&desktop), [("019d-codex".to_string(), here.clone()), ("01a-pi".to_string(), here)]);
}

#[test]
fn a_pull_moves_no_session_but_the_ones_it_brought() {
    let mut laptop = machine("same-id", "laptop", "dev/widgets");
    let mut desktop = machine("same-id", "desktop", "code/widgets");
    claude_session(&laptop, "s1", &["Fix the login test"]);
    // The desktop already has a session with the same id, held in another repository.
    let gadgets = desktop.project.parent().unwrap().join("gadgets");
    std::fs::create_dir_all(gadgets.join(".git")).unwrap();
    let elsewhere = Machine { home: desktop.home.clone(), project: gadgets.clone(), conn: rusqlite::Connection::open_in_memory().unwrap() };
    claude_session(&elsewhere, "s1", &["Something else"]);
    reindex(&mut desktop);

    pull(&mut laptop, &mut desktop);

    let mut projects: Vec<String> = listed(&desktop).into_iter().map(|(_, project)| project).collect();
    projects.sort();
    assert_eq!(projects, [gadgets.to_string_lossy().into_owned(), desktop.project.to_string_lossy().into_owned()]);
}

#[test]
fn a_pulled_session_can_be_pulled_on_from_the_machine_that_pulled_it() {
    let mut laptop = machine("chain", "laptop", "dev/widgets");
    let mut desktop = machine("chain", "desktop", "code/widgets");
    let mut server = machine("chain", "server", "srv/widgets");
    claude_session(&laptop, "s1", &["Fix the login test"]);
    analyze(&laptop, "s1", 5_000, "Reran the suite three times");
    pull(&mut laptop, &mut desktop);

    let summary = pull(&mut desktop, &mut server);

    assert_eq!(summary.sessions, sync::Counts { added: 1, ..Default::default() });
    assert_eq!(listed(&server), [("s1".to_string(), server.project.to_string_lossy().into_owned())]);
    assert_eq!(notes_on(&server, "s1"), ["Reran the suite three times"]);
}

#[test]
fn a_subagent_goes_with_its_session_and_never_without_it() {
    let mut laptop = machine("subagent", "laptop", "dev/widgets");
    let mut desktop = machine("subagent", "desktop", "code/widgets");
    let subagent = |id: &str, day: &str| {
        let file = claude_folder(&laptop, &laptop.project).join(id).join("subagents/agent-a1.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, turn(&laptop, id, day, 2, "Look through the tests")).unwrap();
    };
    // A January session whose subagent ran in March, and a March session whose subagent's clock read January.
    claude_session_on(&laptop, "old", "2026-01-10", &["Fix the login test"]);
    subagent("old", "2026-03-05");
    claude_session_on(&laptop, "new", "2026-03-01", &["Fix the logout test"]);
    subagent("new", "2026-01-05");
    let february = chrono::DateTime::parse_from_rfc3339("2026-02-01T00:00:00Z").unwrap().timestamp_millis();

    let bytes = export(&mut laptop, Some(february));
    import(&mut desktop, &bytes).unwrap();

    let rows = index::list(&desktop.conn).unwrap();
    let mut got: Vec<(String, Option<String>)> = rows.into_iter().map(|r| (r.native_id, r.parent_id)).collect();
    got.sort();
    assert_eq!(got, [("agent-a1".to_string(), Some("new".to_string())), ("new".to_string(), None)]);
}
