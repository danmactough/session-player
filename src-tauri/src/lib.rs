mod apps;
mod cli;
pub mod local;
pub mod paired;
pub mod relay;
pub mod save;
mod terminals;

// The app's data lives on the server (`sp serve`, on this machine or a paired one). The page talks
// to the local server directly over loopback HTTP, and to a paired remote one through the relay
// here, which holds that server's token. What stays in the shell is native: Finder, terminals,
// other apps, dialogs, the updater, and managing the local server as a service.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use session_player_client::ServersStore;
use session_player_core::ops::ServerEvent;
use session_player_core::{bundle, runlog};
use tauri::ipc::{Channel, InvokeBody, Request, Response};
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    /// Where the server keeps its data (`server.lock`, `server.token`) and this app keeps
    /// `servers.json`: the same folder `sp` uses.
    data_dir: PathBuf,
    /// The log of Analyze and report runs; see `runlog`.
    log_path: PathBuf,
    /// The `sp` inside the app, which manages the local server as a service. None in a build
    /// without one (`npm run app`).
    sp: Option<PathBuf>,
    /// One Client per paired server, which relayed calls share.
    clients: relay::Clients,
    /// Open relayed event streams.
    streams: relay::Streams,
}

type Shared = Arc<AppState>;

/// How long install and start wait for the server to take its lock.
const START_WAIT: Duration = Duration::from_secs(15);

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn store(state: &AppState) -> ServersStore {
    ServersStore::new(&state.data_dir)
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(err)?
}

/// A call to a paired server's operation. The body is the request's raw body (JSON or an upload);
/// the headers name the server (`server`, percent-encoded), the operation (`op`), its query
/// (`query`, URL-encoded) and `content-type`. Returns the framed response (see `relay`).
#[tauri::command]
async fn relay_request(state: State<'_, Shared>, request: Request<'_>) -> Result<Response, String> {
    let headers: Vec<(String, String)> = request.headers().iter().filter_map(|(n, v)| Some((n.as_str().to_string(), v.to_str().ok()?.to_string()))).collect();
    let relay::RequestHeaders { server, op, query, content_type } = relay::request_headers(&headers)?;
    let body = match request.body() {
        InvokeBody::Raw(bytes) => bytes.clone(),
        InvokeBody::Json(serde_json::Value::Null) => Vec::new(),
        InvokeBody::Json(value) => serde_json::to_vec(value).map_err(err)?,
    };
    let paired = store(&state).list().map_err(|e| format!("{e:#}"))?;
    let shared = state.inner().clone();
    let framed = blocking(move || relay::relay_call(relay::RelayCall { clients: &shared.clients, paired: &paired, server: &server, op: &op, query: &query, content_type: &content_type, body })).await?;
    Ok(Response::new(framed))
}

struct SaveDialog {
    app: AppHandle,
    title: &'static str,
    /// The filter's label, e.g. "Markdown".
    kind: &'static str,
    /// Without the dot; the saved file always ends in it.
    extension: &'static str,
    /// The page's suggested file name; only its last part is used.
    name: String,
    contents: Vec<u8>,
}

/// Shows the save dialog and writes `contents` where the user chose. The page never names the
/// path, so it can't write anywhere the user didn't pick. None when the user cancels.
async fn save_with_dialog(save: SaveDialog) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    // The blocking dialog waits on the main thread, so it must not be shown from it.
    blocking(move || {
        let chosen = save
            .app
            .dialog()
            .file()
            .set_title(save.title)
            .set_file_name(save::suggested_name(&save.name, save.extension))
            .add_filter(save.kind, &[save.extension])
            .blocking_save_file();
        let Some(chosen) = chosen else { return Ok(None) };
        let path = save::force_extension(chosen.into_path().map_err(err)?, save.extension);
        std::fs::write(&path, &save.contents).map_err(err)?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
}

/// Saves a report's Markdown through the save dialog, as `.md`. `name` is the suggested file name.
/// Returns the path written, or None when the user cancels.
#[tauri::command]
async fn save_markdown(app: AppHandle, name: String, text: String) -> Result<Option<String>, String> {
    save_with_dialog(SaveDialog { app, title: "Save report", kind: "Markdown", extension: "md", name, contents: text.into_bytes() }).await
}

/// Saves a downloaded export through the save dialog, as a session bundle. The raw body is the
/// file; the `name` header is the suggested file name, percent-encoded. Returns the path written,
/// or None when the user cancels.
#[tauri::command]
async fn save_file(app: AppHandle, request: Request<'_>) -> Result<Option<String>, String> {
    let name = request.headers().get("name").and_then(|v| v.to_str().ok()).ok_or("The save needs a name header.")?;
    let name = relay::percent_decode(name)?;
    let InvokeBody::Raw(bytes) = request.body() else { return Err("The save needs a raw body.".to_string()) };
    save_with_dialog(SaveDialog { app, title: "Export session", kind: "Session bundle", extension: bundle::EXTENSION, name, contents: bytes.clone() }).await
}

/// An image URL from a paired server's `load_session`, framed like `relay_request`.
#[tauri::command]
async fn relay_image(state: State<'_, Shared>, server: String, path: String) -> Result<Response, String> {
    let paired = store(&state).list().map_err(|e| format!("{e:#}"))?;
    let shared = state.inner().clone();
    let framed = blocking(move || relay::relay_image(relay::RelayImage { clients: &shared.clients, paired: &paired, server: &server, path: &path })).await?;
    Ok(Response::new(framed))
}

/// Sends a paired server's events to `on_event`, starting after `last_id` (None: from now), and
/// reconnects from the last event sent when the connection breaks. Returns the stream's id for
/// `relay_close_stream`.
#[tauri::command]
fn relay_open_stream(state: State<'_, Shared>, server: String, last_id: Option<u64>, on_event: Channel<ServerEvent>) -> Result<u64, String> {
    let paired = store(&state).list().map_err(|e| format!("{e:#}"))?;
    let server = relay::resolve_server(&server, &paired)?;
    let (id, stop) = state.streams.open();
    let shared = state.inner().clone();
    std::thread::spawn(move || {
        // A failed send means the page's channel is gone; returning false ends the stream.
        let mut send = |event: ServerEvent| on_event.send(event).is_ok();
        relay::run_stream(relay::StreamParams { server, last_id, stop: &stop, retry: Duration::from_secs(2), on_event: &mut send });
        shared.streams.ended(id);
    });
    Ok(id)
}

#[tauri::command]
fn relay_close_stream(state: State<'_, Shared>, id: u64) {
    state.streams.close(id);
}

#[tauri::command]
async fn local_server_state(state: State<'_, Shared>) -> Result<local::LocalServerState, String> {
    let shared = state.inner().clone();
    blocking(move || Ok(local::server_state(local::StateParams { data: &shared.data_dir, sp: shared.sp.as_deref() }))).await
}

fn bundled_sp(state: &AppState) -> Result<PathBuf, String> {
    state.sp.clone().ok_or_else(|| "This build of Session Player has no sp command inside it, so it can't install the server. Releases do; run `sp service install` instead.".to_string())
}

/// Installs the server as a per-user service (LaunchAgent or systemd user unit) and waits for it.
#[tauri::command]
async fn local_server_install(state: State<'_, Shared>) -> Result<local::LocalServerState, String> {
    let shared = state.inner().clone();
    let sp = bundled_sp(&shared)?;
    blocking(move || local::install(local::StartParams { data: &shared.data_dir, sp: &sp, wait: START_WAIT })).await
}

/// Starts the installed, stopped service and waits for it.
#[tauri::command]
async fn local_server_start(state: State<'_, Shared>) -> Result<local::LocalServerState, String> {
    let shared = state.inner().clone();
    let sp = bundled_sp(&shared)?;
    blocking(move || local::start(local::StartParams { data: &shared.data_dir, sp: &sp, wait: START_WAIT })).await
}

/// `{ url, token }` of the running local server.
#[tauri::command]
async fn local_server_connection(state: State<'_, Shared>) -> Result<local::Connection, String> {
    let shared = state.inner().clone();
    blocking(move || local::local_connection(&session_player_client::probe_local(&shared.data_dir))).await
}

#[tauri::command]
fn paired_servers_list(state: State<'_, Shared>) -> Result<Vec<paired::ServerRef>, String> {
    paired::list(&store(&state))
}

/// Redeems a pairing string from `sp pair` and saves the server as `name` (its address when None).
#[tauri::command]
async fn paired_server_connect(state: State<'_, Shared>, pairing: String, name: Option<String>) -> Result<paired::ServerRef, String> {
    let shared = state.inner().clone();
    blocking(move || {
        let device_name = hostname();
        paired::connect(paired::ConnectParams { store: &store(&shared), clients: &shared.clients, pairing: &pairing, name: name.as_deref(), device_name: &device_name })
    })
    .await
}

#[tauri::command]
fn paired_server_remove(state: State<'_, Shared>, name: String) -> Result<(), String> {
    paired::remove(paired::RemoveParams { store: &store(&state), clients: &state.clients, name: &name })
}

/// What the server lists this machine as.
fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Session Player app".to_string())
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

/// Opens the log of Analyze and report runs in the app the Mac uses for
/// logs (Console, unless changed).
#[tauri::command]
fn open_analyze_log(state: State<Shared>) -> Result<(), String> {
    let path = &state.log_path;
    if !path.exists() {
        runlog::append(path, "Log started");
    }
    std::process::Command::new("open").arg(path).spawn().map_err(err)?;
    Ok(())
}

#[tauri::command]
fn reveal(path: String) -> Result<(), String> {
    std::process::Command::new("open").arg("-R").arg(&path).spawn().map_err(err)?;
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

/// What File > Install Command Line Tool does: links ~/.local/bin/sp to the sp
/// inside the app, then says what happened.
fn install_cli(app: &AppHandle) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
    let result = match cli::bundled() {
        None => Err("This build of Session Player has no sp command inside it. Releases do.".to_string()),
        Some(sp) => cli::link(&sp, &dirs::home_dir().unwrap_or_default()),
    };
    let (kind, message) = match result {
        Ok(dest) => (
            MessageDialogKind::Info,
            // The app can't see the PATH a terminal sets up, so it can't tell whether this folder is on it.
            format!("The sp command is at {}.\n\nIf your terminal doesn't find sp, add ~/.local/bin to your PATH. Run \"sp --help\" to see what it does.", dest.display()),
        ),
        Err(e) => (MessageDialogKind::Error, e),
    };
    app.dialog().message(message).title("Install Command Line Tool").kind(kind).show(|_| {});
}

/// The standard menu with "Settings…" (⌘,) added to the app menu, after
/// "About", where macOS apps put it, and "Import Session…" (⌘⇧I) and
/// "Install Command Line Tool" at the top of the File menu.
fn install_menu(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    let menu = Menu::default(app)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
    let check_updates = MenuItem::with_id(app, "check-for-updates", "Check for Updates…", true, None::<&str>)?;
    let items = menu.items()?;
    if let Some(app_menu) = items.first().and_then(|m| m.as_submenu()) {
        app_menu.insert_items(&[&check_updates, &settings, &PredefinedMenuItem::separator(app)?], 2)?;
    }
    if let Some(file_menu) = items.iter().filter_map(|m| m.as_submenu()).find(|m| m.text().is_ok_and(|t| t == "File")) {
        let import = MenuItem::with_id(app, "import-session", "Import Session…", true, Some("CmdOrCtrl+Shift+I"))?;
        let install_cli = MenuItem::with_id(app, "install-cli", "Install Command Line Tool", true, None::<&str>)?;
        file_menu.insert_items(&[&import, &PredefinedMenuItem::separator(app)?, &install_cli, &PredefinedMenuItem::separator(app)?], 0)?;
    }
    app.set_menu(menu)?;
    Ok(())
}

/// The folders `sp` uses: Tauri's `app_data_dir` and `app_log_dir`, unless
/// `SESSION_PLAYER_DATA_DIR` points both elsewhere.
fn folders(app: &tauri::App) -> tauri::Result<(PathBuf, PathBuf)> {
    match std::env::var_os("SESSION_PLAYER_DATA_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => Ok((PathBuf::from(&dir), PathBuf::from(&dir).join("logs"))),
        None => Ok((app.path().app_data_dir()?, app.path().app_log_dir()?)),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(external_links())
        .setup(|app| {
            let (data_dir, logs) = folders(app)?;
            std::fs::create_dir_all(&data_dir)?;
            let sp = cli::bundled();
            // After an update the service may still name the sp of an older app; reinstalling
            // points it at this one.
            if let Some(sp) = sp.clone() {
                std::thread::spawn(move || {
                    if let Err(e) = local::refresh_after_update(&sp) {
                        eprintln!("Couldn't refresh the server's service: {e}");
                    }
                });
            }
            app.manage::<Shared>(Arc::new(AppState {
                data_dir,
                log_path: logs.join("analyze.log"),
                sp,
                clients: relay::Clients::default(),
                streams: relay::Streams::default(),
            }));
            install_menu(app.handle())?;
            Ok(())
        })
        // A reload (or any new page) can't close the streams the old page opened, and their
        // channels may still accept sends, so close them all when the main webview starts loading.
        .on_page_load(|webview, payload| {
            if webview.label() == "main"
                && payload.event() == PageLoadEvent::Started
                && let Some(state) = webview.try_state::<Shared>()
            {
                state.streams.close_all();
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "settings" => {
                let _ = app.emit("open-settings", ());
            }
            "check-for-updates" => {
                let _ = app.emit("check-for-updates", ());
            }
            "import-session" => {
                let _ = app.emit("import-session", ());
            }
            "install-cli" => install_cli(app),
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            relay_request,
            relay_image,
            relay_open_stream,
            relay_close_stream,
            local_server_state,
            local_server_install,
            local_server_start,
            local_server_connection,
            paired_servers_list,
            paired_server_connect,
            paired_server_remove,
            reveal,
            open_targets,
            open_in_app,
            terminal_apps,
            open_in_terminal,
            open_analyze_log,
            save_markdown,
            save_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
