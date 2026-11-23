// `sp --server <name> analyze` runs Analyze as a job on the paired server, with the server's claude.

#[path = "../../testutil/tempdir.rs"]
mod tempdir;

mod common;

use common::{Machine, ok, server_logs, stderr, stdout};
use tempdir::TempDir;

#[test]
fn sp_analyze_with_a_paired_server_analyzes_the_servers_sessions_with_the_servers_claude() {
    let dir = TempDir::new("cli-remote-analyze");
    let alpha = Machine::new(&dir, "alpha");
    let beta = Machine::new(&dir, "beta");
    let alpha_claude = alpha.fake_claude();
    let beta_claude = beta.fake_claude();
    let alpha_widgets = alpha.checkout("widgets");
    let beta_widgets = beta.checkout("widgets");
    alpha.session(&alpha_widgets, "aaaa1111-server-widgets", "2026-03-10");
    alpha.session(&alpha_widgets, "aaaa2222-server-old", "2026-01-05");
    beta.session(&beta_widgets, "bbbb2222-local-widgets", "2026-03-12");
    ok(&alpha.sp(&["index"]), "sp index on alpha");
    ok(&beta.sp(&["index"]), "sp index on beta");
    let _server = alpha.serve_external();
    common::pair(&alpha, &beta, "lab");

    let output = beta.sp(&["--server", "lab", "analyze", "--project", beta_widgets.to_str().unwrap(), "--since", "2026-03-01"]);
    assert_eq!(output.status.code(), Some(0), "stdout: {}\nstderr: {}\n{}", stdout(&output), stderr(&output), server_logs(&alpha.root));
    assert_eq!(alpha_claude.models(), vec!["haiku"], "the server analyzed its one session in range\n{}", server_logs(&alpha.root));
    assert!(beta_claude.models().is_empty(), "beta's claude is never called");

    let after = ok(&beta.sp(&["--server", "lab", "sessions", "--project", beta_widgets.to_str().unwrap()]), "sp --server lab sessions");
    let line = |id: &str| after.lines().find(|l| l.starts_with(id)).unwrap_or_else(|| panic!("{id} isn't listed: {after}")).to_string();
    assert!(line("aaaa1111").contains("analyzed"), "{after}");
    assert!(!line("aaaa2222").contains("analyzed"), "a session before --since isn't analyzed: {after}");
    let local = ok(&beta.sp(&["sessions", "--project", beta_widgets.to_str().unwrap()]), "sp sessions on beta");
    assert!(!local.contains("analyzed"), "nothing is analyzed on beta: {local}");
}

/// `sp analyze`'s standard error after it analyzes two sessions, one the fake claude answers and
/// one it fails: in process with `server` None, else on alpha's server paired as `server`.
fn analyze_two(dir: &TempDir, server: Option<&str>) -> String {
    let alpha = Machine::new(dir, "alpha");
    let beta = Machine::new(dir, "beta");
    let claude = alpha.fake_claude();
    claude.fail_on("Prompt of aaaa2222-broken");
    let widgets = alpha.checkout("widgets");
    alpha.session(&widgets, "aaaa1111-fine", "2026-03-10");
    alpha.session(&widgets, "aaaa2222-broken", "2026-03-11");
    ok(&alpha.sp(&["index"]), "sp index on alpha");
    let project = widgets.to_str().unwrap();
    let output = match server {
        None => alpha.sp(&["analyze", "--project", project, "--since", "2026-03-01"]),
        Some(name) => {
            let _server = alpha.serve_external();
            common::pair(&alpha, &beta, name);
            let beta_widgets = beta.checkout("widgets");
            beta.sp(&["--server", name, "analyze", "--project", beta_widgets.to_str().unwrap(), "--since", "2026-03-01"])
        }
    };
    assert_eq!(output.status.code(), Some(1), "one session failed\nstdout: {}\nstderr: {}\n{}", stdout(&output), stderr(&output), server_logs(&alpha.root));
    stderr(&output)
}

fn assert_session_lines(err: &str) {
    let fine = err.lines().find(|l| l.contains("Prompt of aaaa1111-fine")).unwrap_or_else(|| panic!("no line for the analyzed session: {err}"));
    assert!(fine.starts_with('[') && fine.contains("/2] Prompt of aaaa1111-fine: succeeded, 1 components, 1 annotations"), "{err}");
    let broken = err.lines().find(|l| l.contains("Prompt of aaaa2222-broken")).unwrap_or_else(|| panic!("no line for the failed session: {err}"));
    assert!(broken.starts_with('[') && broken.contains("/2] Prompt of aaaa2222-broken: failed: claude failed") && broken.contains("claude broke"), "{err}");
    assert!(!err.contains("] done"), "{err}");
}

#[test]
fn sp_analyze_prints_each_sessions_title_with_its_summary_or_the_first_line_of_its_error() {
    let dir = TempDir::new("cli-analyze-lines");
    assert_session_lines(&analyze_two(&dir, None));
}

#[test]
fn sp_analyze_with_a_paired_server_prints_each_sessions_title_with_its_summary_or_the_first_line_of_its_error() {
    let dir = TempDir::new("cli-remote-analyze-lines");
    assert_session_lines(&analyze_two(&dir, Some("lab")));
}

/// Forwards bytes between `sp` and a real server on `127.0.0.1:{target}`, rewriting the Host header
/// for the target's port and counting `job` requests. It only relays; the server answers.
struct CountingRelay {
    port: u16,
    sent: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    accept: Option<std::thread::JoinHandle<()>>,
}

impl CountingRelay {
    fn start(target: u16) -> CountingRelay {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};
        use std::sync::atomic::Ordering;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        // hyper writes header names in lowercase.
        let (from, to) = (format!("host: 127.0.0.1:{port}"), format!("host: 127.0.0.1:{target}"));
        let accept = {
            let (sent, stop) = (sent.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let Ok((client, _)) = listener.accept() else {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    };
                    client.set_nonblocking(false).unwrap();
                    let server = TcpStream::connect(("127.0.0.1", target)).unwrap();
                    let (mut client_read, mut server_write) = (client.try_clone().unwrap(), server.try_clone().unwrap());
                    let (mut server_read, mut client_write) = (server, client);
                    let (sent, from, to) = (sent.clone(), from.clone(), to.clone());
                    std::thread::spawn(move || {
                        let mut buf = [0u8; 16 * 1024];
                        while let Ok(n) = client_read.read(&mut buf) {
                            if n == 0 {
                                break;
                            }
                            let chunk = String::from_utf8_lossy(&buf[..n]).replace(&from, &to);
                            sent.lock().unwrap().extend_from_slice(chunk.as_bytes());
                            if server_write.write_all(chunk.as_bytes()).is_err() {
                                break;
                            }
                        }
                        let _ = server_write.shutdown(std::net::Shutdown::Both);
                    });
                    std::thread::spawn(move || {
                        let _ = std::io::copy(&mut server_read, &mut client_write);
                        let _ = client_write.shutdown(std::net::Shutdown::Both);
                    });
                }
            })
        };
        CountingRelay { port, sent, stop, accept: Some(accept) }
    }

    fn job_polls(&self) -> usize {
        String::from_utf8_lossy(&self.sent.lock().unwrap()).matches("POST /api/v1/job ").count()
    }
}

impl Drop for CountingRelay {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
    }
}

#[test]
fn sp_analyze_on_a_paired_server_polls_the_job_about_twice_a_second_not_ten_times() {
    let dir = TempDir::new("cli-remote-analyze-polls");
    let alpha = Machine::new(&dir, "alpha");
    let beta = Machine::new(&dir, "beta");
    let claude = alpha.fake_claude();
    let widgets = alpha.checkout("widgets");
    alpha.session(&widgets, "aaaa1111-slow", "2026-03-10");
    ok(&alpha.sp(&["index"]), "sp index on alpha");
    let _server = alpha.serve_local();
    common::pair(&alpha, &beta, "lab");
    let session_player_client::LocalProbe::Live { port, .. } = session_player_client::probe_local(&alpha.data) else { panic!("alpha's server isn't live") };
    let relay = CountingRelay::start(port);
    let mut servers: serde_json::Value = serde_json::from_slice(&std::fs::read(beta.data.join("servers.json")).unwrap()).unwrap();
    servers[0]["port"] = relay.port.into();
    std::fs::write(beta.data.join("servers.json"), serde_json::to_vec(&servers).unwrap()).unwrap();
    claude.hold();
    let release = {
        let dir = claude.dir.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(3));
            let _ = std::fs::remove_file(dir.join("hold"));
        })
    };

    let output = beta.sp(&["--server", "lab", "analyze", "--project", beta.checkout("widgets").to_str().unwrap(), "--since", "2026-03-01"]);
    release.join().unwrap();

    assert_eq!(output.status.code(), Some(0), "stdout: {}\nstderr: {}\n{}", stdout(&output), stderr(&output), server_logs(&alpha.root));
    let polls = relay.job_polls();
    assert!((2..=10).contains(&polls), "{polls} job polls in about 3 s");
}
