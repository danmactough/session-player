mod apps;
#[doc(hidden)]
pub mod bundle;
mod terminals;
#[doc(hidden)]
pub mod index;
#[doc(hidden)]
pub mod model;
mod parse;

#[doc(hidden)]
pub mod test_support {
    pub use crate::bundle;
    pub use crate::index;
    pub use crate::model::SessionRow as Row;

    /// Parses and indexes one file, returning its index row and a compact
    /// "kind:text" rendering of its events ("!" marks errors).
    pub fn index_one(path: &str) -> (Row, Vec<String>) {
        let p = std::path::Path::new(path);
        let source = crate::parse::detect(p).expect("detect");
        let parsed = crate::parse::parse_file(p, source).expect("parse");
        let events = parsed
            .events
            .iter()
            .map(|e| {
                let kind = serde_json::to_value(e.kind).unwrap();
                // "!" marks an error, "~" a failing shell exit.
                let mark = if e.is_error { "!" } else if e.failed_exit { "~" } else { "" };
                format!("{}{}:{}", kind.as_str().unwrap(), mark, e.text)
            })
            .collect();
        let (row, _) = crate::index::index_parsed(path, source, parsed).expect("row");
        (row, events)
    }

    /// Whether the file becomes a row in the session list.
    pub fn is_listed(path: &str) -> bool {
        let p = std::path::Path::new(path);
        let source = crate::parse::detect(p).expect("detect");
        let parsed = crate::parse::parse_file(p, source).expect("parse");
        crate::index::index_parsed(path, source, parsed).is_some()
    }

    /// Renders one pi branch as "kind:text" strings.
    pub fn load_branch(path: &str, leaf: &str) -> Vec<String> {
        crate::parse::pi::parse_branch(std::path::Path::new(path), Some(leaf))
            .expect("parse")
            .events
            .iter()
            .map(|e| format!("{}:{}", serde_json::to_value(e.kind).unwrap().as_str().unwrap(), e.text))
            .collect()
    }
}

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::{DebouncedEvent, Debouncer, new_debouncer};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use model::{Event, SessionRow};

struct AppState {
    db_path: PathBuf,
    /// Images from the most recently opened session, served over spimg:// so
    /// large screenshots stay out of the IPC payload. Keyed by a generation
    /// number that changes with every load.
    images: Mutex<(u64, Vec<Vec<String>>)>,
    running: AtomicBool,
    pending: AtomicBool,
    pending_full: AtomicBool,
    watcher: Mutex<Option<Debouncer<notify::RecommendedWatcher>>>,
    /// The bundle shown in the export sheet. Saving writes this exact bundle,
    /// so what the user reviewed is what leaves the machine.
    export: Mutex<Option<(u64, bundle::Bundle)>>,
}

type Shared = Arc<AppState>;

/// Where imported bundles live: next to the index, inside the app's own data.
fn library_dir(state: &AppState) -> PathBuf {
    state.db_path.with_file_name("library")
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize, Clone)]
struct Progress {
    done: usize,
    total: usize,
}

/// Runs an index pass on a background thread. Requests that arrive while a
/// pass is running are coalesced into one follow-up pass.
fn request_index(app: &AppHandle, state: &Shared, full: bool) {
    if full {
        state.pending_full.store(true, Ordering::SeqCst);
    }
    state.pending.store(true, Ordering::SeqCst);
    if state.running.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return;
    }
    let app = app.clone();
    let state = state.clone();
    std::thread::spawn(move || {
        loop {
            state.pending.store(false, Ordering::SeqCst);
            let full = state.pending_full.swap(false, Ordering::SeqCst);
            let _ = app.emit("index-started", ());
            let library = library_dir(&state);
            let result = index::open(&state.db_path).and_then(|mut conn| {
                let app2 = app.clone();
                index::run(&mut conn, full, Some(&library), &move |done, total| {
                    let _ = app2.emit("index-progress", Progress { done, total });
                })
            });
            match result {
                Ok(stats) => {
                    let _ = app.emit("index-done", stats);
                }
                Err(e) => {
                    let _ = app.emit("index-error", e.to_string());
                }
            }
            if !state.pending.load(Ordering::SeqCst) {
                state.running.store(false, Ordering::SeqCst);
                // A request may have landed between the check and the store.
                if state.pending.load(Ordering::SeqCst)
                    && state
                        .running
                        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                {
                    continue;
                }
                break;
            }
        }
    });
}

fn restart_watcher(app: &AppHandle, state: &Shared) -> anyhow::Result<()> {
    let conn = index::open(&state.db_path)?;
    let roots = index::roots(&conn)?;
    let app2 = app.clone();
    let state2 = state.clone();
    let mut debouncer = new_debouncer(
        Duration::from_millis(1500),
        move |res: Result<Vec<DebouncedEvent>, notify::Error>| {
            if let Ok(events) = res
                && events
                    .iter()
                    .any(|e| e.path.extension().and_then(|x| x.to_str()) == Some("jsonl"))
            {
                request_index(&app2, &state2, false);
            }
        },
    )?;
    for root in roots {
        let p = Path::new(&root);
        if p.is_dir() {
            let _ = debouncer.watcher().watch(p, RecursiveMode::Recursive);
        }
    }
    *state.watcher.lock().unwrap() = Some(debouncer);
    Ok(())
}

#[derive(Serialize)]
struct RootInfo {
    path: String,
    exists: bool,
    sessions: i64,
}

#[tauri::command]
fn get_roots(state: State<Shared>) -> Result<Vec<RootInfo>, String> {
    let conn = index::open(&state.db_path).map_err(err)?;
    let counts = index::root_counts(&conn).map_err(err)?;
    Ok(index::roots(&conn)
        .map_err(err)?
        .into_iter()
        .map(|path| RootInfo {
            exists: Path::new(&path).is_dir(),
            sessions: counts.get(&path).copied().unwrap_or(0),
            path,
        })
        .collect())
}

#[tauri::command]
fn suggested_roots(state: State<Shared>) -> Result<Vec<String>, String> {
    let conn = index::open(&state.db_path).map_err(err)?;
    let existing = index::roots(&conn).map_err(err)?;
    Ok(index::default_roots()
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|p| !existing.contains(p))
        .collect())
}

#[tauri::command]
fn add_root(app: AppHandle, state: State<Shared>, path: String) -> Result<(), String> {
    let conn = index::open(&state.db_path).map_err(err)?;
    index::add_root(&conn, &path).map_err(err)?;
    restart_watcher(&app, &state).map_err(err)?;
    request_index(&app, &state, false);
    Ok(())
}

#[tauri::command]
fn remove_root(app: AppHandle, state: State<Shared>, path: String) -> Result<(), String> {
    let mut conn = index::open(&state.db_path).map_err(err)?;
    index::remove_root(&mut conn, &path).map_err(err)?;
    restart_watcher(&app, &state).map_err(err)?;
    let _ = app.emit("index-done", index::IndexStats::default());
    Ok(())
}

#[tauri::command]
fn reindex(app: AppHandle, state: State<Shared>, full: bool) {
    request_index(&app, &state, full);
}

#[tauri::command]
fn list_sessions(state: State<Shared>) -> Result<Vec<SessionRow>, String> {
    let conn = index::open(&state.db_path).map_err(err)?;
    index::list(&conn).map_err(err)
}

#[tauri::command]
fn search(state: State<Shared>, query: String) -> Result<Vec<index::SearchHit>, String> {
    let conn = index::open(&state.db_path).map_err(err)?;
    index::search(&conn, &query).map_err(err)
}

#[tauri::command]
async fn load_session(state: State<'_, Shared>, path: String, branch: Option<String>) -> Result<Vec<Event>, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let p = PathBuf::from(&path);
        let mut events = if bundle::is_bundle_path(&p) {
            bundle::load_events(&p).map_err(err)?
        } else {
            let source = parse::detect(&p).ok_or_else(|| "Unrecognized session format".to_string())?;
            let parsed = match (&branch, source) {
                (Some(leaf), model::Source::Pi) => parse::pi::parse_branch(&p, Some(leaf)),
                _ => parse::parse_file(&p, source),
            };
            parsed.map_err(err)?.events
        };
        let mut cache = state.images.lock().unwrap();
        let generation = cache.0 + 1;
        let mut images = Vec::with_capacity(events.len());
        for (i, e) in events.iter_mut().enumerate() {
            let mut kept = Vec::new();
            for (j, img) in e.images.iter_mut().enumerate() {
                if img.starts_with("data:") {
                    kept.push(std::mem::replace(img, format!("spimg://localhost/{generation}/{i}/{j}")));
                } else {
                    kept.push(String::new());
                }
            }
            images.push(kept);
        }
        *cache = (generation, images);
        Ok(events)
    })
    .await
    .map_err(err)?
}

#[derive(Serialize)]
struct ExportPreview {
    /// Pass back to `export_save` to write the bundle previewed here.
    token: u64,
    summary: bundle::Summary,
    subagents: Vec<SubagentChoice>,
}

#[derive(Serialize)]
struct SubagentChoice {
    path: String,
    title: String,
    included: bool,
}

/// Builds the bundle for a session and describes it for the export sheet.
/// `subagents` lists the subagent paths to include (None: all of them).
#[tauri::command]
async fn export_preview(
    app: AppHandle,
    state: State<'_, Shared>,
    path: String,
    branch: Option<String>,
    subagents: Option<Vec<String>>,
) -> Result<ExportPreview, String> {
    let state = state.inner().clone();
    let exporter = format!("Session Player {}", app.package_info().version);
    tauri::async_runtime::spawn_blocking(move || {
        let conn = index::open(&state.db_path).map_err(err)?;
        let choices = if branch.is_some() { Vec::new() } else { bundle::subagents_of(&conn, &path).map_err(err)? };
        let request = bundle::ExportRequest { path: &path, branch: branch.as_deref(), subagents: subagents.clone() };
        let built = bundle::build(&conn, request, &exporter).map_err(|e| format!("{e:#}"))?;
        let summary = bundle::summarize(&built);
        let mut pending = state.export.lock().unwrap();
        let token = pending.as_ref().map_or(1, |(t, _)| t + 1);
        *pending = Some((token, built));
        Ok(ExportPreview {
            token,
            summary,
            subagents: choices
                .into_iter()
                .map(|(path, title)| SubagentChoice {
                    included: subagents.as_ref().is_none_or(|wanted| wanted.contains(&path)),
                    path,
                    title,
                })
                .collect(),
        })
    })
    .await
    .map_err(err)?
}

/// Writes the bundle from the preview identified by `token` to `dest`.
#[tauri::command]
async fn export_save(state: State<'_, Shared>, token: u64, dest: String) -> Result<String, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let pending = state.export.lock().unwrap().take();
        let built = match pending {
            Some((t, b)) if t == token => b,
            other => {
                *state.export.lock().unwrap() = other;
                return Err("This preview is out of date. Review the export again before saving.".to_string());
            }
        };
        let mut dest = PathBuf::from(dest);
        if !bundle::is_bundle_path(&dest) {
            dest.as_mut_os_string().push(format!(".{}", bundle::EXTENSION));
        }
        bundle::write(&built, &dest).map_err(|e| format!("{e:#}"))?;
        Ok(dest.to_string_lossy().into_owned())
    })
    .await
    .map_err(err)?
}

/// Drops the previewed bundle when the export sheet closes without saving.
#[tauri::command]
fn export_cancel(state: State<Shared>) {
    *state.export.lock().unwrap() = None;
}

/// Imports a bundle file into the library and re-indexes. With conflicts
/// and no `on_conflict`, nothing is written and the conflicts are returned.
#[tauri::command]
async fn import_bundle(
    app: AppHandle,
    state: State<'_, Shared>,
    path: String,
    on_conflict: Option<bundle::OnConflict>,
) -> Result<bundle::ImportReport, String> {
    let state = state.inner().clone();
    let library = library_dir(&state);
    let report = tauri::async_runtime::spawn_blocking(move || bundle::import(&library, Path::new(&path), on_conflict))
        .await
        .map_err(err)?
        .map_err(|e| format!("{e:#}"))?;
    if !report.imported.is_empty() {
        request_index(&app, &state, false);
    }
    Ok(report)
}

fn serve_image(state: &Shared, uri: &str) -> tauri::http::Response<Vec<u8>> {
    use base64::Engine;
    let not_found = || tauri::http::Response::builder().status(404).body(Vec::new()).unwrap();
    let parts: Vec<u64> = uri
        .split(['/', '?'])
        .filter_map(|s| s.parse().ok())
        .collect();
    let [generation, i, j] = parts[..] else { return not_found() };
    let cache = state.images.lock().unwrap();
    if cache.0 != generation {
        return not_found();
    }
    let Some(data_url) = cache.1.get(i as usize).and_then(|v| v.get(j as usize)) else {
        return not_found();
    };
    let Some((head, b64)) = data_url.split_once(";base64,") else { return not_found() };
    let mime = head.trim_start_matches("data:").to_string();
    match base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
        Ok(bytes) => tauri::http::Response::builder()
            .header("Content-Type", mime)
            .header("Cache-Control", "max-age=3600")
            .body(bytes)
            .unwrap(),
        Err(_) => not_found(),
    }
}

/// The desktop apps that can open this session, with the link for each.
#[tauri::command]
async fn open_targets(source: String, native_id: String, path: String) -> Vec<apps::OpenTarget> {
    apps::targets(&source, &native_id, &path)
}

/// Opens one of the links from `open_targets` in its app.
#[tauri::command]
fn open_in_app(url: String) -> Result<(), String> {
    let scheme = url.split(':').next().unwrap_or("");
    if !apps::SCHEMES.contains(&scheme) {
        return Err(format!("not an app link: {url}"));
    }
    tauri_plugin_opener::open_url(&url, None::<&str>).map_err(err)
}

/// The terminal apps installed, for "Open in Terminal".
#[tauri::command]
fn terminal_apps() -> Vec<terminals::TerminalApp> {
    terminals::installed()
}

/// Runs the session's resume command in a new window of a terminal app.
#[tauri::command]
async fn open_in_terminal(app: String, source: String, native_id: String, path: String, cwd: Option<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let session = terminals::Session { source: &source, native_id: &native_id, path: &path, cwd: cwd.as_deref() };
        terminals::open(&app, &session)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
fn reveal(path: String) -> Result<(), String> {
    std::process::Command::new("open").arg("-R").arg(&path).spawn().map_err(err)?;
    Ok(())
}

/// Lets the webview report errors to the terminal running the app.
#[tauri::command]
fn log_frontend(level: String, message: String) {
    eprintln!("[webview:{level}] {message}");
}

#[tauri::command]
fn home_dir() -> String {
    dirs::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default()
}

#[tauri::command]
fn get_setting(state: State<Shared>, key: String) -> Result<Option<String>, String> {
    let conn = index::open(&state.db_path).map_err(err)?;
    conn.query_row("SELECT value FROM settings WHERE key=?1", [format!("ui.{key}")], |r| r.get(0))
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(err(e)),
        })
}

#[tauri::command]
fn set_setting(state: State<Shared>, key: String, value: String) -> Result<(), String> {
    let conn = index::open(&state.db_path).map_err(err)?;
    conn.execute(
        "INSERT OR REPLACE INTO settings(key, value) VALUES(?1, ?2)",
        [format!("ui.{key}"), value],
    )
    .map_err(err)?;
    Ok(())
}

/// Sends any navigation away from the app's own pages (a link in a
/// transcript, however it was clicked) to the default browser instead of
/// replacing the app in its window.
fn external_links<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("external-links")
        .on_navigation(|_, url| {
            let own = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "tauri.localhost"));
            let external = matches!(url.scheme(), "http" | "https" | "mailto") && !own;
            if external {
                let _ = tauri_plugin_opener::open_url(url.as_str(), None::<&str>);
            }
            !external
        })
        .build()
}

/// The standard menu with "Settings…" (⌘,) added to the app menu, after
/// "About", where macOS apps put it, and "Import Session…" (⌘⇧I) at the top
/// of the File menu.
fn install_menu(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    let menu = Menu::default(app)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
    let items = menu.items()?;
    if let Some(app_menu) = items.first().and_then(|m| m.as_submenu()) {
        app_menu.insert_items(&[&settings, &PredefinedMenuItem::separator(app)?], 2)?;
    }
    if let Some(file_menu) = items.iter().filter_map(|m| m.as_submenu()).find(|m| m.text().is_ok_and(|t| t == "File")) {
        let import = MenuItem::with_id(app, "import-session", "Import Session…", true, Some("CmdOrCtrl+Shift+I"))?;
        file_menu.insert_items(&[&import, &PredefinedMenuItem::separator(app)?], 0)?;
    }
    app.set_menu(menu)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(external_links())
        .register_uri_scheme_protocol("spimg", |ctx, req| {
            let state = ctx.app_handle().state::<Shared>();
            serve_image(state.inner(), &req.uri().to_string())
        })
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db_path = dir.join("index.sqlite");
            let conn = index::open(&db_path)?;
            index::seed_roots(&conn)?;
            drop(conn);
            let state: Shared = Arc::new(AppState {
                db_path,
                images: Mutex::new((0, Vec::new())),
                running: AtomicBool::new(false),
                pending: AtomicBool::new(false),
                pending_full: AtomicBool::new(false),
                watcher: Mutex::new(None),
                export: Mutex::new(None),
            });
            app.manage(state.clone());
            let handle = app.handle().clone();
            if let Err(e) = restart_watcher(&handle, &state) {
                eprintln!("watcher failed: {e}");
            }
            request_index(&handle, &state, false);
            install_menu(app.handle())?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            match event.id().as_ref() {
                "settings" => {
                    let _ = app.emit("open-settings", ());
                }
                "import-session" => {
                    let _ = app.emit("import-session", ());
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_roots,
            suggested_roots,
            add_root,
            remove_root,
            reindex,
            list_sessions,
            search,
            load_session,
            reveal,
            open_targets,
            open_in_app,
            terminal_apps,
            open_in_terminal,
            home_dir,
            log_frontend,
            get_setting,
            set_setting,
            export_preview,
            export_save,
            export_cancel,
            import_bundle
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_cached_image_for_current_generation_only() {
        let state: Shared = Arc::new(AppState {
            db_path: PathBuf::new(),
            images: Mutex::new((7, vec![vec![], vec!["data:image/png;base64,aGVsbG8=".into()]])),
            running: AtomicBool::new(false),
            pending: AtomicBool::new(false),
            pending_full: AtomicBool::new(false),
            watcher: Mutex::new(None),
            export: Mutex::new(None),
        });
        let ok = serve_image(&state, "spimg://localhost/7/1/0");
        assert_eq!(ok.status(), 200);
        assert_eq!(ok.headers()["Content-Type"], "image/png");
        assert_eq!(ok.body(), b"hello");
        assert_eq!(serve_image(&state, "spimg://localhost/6/1/0").status(), 404);
        assert_eq!(serve_image(&state, "spimg://localhost/7/0/0").status(), 404);
        assert_eq!(serve_image(&state, "http://spimg.localhost/7/1/0").status(), 200);
    }
}
