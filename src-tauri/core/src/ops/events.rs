//! The bounded ring of pushed events callers read by id.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use super::wire::{PushedEvent, ServerEvent};

pub const DEFAULT_CAPACITY: usize = 1000;

pub struct Events {
    capacity: usize,
    ring: Mutex<Ring>,
    arrived: Condvar,
}

struct Ring {
    next_id: u64,
    events: VecDeque<ServerEvent>,
}

pub enum Since {
    Events(Vec<ServerEvent>),
    /// The ring no longer holds (or never held) the event after `after`; `newest` is the id to
    /// resume from.
    Gone { newest: u64 },
}

impl Events {
    /// Ids start at `first_id` (the start time in ms), so an id from an earlier run is always
    /// older than anything this ring holds.
    pub fn new(capacity: usize, first_id: u64) -> Events {
        Events { capacity, ring: Mutex::new(Ring { next_id: first_id, events: VecDeque::new() }), arrived: Condvar::new() }
    }

    pub fn publish(&self, event: PushedEvent, at: u64) {
        let mut ring = self.ring.lock().unwrap();
        let id = ring.next_id;
        ring.next_id += 1;
        ring.events.push_back(ServerEvent { id, at, event });
        while ring.events.len() > self.capacity {
            ring.events.pop_front();
        }
        self.arrived.notify_all();
    }

    pub fn since(&self, after: Option<u64>, wait: Duration) -> Since {
        let deadline = Instant::now() + wait;
        let mut ring = self.ring.lock().unwrap();
        let after = after.unwrap_or(ring.next_id - 1);
        loop {
            let oldest = ring.events.front().map_or(ring.next_id, |e| e.id);
            if after < oldest - 1 || after >= ring.next_id {
                return Since::Gone { newest: ring.next_id - 1 };
            }
            let newer: Vec<ServerEvent> = ring.events.iter().filter(|e| e.id > after).cloned().collect();
            let now = Instant::now();
            if !newer.is_empty() || now >= deadline {
                return Since::Events(newer);
            }
            ring = self.arrived.wait_timeout(ring, deadline - now).unwrap().0;
        }
    }
}
