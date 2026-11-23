use std::path::Path;

/// rust-embed needs the repository's dist/ to exist at compile time. When the web UI hasn't been
/// built (a Rust-only checkout or CI job), a placeholder page stands in for it.
fn main() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    if dist.exists() {
        return;
    }
    std::fs::create_dir_all(&dist).expect("create dist/");
    let page = "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Session Player</title></head>\n<body>\n<p>This sp was built without the web UI. Run <code>npm run build</code>, then rebuild sp.</p>\n</body>\n</html>\n";
    std::fs::write(dist.join("index.html"), page).expect("write the placeholder dist/index.html");
}
