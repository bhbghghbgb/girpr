//! The logical→on-disk path map, and the store that builds it.
//!
//! **L1.** A Windows device name is the one path component that cannot be
//! written as itself: `CON`, `NUL`, `COM1` and their extensions open a device,
//! not a file. Refusing the value keeps the bundle safe but throws away the
//! observation — and a live manifest that names `CON` is a fact about the API.
//!
//! So a device component is **escaped on disk** and **preserved in the
//! documents**. `entries[].file`, `patchmaps/*.json`'s `path` and the index's
//! references keep the game's spelling; `game/` and `expected/` hold the escaped
//! name; and this document records every pair. An external reader follows the
//! mapping rather than re-deriving it, and [`crate::paths::escape_path`] is the
//! same rule the writer used.
//!
//! # Why a run-wide store
//!
//! The mapping is known at write time, in more than one module: a manifest's
//! tag directory ([`crate::manifest::dump`]) and each assembled file
//! ([`crate::emit::gamedir`]). The document is written once, after both, so the
//! writers record into [`PathMap`] and the run asks it for [`PathMap::document`].
//!
//! **Recorded only when they differ.** A bundle with no device name writes an
//! empty `entries` list, which is the honest statement "nothing needed
//! escaping" — the same reason `diagnostics/rejections.json` is always written.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// The artifact-relative name of the mapping document.
pub const DOCUMENT: &str = "diagnostics/path_map.json";

/// One logical path and the file it was written to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathMapEntry {
    /// The path as the documents spell it: the game's own path, or a manifest
    /// tag directory.
    pub logical: String,
    /// The artifact-relative path actually written.
    pub on_disk: String,
}

/// `diagnostics/path_map.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathMapDocument {
    /// The artifact schema version.
    pub schema_version: u32,
    /// Every path written under a different name, sorted by `logical` so two
    /// captures of one input write the same document.
    pub entries: Vec<PathMapEntry>,
}

impl PathMapDocument {
    /// The on-disk path for `logical`, if it was escaped.
    pub fn resolve(&self, logical: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.logical == logical)
            .map(|e| e.on_disk.as_str())
    }

    /// Whether nothing needed escaping.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The run-wide set of paths that were written under an escaped name.
#[derive(Debug, Default)]
pub struct PathMap {
    entries: Mutex<BTreeMap<String, String>>,
}

impl PathMap {
    /// An empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Escape `logical`, record the pair when it differs, and return the
    /// on-disk path.
    pub fn escape(&self, logical: &str) -> String {
        let on_disk = crate::paths::escape_path(logical);
        self.record(logical, &on_disk);
        on_disk
    }

    /// Record a pair the caller already computed.
    ///
    /// Used where the on-disk name comes from a writer that owns the escaping
    /// rule, so the mapping and the write cannot disagree.
    pub fn record(&self, logical: &str, on_disk: &str) {
        if logical == on_disk {
            return;
        }
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.insert(logical.to_string(), on_disk.to_string());
    }

    /// How many paths were escaped.
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Whether nothing needed escaping.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The document to write.
    pub fn document(&self) -> PathMapDocument {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        PathMapDocument {
            schema_version: crate::SCHEMA_VERSION,
            entries: entries
                .iter()
                .map(|(logical, on_disk)| PathMapEntry {
                    logical: logical.clone(),
                    on_disk: on_disk.clone(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_component_is_recorded_with_its_logical_spelling() {
        let map = PathMap::new();
        assert_eq!(map.escape("game/a/CON"), "game/a/%CON");
        assert_eq!(map.escape("game/a/b"), "game/a/b");
        let doc = map.document();
        assert_eq!(doc.schema_version, crate::SCHEMA_VERSION);
        assert_eq!(
            doc.entries,
            vec![PathMapEntry {
                logical: "game/a/CON".into(),
                on_disk: "game/a/%CON".into(),
            }]
        );
        assert_eq!(doc.resolve("game/a/CON"), Some("game/a/%CON"));
        assert_eq!(doc.resolve("game/a/b"), None);
        assert_eq!(map.len(), 1);
        assert!(!map.is_empty());
    }

    #[test]
    fn the_document_sorts_so_two_runs_agree() {
        let a = PathMap::new();
        a.escape("game/z/CON");
        a.escape("expected/a/NUL");
        let b = PathMap::new();
        b.escape("expected/a/NUL");
        b.escape("game/z/CON");
        assert_eq!(a.document(), b.document());
        let doc = a.document();
        let logical: Vec<&str> = doc.entries.iter().map(|e| e.logical.as_str()).collect();
        assert_eq!(logical, vec!["expected/a/NUL", "game/z/CON"]);
    }

    #[test]
    fn an_unescaped_bundle_writes_an_empty_document() {
        let map = PathMap::new();
        assert_eq!(map.escape("game/a/b"), "game/a/b");
        let doc = map.document();
        assert!(doc.entries.is_empty());
        assert!(doc.is_empty());
        assert_eq!(doc.schema_version, crate::SCHEMA_VERSION);
    }
}
