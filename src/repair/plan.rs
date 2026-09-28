//! The work list: which files the run will touch, in what order, and where each
//! one's chunks are downloaded from (docs/02 step 4).
//!
//! The plan is derived from the **latest** manifest alone, which is what makes
//! any-version → latest work: the local manifest is only ever an optimization
//! (chunk reuse), never an input to correctness.

use std::collections::{HashMap, HashSet};

use crate::sophon::{SophonChunk, SophonChunkFile};

/// One target file, plus the chunk URL prefix its chunks live under.
pub struct PlannedFile {
    /// `/`-separated rel path, as it appears in the manifest.
    pub rel: String,
    pub size: i64,
    pub md5: String,
    pub chunks: Vec<SophonChunk>,
}

/// The sorted, deduped work list for one run.
pub struct RepairPlan {
    pub latest: String,
    pub files: Vec<PlannedFile>,
    pub url_prefix_by_file: HashMap<String, String>,
}

impl RepairPlan {
    /// Paths the files-cleanup must treat as expected (docs/02 step 7).
    pub fn expected_paths(&self) -> HashSet<String> {
        self.files.iter().map(|f| f.rel.clone()).collect()
    }
}

/// Build sorted, deduped, blacklist-filtered plan. `per_manifest`:
/// (chunk_url_prefix, files).
pub fn build_plan(
    per_manifest: Vec<(String, Vec<SophonChunkFile>)>,
    blacklist: &HashSet<String>,
    latest_tag: &str,
) -> RepairPlan {
    let mut files: Vec<PlannedFile> = Vec::new();
    let mut prefix: HashMap<String, String> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (url_prefix, list) in per_manifest {
        for f in list {
            // Folders are dropped: directories are created on demand instead.
            if f.is_folder || f.file.is_empty() {
                continue;
            }
            let rel = f.file.replace('\\', "/");
            if blacklist.contains(&rel) || !seen.insert(rel.clone()) {
                continue;
            }
            prefix.insert(rel.clone(), url_prefix.clone());
            files.push(PlannedFile {
                rel,
                size: f.size,
                md5: f.md5.clone(),
                chunks: f.chunks,
            });
        }
    }
    // Stable order so log lines and the parallel spawn order are reproducible.
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    RepairPlan {
        latest: latest_tag.to_string(),
        files,
        url_prefix_by_file: prefix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_sorted_dedup_blacklist() {
        let files = vec![
            SophonChunkFile {
                file: "b.dat".into(),
                chunks: vec![],
                is_folder: false,
                size: 2,
                md5: "m2".into(),
            },
            SophonChunkFile {
                file: "a.dat".into(),
                chunks: vec![],
                is_folder: false,
                size: 1,
                md5: "m1".into(),
            },
            SophonChunkFile {
                file: "drop.dat".into(),
                chunks: vec![],
                is_folder: false,
                size: 1,
                md5: "m".into(),
            },
        ];
        let per = vec![("http://x".to_string(), files)];
        let bl: HashSet<String> = ["drop.dat".to_string()].into_iter().collect();
        let plan = build_plan(per, &bl, "5.0");
        assert_eq!(
            plan.files
                .iter()
                .map(|f| f.rel.as_str())
                .collect::<Vec<_>>(),
            vec!["a.dat", "b.dat"]
        );
        assert_eq!(plan.latest, "5.0");
        assert_eq!(plan.url_prefix_by_file.get("a.dat").unwrap(), "http://x");
    }

    #[test]
    fn plan_dedups_across_manifests_and_drops_folders() {
        let dup = SophonChunkFile {
            file: "a\\b.dat".into(),
            chunks: vec![],
            is_folder: false,
            size: 1,
            md5: "m1".into(),
        };
        let folder = SophonChunkFile {
            file: "dir/".into(),
            chunks: vec![],
            is_folder: true,
            size: 0,
            md5: String::new(),
        };
        // Same path, backslash spelling, in a second manifest: one entry only.
        let per = vec![
            ("http://x".to_string(), vec![dup.clone()]),
            ("http://y".to_string(), vec![dup, folder]),
        ];
        let plan = build_plan(per, &HashSet::new(), "5.0");
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].rel, "a/b.dat");
        // First manifest wins the prefix.
        assert_eq!(plan.url_prefix_by_file.get("a/b.dat").unwrap(), "http://x");
        assert_eq!(plan.expected_paths().len(), 1);
    }
}
