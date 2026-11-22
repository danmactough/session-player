//! Watches the session roots while the server runs and starts an incremental index pass when a
//! file changes. Raw notify events with a hand-written debounce: notify-debouncer-mini fed the
//! index pass's own reads back into the watcher on Linux. Access events (reads) are ignored.

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher as _};
use session_player_core::index;
use session_player_core::ops::{self, Ctx, REINDEX, ReindexRequest};

pub const DEBOUNCE: Duration = Duration::from_millis(400);

enum Message {
    Changed,
    Stop,
}

pub(crate) struct Watcher {
    watcher: notify::RecommendedWatcher,
    messages: Sender<Message>,
    thread: std::thread::JoinHandle<()>,
}

impl Watcher {
    pub(crate) fn stop(self) {
        drop(self.watcher);
        let _ = self.messages.send(Message::Stop);
        let _ = self.thread.join();
    }
}

/// Watches the roots registered now.
pub(crate) fn start(ctx: &Arc<Ctx>) -> anyhow::Result<Watcher> {
    let (messages, received) = std::sync::mpsc::channel();
    let changes = messages.clone();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        if let Ok(event) = result
            && !matches!(event.kind, EventKind::Access(_))
        {
            let _ = changes.send(Message::Changed);
        }
    })?;
    let conn = index::open(&ctx.paths().db)?;
    for root in index::roots(&conn)? {
        if let Err(e) = watcher.watch(Path::new(&root), RecursiveMode::Recursive) {
            eprintln!("Couldn't watch {root}: {e}");
        }
    }
    drop(conn);
    let ctx = ctx.clone();
    let thread = std::thread::Builder::new().name("sp-watcher".to_string()).spawn(move || debounce(&ctx, &received))?;
    Ok(Watcher { watcher, messages, thread })
}

/// The first change opens a 400 ms window; the pass starts when it closes, so a burst of writes
/// (an agent appending to a session) starts one pass, and a steady stream starts one per window.
fn debounce(ctx: &Arc<Ctx>, received: &Receiver<Message>) {
    loop {
        match received.recv() {
            Ok(Message::Changed) => {}
            Ok(Message::Stop) | Err(_) => return,
        }
        let deadline = Instant::now() + DEBOUNCE;
        loop {
            match received.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Message::Changed) => {}
                Ok(Message::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => break,
            }
        }
        if let Err(e) = ops::call(ctx, &REINDEX, &ReindexRequest { full: false }) {
            eprintln!("Couldn't start an index pass: {e:?}");
        }
    }
}
