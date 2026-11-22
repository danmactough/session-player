//! Watches the session roots while the server runs and starts an incremental index pass when a
//! file changes. Raw notify events with a hand-written debounce: notify-debouncer-mini fed the
//! index pass's own reads back into the watcher on Linux. Access events (reads) are ignored.

use std::collections::BTreeSet;
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
    /// The root list may have changed: an index pass started or ended, which add_root and
    /// remove_root both cause.
    Roots,
    Stop,
}

pub(crate) struct Watcher {
    messages: Sender<Message>,
    thread: std::thread::JoinHandle<()>,
}

impl Watcher {
    pub(crate) fn stop(self) {
        let _ = self.messages.send(Message::Stop);
        let _ = self.thread.join();
    }

    pub(crate) fn roots_changed(&self) -> RootsChanged {
        RootsChanged(self.messages.clone())
    }
}

pub(crate) struct RootsChanged(Sender<Message>);

impl RootsChanged {
    pub(crate) fn send(&self) {
        let _ = self.0.send(Message::Roots);
    }
}

struct Watching {
    watcher: notify::RecommendedWatcher,
    roots: BTreeSet<String>,
}

impl Watching {
    /// Watches the roots registered now and stops watching the ones no longer registered.
    fn update(&mut self, ctx: &Ctx) {
        let roots = match index::open(&ctx.paths().db).and_then(|conn| index::roots(&conn)) {
            Ok(roots) => roots.into_iter().collect::<BTreeSet<_>>(),
            Err(e) => {
                eprintln!("Couldn't read the session folders: {e:#}");
                return;
            }
        };
        for root in self.roots.difference(&roots) {
            let _ = self.watcher.unwatch(Path::new(root));
        }
        for root in roots.difference(&self.roots) {
            if let Err(e) = self.watcher.watch(Path::new(root), RecursiveMode::Recursive) {
                eprintln!("Couldn't watch {root}: {e}");
            }
        }
        self.roots = roots;
    }
}

/// Watches the roots registered now, and the roots registered later once told they changed.
pub(crate) fn start(ctx: &Arc<Ctx>) -> anyhow::Result<Watcher> {
    let (messages, received) = std::sync::mpsc::channel();
    let changes = messages.clone();
    let watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        if let Ok(event) = result
            && !matches!(event.kind, EventKind::Access(_))
        {
            let _ = changes.send(Message::Changed);
        }
    })?;
    let mut watching = Watching { watcher, roots: BTreeSet::new() };
    watching.update(ctx);
    let ctx = ctx.clone();
    let thread = std::thread::Builder::new().name("sp-watcher".to_string()).spawn(move || debounce(&ctx, &received, &mut watching))?;
    Ok(Watcher { messages, thread })
}

/// The first change opens a 400 ms window; the pass starts when it closes, so a burst of writes
/// (an agent appending to a session) starts one pass, and a steady stream starts one per window.
fn debounce(ctx: &Arc<Ctx>, received: &Receiver<Message>, watching: &mut Watching) {
    loop {
        match received.recv() {
            Ok(Message::Changed) => {}
            Ok(Message::Roots) => {
                watching.update(ctx);
                continue;
            }
            Ok(Message::Stop) | Err(_) => return,
        }
        let deadline = Instant::now() + DEBOUNCE;
        loop {
            match received.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Message::Changed) => {}
                Ok(Message::Roots) => watching.update(ctx),
                Ok(Message::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => break,
            }
        }
        if let Err(e) = ops::call(ctx, &REINDEX, &ReindexRequest { full: false }) {
            eprintln!("Couldn't start an index pass: {e:?}");
        }
    }
}
