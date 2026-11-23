//! Saving a file the page made (an export bundle, a report's Markdown) where the user chooses. The
//! shell shows the save dialog itself, so the page names only a suggested file name and never a
//! path, and the file always gets the extension of what it is.

use std::path::PathBuf;

/// What the dialog offers as the file name: the last part of `name` (the page may send only a
/// name, never a folder), with `extension` forced on. A name with nothing left becomes `Untitled`.
pub fn suggested_name(name: &str, extension: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    let last = if last.is_empty() || last == "." || last == ".." { "Untitled" } else { last };
    force_extension(PathBuf::from(last), extension).to_string_lossy().into_owned()
}

/// `path` with `.{extension}` added unless it already ends in it (in any case).
pub fn force_extension(mut path: PathBuf, extension: &str) -> PathBuf {
    if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case(extension)) {
        path.as_mut_os_string().push(format!(".{extension}"));
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn force_extension_adds_the_extension_unless_the_path_already_ends_in_it() {
        let cases = [
            ("/Users/dan/export", "/Users/dan/export.spsession"),
            ("/Users/dan/export.zip", "/Users/dan/export.zip.spsession"),
            ("/Users/dan/export.spsession", "/Users/dan/export.spsession"),
            ("/Users/dan/export.SPSESSION", "/Users/dan/export.SPSESSION"),
            ("/Users/dan/export.spsession.txt", "/Users/dan/export.spsession.txt.spsession"),
            ("/Users/dan/.spsession", "/Users/dan/.spsession.spsession"),
        ];
        for (chosen, saved) in cases {
            assert_eq!(force_extension(PathBuf::from(chosen), "spsession"), PathBuf::from(saved), "{chosen}");
        }
        assert_eq!(force_extension(PathBuf::from("/tmp/My report"), "md"), PathBuf::from("/tmp/My report.md"));
        assert_eq!(force_extension(PathBuf::from("/tmp/My report.MD"), "md"), PathBuf::from("/tmp/My report.MD"));
    }

    #[test]
    fn suggested_name_keeps_only_a_file_name_and_forces_the_extension() {
        let cases = [
            ("Fix the login bug.spsession", "Fix the login bug.spsession"),
            ("Fix the login bug", "Fix the login bug.spsession"),
            ("../../etc/passwd", "passwd.spsession"),
            ("/Users/dan/.ssh/authorized_keys", "authorized_keys.spsession"),
            ("folder\\name", "name.spsession"),
            ("  ", "Untitled.spsession"),
            ("", "Untitled.spsession"),
            ("..", "Untitled.spsession"),
            ("a/", "Untitled.spsession"),
        ];
        for (name, suggested) in cases {
            assert_eq!(suggested_name(name, "spsession"), suggested, "{name:?}");
        }
        assert_eq!(suggested_name("Weekly report", "md"), "Weekly report.md");
    }

    #[test]
    fn a_suggested_name_is_never_a_path() {
        for name in ["../x", "/abs/x", "a/b/c", "..\\x", "x/.."] {
            let suggested = suggested_name(name, "md");
            assert_eq!(Path::new(&suggested).components().count(), 1, "{name:?} -> {suggested:?}");
            assert!(!suggested.contains(['/', '\\']), "{name:?} -> {suggested:?}");
        }
    }
}
