// The operation registry: every operation is declared once and reached the same way by name
// (the server) or by its typed static (the in-process CLI).
#[path = "../../testutil/tempdir.rs"]
mod tempdir;

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use serde_json::json;
use session_player_core::ops::{
    self, API_MINOR, API_VERSION, Access, Body, Ctx, DeviceId, DispatchParams, ErrorCode, Extension, HELLO, HelloRequest, Input, OpError,
    OpSpec, OpenParams, Output, Paths,
};
use tempdir::TempDir;

fn paths(dir: &Path) -> Paths {
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    Paths { db: data.join("index.sqlite"), library: data.join("library"), logs: data.join("logs"), data }
}

fn open_with(dir: &Path, extensions: Vec<Extension>) -> anyhow::Result<Arc<Ctx>> {
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    Ctx::open(OpenParams {
        paths: paths(dir),
        home,
        version: "9.9.9".to_string(),
        claude: None,
        clock: Some(Arc::new(|| 7_000_000)),
        event_capacity: None,
        extensions,
    })
}

fn open(dir: &Path) -> Arc<Ctx> {
    open_with(dir, vec![]).expect("open the Ctx")
}

fn local() -> DeviceId {
    DeviceId("local".to_string())
}

fn dispatch_json(ctx: &Arc<Ctx>, name: &str, input: serde_json::Value) -> Result<serde_json::Value, OpError> {
    match ops::dispatch(ctx, DispatchParams { name: name.to_string(), device: local(), input: Input::Json(input) })? {
        Output::Json(value) => Ok(value),
        Output::Raw(_) => panic!("{name} answered with a raw body"),
    }
}

fn echo(_ctx: &Arc<Ctx>, _device: &DeviceId, input: Input) -> Result<Output, OpError> {
    match input {
        Input::Json(value) => Ok(Output::Json(json!({ "echo": value }))),
        Input::Raw { .. } => Err(OpError::BadRequest("expected JSON".to_string())),
    }
}

fn extension(name: &'static str) -> Extension {
    Extension { spec: OpSpec { name, body: Body::Json, inline: false, access: Access::Authenticated }, handler: echo }
}

#[test]
fn hello_through_call_returns_the_api_version_the_server_version_and_the_home_folder() {
    let dir = TempDir::new("registry-hello");
    let ctx = open(&dir);

    let hello = ops::call(&ctx, &HELLO, &HelloRequest {}).expect("hello");

    assert_eq!(hello.api_version, API_VERSION);
    assert_eq!(hello.api_minor, API_MINOR);
    assert_eq!((API_VERSION, API_MINOR), (1, 0));
    assert_eq!(hello.server_version, "9.9.9");
    assert_eq!(hello.home, dir.join("home").to_string_lossy());
    assert!(hello.loopback_only, "with no paired device the server stays on loopback");
}

#[test]
fn dispatching_hello_by_name_gives_the_same_json_as_the_typed_call() {
    let dir = TempDir::new("registry-dispatch");
    let ctx = open(&dir);

    let typed = ops::call(&ctx, &HELLO, &HelloRequest {}).expect("hello");
    let by_name = dispatch_json(&ctx, "hello", json!({})).expect("dispatch hello");

    assert_eq!(by_name, serde_json::to_value(typed).unwrap());
}

#[test]
fn an_unknown_operation_name_is_refused_as_an_unknown_operation_and_has_no_spec() {
    let dir = TempDir::new("registry-unknown");
    let ctx = open(&dir);

    let refused = dispatch_json(&ctx, "no_such_operation", json!({}));

    assert!(matches!(refused, Err(OpError::UnknownOperation(_))), "{refused:?}");
    assert_eq!(ops::spec_of(&ctx, "no_such_operation"), None);
}

#[test]
fn a_request_body_that_does_not_fit_the_operation_is_a_bad_request() {
    let dir = TempDir::new("registry-bad-body");
    let ctx = open(&dir);

    let refused = dispatch_json(&ctx, "reindex", json!({ "full": "yes please" }));

    assert!(matches!(refused, Err(OpError::BadRequest(_))), "{refused:?}");
}

#[test]
fn every_operation_name_is_unique_and_has_a_spec() {
    let dir = TempDir::new("registry-unique");
    let ctx = open(&dir);

    let specs = ops::specs(&ctx);
    let names: HashSet<&str> = specs.iter().map(|s| s.name).collect();

    assert_eq!(names.len(), specs.len(), "duplicate names in {specs:?}");
    for name in ["hello", "job", "cancel_job", "list_sessions", "reindex"] {
        let spec = ops::spec_of(&ctx, name).unwrap_or_else(|| panic!("no spec for {name}"));
        assert_eq!(spec.name, name);
        assert_eq!(spec.body, Body::Json);
    }
}

#[test]
fn in_core_no_operation_is_anonymous_or_kept_off_http_and_only_hello_job_and_cancel_job_run_inline() {
    let dir = TempDir::new("registry-access");
    let ctx = open(&dir);

    let specs = ops::specs(&ctx);
    let named = |pick: &dyn Fn(&OpSpec) -> bool| specs.iter().filter(|s| pick(s)).map(|s| s.name).collect::<HashSet<_>>();

    assert_eq!(named(&|s| s.access == Access::Anonymous), HashSet::new(), "connect is a server extension");
    assert_eq!(named(&|s| s.access == Access::NotOverHttp), HashSet::new(), "service is a server extension");
    assert_eq!(named(&|s| s.inline), HashSet::from(["hello", "job", "cancel_job"]));
}

#[test]
fn an_extension_is_listed_with_its_spec_and_reached_by_name() {
    let dir = TempDir::new("registry-extension");
    let ctx = open_with(&dir, vec![extension("test_echo")]).expect("open with an extension");

    assert_eq!(ops::spec_of(&ctx, "test_echo"), Some(extension("test_echo").spec));
    assert!(ops::specs(&ctx).iter().any(|s| s.name == "test_echo"));
    assert_eq!(dispatch_json(&ctx, "test_echo", json!({ "n": 1 })).expect("dispatch"), json!({ "echo": { "n": 1 } }));
}

#[test]
fn an_extension_named_like_a_core_operation_makes_open_fail() {
    let dir = TempDir::new("registry-dup-core");

    assert!(open_with(&dir, vec![extension("hello")]).is_err());
}

#[test]
fn two_extensions_with_the_same_name_make_open_fail() {
    let dir = TempDir::new("registry-dup-ext");

    assert!(open_with(&dir, vec![extension("test_echo"), extension("test_echo")]).is_err());
}

#[test]
fn error_codes_are_snake_case_on_the_wire_and_map_to_their_http_statuses() {
    let table = [
        (ErrorCode::Unauthenticated, "unauthenticated", 401),
        (ErrorCode::ForbiddenOrigin, "forbidden_origin", 403),
        (ErrorCode::UnknownOperation, "unknown_operation", 404),
        (ErrorCode::NotFound, "not_found", 404),
        (ErrorCode::MethodNotAllowed, "method_not_allowed", 405),
        (ErrorCode::Conflict, "conflict", 409),
        (ErrorCode::TooLarge, "too_large", 413),
        (ErrorCode::BadRequest, "bad_request", 422),
        (ErrorCode::Internal, "internal", 500),
    ];
    for (code, wire, status) in table {
        assert_eq!(serde_json::to_value(code).unwrap(), json!(wire));
        assert_eq!(code.status(), status, "{wire}");
    }
}
