// `sp serve`, `sp pair`, `sp connect`, `sp devices` and `sp unpair`, driving the real binary.

#[path = "../../testutil/tempdir.rs"]
mod tempdir;

mod common;

use std::time::Duration;

use common::{Machine, ok, pairing_string, stderr, stdout};
use session_player_client::{LocalProbe, PairingString, probe_local};
use session_player_server::{ServerConfig, TlsMode};
use tempdir::TempDir;

#[test]
fn sp_serve_answers_hello_and_exits_cleanly_on_sigterm() {
    let dir = TempDir::new("cli-serve");
    let alpha = Machine::new(&dir, "alpha");
    let mut server = alpha.serve_local();
    assert!(matches!(probe_local(&alpha.data), LocalProbe::Live { .. }));

    let status = server.terminate().expect("sp serve exits within 10 s of SIGTERM");
    assert!(status.success(), "sp serve exited with {status}\n{}", common::server_logs(&alpha.root));
    assert_eq!(probe_local(&alpha.data), LocalProbe::NotLive, "server.lock is released");
}

#[test]
fn sp_pair_prints_a_pairing_string_for_the_external_tls_listener() {
    let dir = TempDir::new("cli-pair");
    let alpha = Machine::new(&dir, "alpha");
    let _server = alpha.serve_external();

    let output = alpha.sp(&["pair"]);
    ok(&output, "sp pair");
    let pairing = PairingString::parse(&pairing_string(&output)).expect("a valid sp1. string");
    assert_eq!(pairing.address, "127.0.0.1");
    assert_ne!(pairing.port, 0);
    assert!(pairing.fingerprint.is_some(), "the external listener serves TLS, so the string pins its certificate");
}

#[test]
fn sp_connect_on_another_data_folder_saves_the_server_under_its_name() {
    let dir = TempDir::new("cli-connect");
    let alpha = Machine::new(&dir, "alpha");
    let beta = Machine::new(&dir, "beta");
    let _server = alpha.serve_external();

    let pairing = pairing_string(&alpha.sp(&["pair"]));
    ok(&beta.sp(&["connect", &pairing, "--name", "lab"]), "sp connect");

    let servers = beta.servers();
    assert_eq!(servers.len(), 1, "{servers:?}");
    let parsed = PairingString::parse(&pairing).unwrap();
    assert_eq!(servers[0].name, "lab");
    assert_eq!((servers[0].address.as_str(), servers[0].port), ("127.0.0.1", parsed.port));
    assert_eq!(servers[0].fingerprint, parsed.fingerprint);
    assert!(alpha.servers().is_empty(), "the server's own data folder gains no servers.json entry");
}

#[test]
fn sp_devices_lists_a_paired_device_and_sp_unpair_revokes_it() {
    let dir = TempDir::new("cli-devices");
    let alpha = Machine::new(&dir, "alpha");
    let beta = Machine::new(&dir, "beta");
    let _server = alpha.serve_external();
    common::pair(&alpha, &beta, "lab");
    let device = beta.servers()[0].device.0.clone();

    let listed = ok(&alpha.sp(&["devices"]), "sp devices");
    assert!(listed.contains(&device), "sp devices lists {device}: {listed}");
    ok(&beta.sp(&["--server", "lab", "reports"]), "sp --server lab reports while paired");

    ok(&alpha.sp(&["unpair", &device]), "sp unpair");
    let listed = ok(&alpha.sp(&["devices"]), "sp devices after unpair");
    assert!(!listed.contains(&device), "{device} is gone: {listed}");
    let refused = beta.sp(&["--server", "lab", "reports"]);
    assert_ne!(refused.status.code(), Some(0), "a revoked device is refused\nstdout: {}", stdout(&refused));
    assert!(!stderr(&refused).contains("panicked"), "{}", stderr(&refused));
}

#[test]
fn sp_unpair_with_an_unknown_device_fails_and_names_it() {
    let dir = TempDir::new("cli-unpair-unknown");
    let alpha = Machine::new(&dir, "alpha");
    let _server = alpha.serve_external();

    let output = alpha.sp(&["unpair", "no-such-device"]);
    assert_ne!(output.status.code(), Some(0));
    assert!(stderr(&output).contains("no-such-device") && !stderr(&output).contains("panicked"), "{}", stderr(&output));
}

fn plain_external_config() -> ServerConfig {
    ServerConfig { tls: TlsMode::Plain, ..common::external_config() }
}

#[test]
fn sp_serve_refuses_plain_http_off_loopback_without_allow_insecure_http() {
    let dir = TempDir::new("cli-serve-plain-refused");
    let alpha = Machine::new(&dir, "alpha");
    std::fs::write(alpha.data.join("server.json"), serde_json::to_vec(&plain_external_config()).unwrap()).unwrap();

    let mut child = alpha.command(&["serve"]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().expect("start sp serve");
    let status = common::wait_exit(&mut child, Duration::from_secs(20));
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("sp serve kept running with plain HTTP off loopback");
    }
    let output = child.wait_with_output().unwrap();

    assert_ne!(output.status.code(), Some(0));
    assert!(stderr(&output).contains("--allow-insecure-http"), "{}", stderr(&output));
    assert_eq!(probe_local(&alpha.data), LocalProbe::NotLive);
}

#[test]
fn sp_serve_with_allow_insecure_http_serves_plain_http_off_loopback_and_warns() {
    let dir = TempDir::new("cli-serve-plain-allowed");
    let alpha = Machine::new(&dir, "alpha");
    let beta = Machine::new(&dir, "beta");
    let server = alpha.serve_with(&plain_external_config(), &["--allow-insecure-http"]);

    let pairing = PairingString::parse(&pairing_string(&alpha.sp(&["pair"]))).expect("a pairing string");
    assert_eq!(pairing.fingerprint, None, "plain HTTP pins no certificate");
    common::pair(&alpha, &beta, "lab");
    ok(&beta.sp(&["--server", "lab", "reports"]), "sp --server lab reports over plain HTTP");
    let warned = server.stderr();
    assert!(warned.contains("plain HTTP") && warned.contains("unencrypted"), "{warned}");
}
