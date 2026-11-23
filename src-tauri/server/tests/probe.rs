// probe_local against a real server: NotLive with none, Live with its port and token while it
// runs, NotLive after it stops; and probing never makes a start fail.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::{CtxParams, loopback_config, open_ctx, read_token, start};
use session_player_client::{LocalProbe, probe_local};
use tempdir::TempDir;

#[test]
fn probe_local_is_not_live_without_a_server_live_with_its_port_and_token_while_it_runs_and_not_live_after_it_stops() {
    let dir = TempDir::new("server-probe");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let data = ctx.paths().data.clone();

    assert_eq!(probe_local(&data), LocalProbe::NotLive);

    let server = start(&ctx, loopback_config());
    let expected = LocalProbe::Live { port: server.addrs().loopback.port(), token: Some(read_token(&data)) };
    assert_eq!(probe_local(&data), expected);

    server.shutdown();
    assert_eq!(probe_local(&data), LocalProbe::NotLive, "a stopped server's lock file must not read as live");
}

#[test]
fn probes_running_without_pause_never_make_a_server_start_fail_and_never_report_port_zero() {
    let dir = TempDir::new("server-probe-race");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let data = ctx.paths().data.clone();
    let stop = Arc::new(AtomicBool::new(false));
    // Stops the prober even when the test fails partway.
    struct StopOnDrop(Arc<AtomicBool>);
    impl Drop for StopOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let _stop_on_drop = StopOnDrop(stop.clone());
    let prober = {
        let (data, stop) = (data.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut probes = 0;
            while !stop.load(Ordering::SeqCst) {
                if let LocalProbe::Live { port, .. } = probe_local(&data) {
                    assert_ne!(port, 0, "a probe reported a live server on port 0");
                }
                probes += 1;
            }
            probes
        })
    };

    for _ in 0..20 {
        let server = start(&ctx, loopback_config());
        server.shutdown();
    }

    stop.store(true, Ordering::SeqCst);
    let probes = prober.join().expect("the prober panicked");
    assert!(probes > 20, "the prober barely ran ({probes} probes)");
}
