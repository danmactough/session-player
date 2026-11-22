// servers.json: owner-only, replaced atomically, a server replaced in place by name, a pinned
// fingerprint never replaced by none, and an unparsable file an error rather than overwritten.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;

use session_player_client::{PairedServer, ServersStore};
use session_player_core::ops::DeviceId;
use tempdir::TempDir;

fn server(name: &str, port: u16, fingerprint: Option<&str>) -> PairedServer {
    PairedServer {
        name: name.to_string(),
        address: "lab.local".to_string(),
        port,
        fingerprint: fingerprint.map(str::to_string),
        token: format!("token-{name}-{port}"),
        device: DeviceId(format!("dev-{name}")),
    }
}

fn names(store: &ServersStore) -> Vec<String> {
    store.list().unwrap().into_iter().map(|s| s.name).collect()
}

#[test]
fn an_empty_store_lists_nothing_and_saved_servers_read_back() {
    let dir = TempDir::new("client-store-roundtrip");
    let store = ServersStore::new(&dir);
    assert!(store.list().unwrap().is_empty());
    assert_eq!(store.get("lab").unwrap(), None);

    store.save(server("lab", 47801, Some("ab"))).unwrap();
    store.save(server("home", 47801, None)).unwrap();

    assert_eq!(ServersStore::new(&dir).get("lab").unwrap(), Some(server("lab", 47801, Some("ab"))));
    assert_eq!(names(&store), vec!["lab", "home"]);
}

#[test]
fn servers_json_is_owner_only_and_replaced_by_a_rename() {
    let dir = TempDir::new("client-store-file");
    let store = ServersStore::new(&dir);
    store.save(server("lab", 47801, Some("ab"))).unwrap();
    let file = dir.join("servers.json");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let before = std::fs::metadata(&file).unwrap();
        assert_eq!(before.permissions().mode() & 0o777, 0o600);
        store.save(server("home", 47801, None)).unwrap();
        let after = std::fs::metadata(&file).unwrap();
        assert_ne!(after.ino(), before.ino(), "servers.json was rewritten in place instead of replaced by a rename");
        assert_eq!(after.permissions().mode() & 0o777, 0o600);
    }
    let others: Vec<String> = std::fs::read_dir(&*dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n != "servers.json").collect();
    assert!(others.is_empty(), "temp files left: {others:?}");
}

#[test]
fn saving_a_known_name_replaces_it_in_place() {
    let dir = TempDir::new("client-store-replace");
    let store = ServersStore::new(&dir);
    for name in ["a", "lab", "z"] {
        store.save(server(name, 1, Some("ab"))).unwrap();
    }

    store.save(server("lab", 2, Some("cd"))).unwrap();

    assert_eq!(names(&store), vec!["a", "lab", "z"]);
    assert_eq!(store.get("lab").unwrap(), Some(server("lab", 2, Some("cd"))), "a re-pair may change the pin");
}

#[test]
fn a_pinned_fingerprint_is_never_replaced_by_none() {
    let dir = TempDir::new("client-store-downgrade");
    let store = ServersStore::new(&dir);
    store.save(server("lab", 1, Some("ab"))).unwrap();

    assert!(store.save(server("lab", 1, None)).is_err(), "a save dropped the pin");

    assert_eq!(store.get("lab").unwrap(), Some(server("lab", 1, Some("ab"))));
}

#[test]
fn remove_reports_whether_a_server_had_that_name() {
    let dir = TempDir::new("client-store-remove");
    let store = ServersStore::new(&dir);
    store.save(server("lab", 1, Some("ab"))).unwrap();
    store.save(server("home", 1, None)).unwrap();

    assert!(store.remove("lab").unwrap());
    assert!(!store.remove("lab").unwrap());
    assert_eq!(names(&store), vec!["home"]);
}

#[test]
fn an_unparsable_file_is_an_error_and_is_left_as_it_was() {
    let dir = TempDir::new("client-store-garbage");
    let file = dir.join("servers.json");
    std::fs::write(&file, "{ not json").unwrap();
    let store = ServersStore::new(&dir);

    assert!(store.list().is_err());
    assert!(store.get("lab").is_err());
    assert!(store.save(server("lab", 1, Some("ab"))).is_err());
    assert!(store.remove("lab").is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ not json");
}
