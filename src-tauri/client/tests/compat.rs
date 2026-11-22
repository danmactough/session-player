// check_compat compares a server's hello with this client's API version: a different api_version
// is an error that says which side to upgrade; a different api_minor still works.
use session_player_client::{Compat, check_compat};
use session_player_core::ops::{API_MINOR, API_VERSION, HelloResponse};

fn hello(api_version: u32, api_minor: u32) -> HelloResponse {
    HelloResponse { api_version, api_minor, server_version: "7.8.9".to_string(), home: "/home/x".to_string(), loopback_only: true }
}

#[test]
fn the_same_api_version_and_minor_is_exact() {
    assert_eq!(check_compat(&hello(API_VERSION, API_MINOR)), Ok(Compat::Exact));
}

#[test]
fn a_higher_minor_is_a_newer_server_that_still_works() {
    assert_eq!(check_compat(&hello(API_VERSION, API_MINOR + 1)), Ok(Compat::NewerServer));
}

#[test]
fn an_older_api_version_says_to_upgrade_the_server_and_names_its_version() {
    let message = check_compat(&hello(API_VERSION - 1, 9)).expect_err("an older api_version");
    assert!(message.contains("7.8.9"), "{message}");
    assert!(message.to_lowercase().contains("upgrade the server"), "{message}");
}

#[test]
fn a_newer_api_version_says_to_upgrade_this_client_and_names_the_servers_version() {
    let message = check_compat(&hello(API_VERSION + 1, 0)).expect_err("a newer api_version");
    assert!(message.contains("7.8.9"), "{message}");
    assert!(!message.to_lowercase().contains("upgrade the server"), "{message}");
    assert!(message.to_lowercase().contains("upgrade"), "{message}");
}
