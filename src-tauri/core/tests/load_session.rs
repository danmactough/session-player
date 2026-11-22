// `load_session` returns the transcript with each data: image replaced by an image route path,
// and `Ctx::image` serves those images from a small cache of recent sessions.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
#[path = "common/ops.rs"]
mod common;

use std::sync::Arc;

use session_player_core::model::Event;
use session_player_core::ops::{self, Ctx, IMAGE_SETS, Image, ImageRef, LOAD_SESSION, LoadSessionRequest, OpError};
use tempdir::TempDir;

const LINKED: &str = "https://example.com/diagram.png";

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Writes a session whose prompt carries `images` (media type, bytes) and one linked image.
fn session_with_images(dir: &TempDir, id: &str, images: &[(&str, &[u8])]) -> String {
    let home = common::home(dir);
    let cwd = home.join("code/app");
    let encoded: Vec<(String, String)> = images.iter().map(|(mime, bytes)| (mime.to_string(), b64(bytes))).collect();
    let refs: Vec<(&str, &str)> = encoded.iter().map(|(m, d)| (m.as_str(), d.as_str())).collect();
    common::session_file(common::SessionFile { home: &home, cwd: &cwd, id, prompt: &format!("Look at these, {id}"), images: &refs, linked_images: &[LINKED] })
        .to_string_lossy()
        .into_owned()
}

fn load(ctx: &Arc<Ctx>, path: &str) -> Vec<Event> {
    ops::call(ctx, &LOAD_SESSION, &LoadSessionRequest { path: path.to_string(), branch: None }).expect("load_session")
}

/// The image refs of the events' route paths, in order.
fn refs(events: &[Event]) -> Vec<ImageRef> {
    let mut found = Vec::new();
    for e in events {
        for url in &e.images {
            if let Some(rest) = url.strip_prefix("/api/v1/image/") {
                let n: Vec<u64> = rest.split('/').map(|p| p.parse().unwrap_or_else(|_| panic!("{url}"))).collect();
                let [generation, event, image] = n[..] else { panic!("{url}") };
                found.push(ImageRef { generation, event: event as usize, image: image as usize });
            }
        }
    }
    found
}

#[test]
fn load_session_rewrites_data_image_urls_to_image_route_paths_whose_images_ctx_image_returns() {
    let dir = TempDir::new("load-session-images");
    let ctx = common::open_simple(&dir, None);
    let path = session_with_images(&dir, "s1", &[("image/png", b"png bytes"), ("image/jpeg", b"jpeg bytes")]);

    let events = load(&ctx, &path);

    let (at, event) = events.iter().enumerate().find(|(_, e)| !e.images.is_empty()).expect("an event with images");
    assert_eq!(event.images.len(), 3);
    assert!(event.images.iter().all(|u| !u.starts_with("data:")), "{:?}", event.images);
    assert_eq!(event.images[2], LINKED, "a linked image is left as it was");
    let found = refs(&events);
    assert_eq!(found.len(), 2);
    assert_eq!((found[0].event, found[0].image, found[1].event, found[1].image), (at, 0, at, 1));
    assert_eq!(found[0].generation, found[1].generation);
    assert_eq!(ctx.image(found[0]).expect("the first image"), Image { content_type: "image/png".to_string(), bytes: b"png bytes".to_vec() });
    assert_eq!(ctx.image(found[1]).expect("the second image"), Image { content_type: "image/jpeg".to_string(), bytes: b"jpeg bytes".to_vec() });
}

#[test]
fn only_the_four_newest_sessions_images_are_kept() {
    let dir = TempDir::new("load-session-four");
    let ctx = common::open_simple(&dir, None);
    let path = session_with_images(&dir, "s1", &[("image/png", b"png bytes")]);

    let loads: Vec<ImageRef> = (0..IMAGE_SETS + 1).map(|_| refs(&load(&ctx, &path))[0]).collect();

    assert_eq!(IMAGE_SETS, 4);
    assert!(loads.windows(2).all(|w| w[0].generation < w[1].generation), "{loads:?}");
    let oldest = ctx.image(loads[0]);
    assert!(matches!(oldest, Err(OpError::NotFound(_))), "{oldest:?}");
    for kept in &loads[1..] {
        assert_eq!(ctx.image(*kept).expect("a kept image").bytes, b"png bytes");
    }
}

#[test]
fn a_stale_generation_or_an_index_out_of_range_is_not_found_and_names_no_path() {
    let dir = TempDir::new("load-session-not-found");
    let ctx = common::open_simple(&dir, None);
    let path = session_with_images(&dir, "s1", &[("image/png", b"png bytes")]);
    let good = refs(&load(&ctx, &path))[0];

    let misses = [
        ImageRef { generation: good.generation + 100, ..good },
        ImageRef { event: good.event + 100, ..good },
        ImageRef { image: good.image + 100, ..good },
        // The linked image, which isn't served.
        ImageRef { image: 1, ..good },
        // An event with no images.
        ImageRef { event: good.event + 1, image: 0, ..good },
    ];
    for miss in misses {
        let answer = ctx.image(miss);
        let Err(OpError::NotFound(message)) = answer else { panic!("{miss:?}: {answer:?}") };
        assert!(!message.contains(&*dir.to_string_lossy()), "{message}");
    }
}

#[test]
fn an_image_lookup_never_returns_another_sessions_bytes() {
    let dir = TempDir::new("load-session-own-bytes");
    let ctx = common::open_simple(&dir, None);
    let first = session_with_images(&dir, "s1", &[("image/png", b"from the first session")]);
    let second = session_with_images(&dir, "s2", &[("image/gif", b"from the second session")]);

    let a = refs(&load(&ctx, &first))[0];
    let b = refs(&load(&ctx, &second))[0];

    assert_ne!(a.generation, b.generation);
    assert_eq!(ctx.image(a).expect("the first session's image"), Image { content_type: "image/png".to_string(), bytes: b"from the first session".to_vec() });
    assert_eq!(ctx.image(b).expect("the second session's image"), Image { content_type: "image/gif".to_string(), bytes: b"from the second session".to_vec() });
}
