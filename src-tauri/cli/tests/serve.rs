// `sp serve`, `sp pair`, `sp connect`, `sp devices` and `sp unpair`, driving the real binary.

#[path = "../../testutil/tempdir.rs"]
mod tempdir;

mod common;

use std::process::Stdio;
use std::time::{Duration, Instant};

use common::{Machine, loopback_config, ok, pairing_string, stderr, stdout};
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
fn sp_serve_with_exit_on_stdin_eof_runs_until_its_standard_input_closes() {
    let dir = TempDir::new("cli-serve-stdin");
    let alpha = Machine::new(&dir, "alpha");
    let mut args = vec!["serve".to_string()];
    args.extend(loopback_config().to_args());
    args.push("--exit-on-stdin-eof".to_string());
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut child = alpha.command(&args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().expect("start sp serve");
    wait_until("the server is live", || matches!(probe_local(&alpha.data), LocalProbe::Live { .. }));
    assert!(child.try_wait().unwrap().is_none(), "an open pipe keeps it running");

    drop(child.stdin.take());

    wait_until("sp serve exits", || child.try_wait().unwrap().is_some());
    assert!(child.wait().unwrap().success());
    assert_eq!(probe_local(&alpha.data), LocalProbe::NotLive, "server.lock is released");
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
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
fn sp_serve_refuses_a_certificate_without_its_key() {
    let dir = TempDir::new("cli-serve-cert-without-key");
    let alpha = Machine::new(&dir, "alpha");

    let output = alpha.sp(&["serve", "--tls-cert", "cert.pem"]);

    assert_ne!(output.status.code(), Some(0));
    assert!(stderr(&output).contains("--tls-key"), "{}", stderr(&output));
    assert_eq!(probe_local(&alpha.data), LocalProbe::NotLive);
}

#[test]
fn sp_serve_refuses_a_wildcard_bind_without_an_advertised_host() {
    let dir = TempDir::new("cli-serve-wildcard-bind");
    let alpha = Machine::new(&dir, "alpha");

    let output = alpha.sp(&["serve", "--port", "0", "--external-port", "0", "--bind", "0.0.0.0"]);

    assert_ne!(output.status.code(), Some(0));
    assert!(stderr(&output).contains("--advertise"), "{}", stderr(&output));
}

#[test]
fn sp_serve_with_allow_insecure_http_serves_plain_http_off_loopback_and_warns() {
    let dir = TempDir::new("cli-serve-plain-allowed");
    let alpha = Machine::new(&dir, "alpha");
    let beta = Machine::new(&dir, "beta");
    let server = alpha.serve(&plain_external_config());

    let pairing = PairingString::parse(&pairing_string(&alpha.sp(&["pair"]))).expect("a pairing string");
    assert_eq!(pairing.fingerprint, None, "plain HTTP pins no certificate");
    common::pair(&alpha, &beta, "lab");
    ok(&beta.sp(&["--server", "lab", "reports"]), "sp --server lab reports over plain HTTP");
    let warned = server.stderr();
    assert!(warned.contains("plain HTTP") && warned.contains("unencrypted"), "{warned}");
}
