//! Where a command runs: in this process, through the server on this machine's data folder, or
//! on a paired server. Pure, so every case is tested without a server.

use session_player_client::{LocalProbe, PairedServer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// In this process, on this machine's database.
    Local,
    /// The server running on this machine's data folder, over its loopback listener.
    LocalServer { port: u16, token: String },
    /// A server paired with `sp connect`, by its name in `servers.json`.
    Remote { name: String },
}

pub struct RouteParams<'a> {
    /// `--local`.
    pub local_flag: bool,
    /// `--server <name>`.
    pub server_flag: Option<&'a str>,
    /// Whether a server runs on this machine's data folder.
    pub probe: LocalProbe,
    /// The servers in `servers.json`, which `--server` names one of.
    pub servers: &'a [PairedServer],
}

pub fn resolve(params: RouteParams) -> anyhow::Result<Route> {
    let _ = params;
    todo!("route::resolve")
}

#[cfg(test)]
mod tests {
    use super::*;
    use session_player_core::ops::DeviceId;

    fn lab() -> PairedServer {
        PairedServer {
            name: "lab".into(),
            address: "192.0.2.10".into(),
            port: 47801,
            fingerprint: Some("ab".repeat(32)),
            token: "device-token".into(),
            device: DeviceId("d1".into()),
        }
    }

    fn live() -> LocalProbe {
        LocalProbe::Live { port: 47800, token: Some("local-token".into()) }
    }

    #[test]
    fn local_flag_runs_in_this_process_even_when_a_local_server_is_live() {
        let route = resolve(RouteParams { local_flag: true, server_flag: None, probe: live(), servers: &[lab()] }).unwrap();
        assert_eq!(route, Route::Local);
    }

    #[test]
    fn a_live_local_server_is_used_by_default() {
        let route = resolve(RouteParams { local_flag: false, server_flag: None, probe: live(), servers: &[lab()] }).unwrap();
        assert_eq!(route, Route::LocalServer { port: 47800, token: "local-token".into() });
    }

    #[test]
    fn with_no_live_local_server_the_command_runs_in_this_process() {
        let route = resolve(RouteParams { local_flag: false, server_flag: None, probe: LocalProbe::NotLive, servers: &[lab()] }).unwrap();
        assert_eq!(route, Route::Local);
    }

    #[test]
    fn server_flag_picks_the_paired_server_with_that_name_over_a_live_local_server() {
        let route = resolve(RouteParams { local_flag: false, server_flag: Some("lab"), probe: live(), servers: &[lab()] }).unwrap();
        assert_eq!(route, Route::Remote { name: "lab".into() });
    }

    #[test]
    fn an_unknown_server_name_is_an_error_that_says_to_pair_with_sp_connect() {
        let err = resolve(RouteParams { local_flag: false, server_flag: Some("elsewhere"), probe: live(), servers: &[lab()] }).unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("elsewhere") && message.contains("sp connect"), "{message}");
    }
}
