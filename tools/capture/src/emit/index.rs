//! The index documents: `MANIFEST.json`, `candidates.json`, `chunks/index.json`.
//!
//! **M8.** The chunk index is the join between every `(tag, file, chunk)` triple
//! and the single blob that satisfies all of them, including which of the three
//! retention sets each role belongs to.