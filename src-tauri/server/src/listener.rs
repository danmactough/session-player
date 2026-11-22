//! The external listener controller: the listener is open iff `external_bind` is set and a device
//! or an unexpired offer exists. It polls, is nudged on pair and unpair, and closes after a grace.

use std::time::Duration;

pub const POLL: Duration = Duration::from_millis(100);
pub const CLOSE_GRACE: Duration = Duration::from_millis(1000);
/// How long `pair` waits for the listener before answering with its bound port.
pub const PAIR_WAIT: Duration = Duration::from_secs(2);

pub struct Wanted {
    pub external_bind: bool,
    pub devices: usize,
    pub open_offers: usize,
}

pub fn wanted(state: &Wanted) -> bool {
    let _ = state;
    todo!("listener::wanted")
}
