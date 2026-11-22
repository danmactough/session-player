//! The web UI, embedded from the repository's dist/. Served on every path outside /api/ without a
//! token; the Host and Origin checks still run first.

use axum::http::{Method, StatusCode, header};
use axum::response::Response;
use session_player_core::ops::ErrorCode;

use crate::auth::error_response;

#[derive(rust_embed::RustEmbed)]
#[folder = "../../dist"]
struct Dist;

pub fn response(method: &Method, path: &str) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return error_response(ErrorCode::MethodNotAllowed, "The web UI takes GET.");
    }
    let name = match path.trim_start_matches('/') {
        "" => "index.html",
        name => name,
    };
    let (name, file) = match Dist::get(name) {
        Some(file) => (name, file),
        // A missing script or stylesheet must fail loudly rather than load the page in its place.
        None if path.starts_with("/assets/") => return error_response(ErrorCode::NotFound, "Nothing is here."),
        // Any other path is one of the UI's own routes.
        None => match Dist::get("index.html") {
            Some(file) => ("index.html", file),
            None => return error_response(ErrorCode::NotFound, "Nothing is here."),
        },
    };
    // hyper leaves the body out of a HEAD response.
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type(name))
        .body(axum::body::Body::from(file.data))
        .unwrap_or_else(|e| error_response(ErrorCode::Internal, &e.to_string()))
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).as_deref() {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("txt") => "text/plain; charset=utf-8",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}
