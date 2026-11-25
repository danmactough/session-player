//! Putting the `sp` command that ships inside the app where a shell finds it.

use std::path::{Path, PathBuf};

/// The `sp` beside the app's own program: in Contents/MacOS in a release build, or in the
/// target folder, where `npm run app` builds it.
pub fn bundled() -> Option<PathBuf> {
    let sp = std::env::current_exe().ok()?.parent()?.join("sp");
    sp.is_file().then_some(sp)
}

/// Links `<home>/.local/bin/sp` to `bundled`, replacing what is there, and
/// returns the link's path. A link, not a copy, so the `sp` a shell runs is
/// always the one in the app as last updated.
#[cfg(unix)]
pub fn link(bundled: &Path, home: &Path) -> Result<PathBuf, String> {
    let dir = home.join(".local/bin");
    let dest = dir.join("sp");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    // Made beside the destination and renamed over it, so there is no moment with no sp there.
    let new = dir.join(".sp-installing");
    let _ = std::fs::remove_file(&new);
    std::os::unix::fs::symlink(bundled, &new).map_err(|e| format!("Couldn't write to {}: {e}", dir.display()))?;
    std::fs::rename(&new, &dest).map_err(|e| format!("Couldn't replace {}: {e}", dest.display()))?;
    Ok(dest)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Removes the test's folder however the test ends. (testutil/tempdir.rs can't be included
    /// from an inline module in this file: `#[path]` would resolve under a src/cli/ that doesn't exist.)
    struct Remove(PathBuf);

    impl Drop for Remove {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_link_points_at_the_apps_sp_and_replaces_an_older_copy() {
        let dir = std::env::temp_dir().join(format!("sp-cli-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _remove = Remove(dir.clone());
        let bundled = dir.join("Session Player.app/Contents/MacOS/sp");
        std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        std::fs::write(&bundled, "the app's sp").unwrap();
        let home = dir.join("home");

        let dest = link(&bundled, &home).unwrap();
        assert_eq!(dest, home.join(".local/bin/sp"));
        assert_eq!(std::fs::read_link(&dest).unwrap(), bundled);

        // An sp copied there by hand, as `sp install` does outside the app, gives way to the link.
        std::fs::remove_file(&dest).unwrap();
        std::fs::write(&dest, "an older copy").unwrap();
        link(&bundled, &home).unwrap();
        assert_eq!(std::fs::read_link(&dest).unwrap(), bundled);
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "the app's sp");
    }
}
