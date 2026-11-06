pub mod annotations;
pub mod bundle;
pub mod generate;
pub mod index;
pub mod model;
pub mod parse;
pub mod reports;

#[doc(hidden)]
pub mod test_support {
    pub use crate::annotations;
    pub use crate::bundle;
    pub use crate::generate;
    pub use crate::index;
    pub use crate::model;
    pub use crate::model::SessionRow as Row;
    pub use crate::parse;
    pub use crate::reports;

    /// Parses and indexes one file, returning its index row and a compact
    /// "kind:text" rendering of its events ("!" marks errors).
    pub fn index_one(path: &str) -> (Row, Vec<String>) {
        let p = std::path::Path::new(path);
        let source = crate::parse::detect(p).expect("detect");
        let parsed = crate::parse::parse_file(p, source).expect("parse");
        let events = parsed
            .events
            .iter()
            .map(|e| {
                let kind = serde_json::to_value(e.kind).unwrap();
                // "!" marks an error, "~" a failing shell exit.
                let mark = if e.is_error { "!" } else if e.failed_exit { "~" } else { "" };
                format!("{}{}:{}", kind.as_str().unwrap(), mark, e.text)
            })
            .collect();
        let (row, _) = crate::index::index_parsed(path, source, parsed).expect("row");
        (row, events)
    }

    /// Whether the file becomes a row in the session list.
    pub fn is_listed(path: &str) -> bool {
        let p = std::path::Path::new(path);
        let source = crate::parse::detect(p).expect("detect");
        let parsed = crate::parse::parse_file(p, source).expect("parse");
        crate::index::index_parsed(path, source, parsed).is_some()
    }

    /// Renders one pi branch as "kind:text" strings.
    pub fn load_branch(path: &str, leaf: &str) -> Vec<String> {
        crate::parse::pi::parse_branch(std::path::Path::new(path), Some(leaf))
            .expect("parse")
            .events
            .iter()
            .map(|e| format!("{}:{}", serde_json::to_value(e.kind).unwrap().as_str().unwrap(), e.text))
            .collect()
    }
}
