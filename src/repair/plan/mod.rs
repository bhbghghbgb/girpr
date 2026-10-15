//! The work list: which files the run will touch, in what order, and where each
//! one's chunks are downloaded from (docs/02 step 4) — plus the purge
//! candidates the run would delete (docs/02 steps 5 and 7).
//!
//! Both are **plans**: they decide, they do not act. As of docs/05 M0 the run
//! builds both of them before it mutates anything, so "what will this run do?"
//! is answerable at one point in the pipeline — which is what the scope-of-work
//! ceilings ([`docs/04` B1/B2/B4]) and `--print-plan` (C1) are written against.
//! [`purge`] owns the cleanup half; the repair half is this module.
//!
//! The repair plan is derived from the **latest** manifest alone, which is what
//! makes any-version → latest work: the local manifest is only ever an
//! optimization (chunk reuse), never an input to correctness.

pub mod purge;

use std::collections::{HashMap, HashSet};

use crate::sophon::{SophonChunk, SophonChunkFile};

pub use purge::{PurgeCandidate, PurgePlan};

use super::manifest::LocalChunkMap;
use super::scope::PathScope;

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

    /// `Σ compressed_size` over every planned file, against a local reuse map.
    ///
    /// This is a **lower bound, not a promise**: `try_reuse_slice` falls back
    /// to a download whenever a local slice fails its MD5, so a file estimated
    /// at 0 bytes can still download its whole body. It is what
    /// `--max-download-bytes` refuses on, and the runtime hard stop is what
    /// actually enforces — see docs/04 §3 B2.
    pub fn est_download_bytes(&self, local_map: &LocalChunkMap) -> u64 {
        self.files
            .iter()
            .map(|f| est_file_download_bytes(f, local_map.get(&f.rel)))
            .sum()
    }
}

/// Per-file download estimate: `Σ c.compressed_size for c in plan[rel].chunks
/// where (c.uncompressed_md5, c.uncompressed_size) ∉ local_map[rel]`.
///
/// This is character-for-character `gircap`'s `patch_fetch` formula
/// (`docs/04` §3 B2), so the `PLAN` line, the `SUMMARY plan_download_bytes` key
/// and the capture's `totals.download_bytes` are the same figure. With no local
/// map (a fresh install) it degenerates to "every chunk downloads", which is
/// the honest worst case.
pub fn est_file_download_bytes(file: &PlannedFile, reuse: Option<&Vec<(String, i64, i64)>>) -> u64 {
    file.chunks
        .iter()
        .filter(|c| match reuse {
            Some(entries) => !entries.iter().any(|(md5, sz, _)| {
                md5.eq_ignore_ascii_case(&c.uncompressed_md5) && *sz == c.uncompressed_size
            }),
            None => true,
        })
        .map(|c| c.compressed_size.max(0) as u64)
        .sum()
}

/// Build sorted, deduped, blacklist-filtered, scope-filtered plan.
/// `per_manifest`: (chunk_url_prefix, files).
///
/// A path outside `scope` is dropped **here**, before the plan exists, so
/// `files_total`, the repair loop and `expected_paths()` all see the same
/// narrowed world. Dropping it later instead would leave the purge with a
/// *narrower* keep-set than an unfiltered run (docs/04 §2).
pub fn build_plan(
    per_manifest: Vec<(String, Vec<SophonChunkFile>)>,
    blacklist: &HashSet<String>,
    latest_tag: &str,
    scope: &PathScope,
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
            if blacklist.contains(&rel) || !scope.is_visible(&rel) || !seen.insert(rel.clone()) {
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
        let plan = build_plan(per, &bl, "5.0", &PathScope::default());
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
        let plan = build_plan(per, &HashSet::new(), "5.0", &PathScope::default());
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].rel, "a/b.dat");
        // First manifest wins the prefix.
        assert_eq!(plan.url_prefix_by_file.get("a/b.dat").unwrap(), "http://x");
        assert_eq!(plan.expected_paths().len(), 1);
    }

    /// The scope is applied *before* the plan exists, so `expected_paths()` —
    /// the only manifest-derived input to the files-cleanup's keep set — is
    /// narrowed too. Applying it anywhere downstream instead would leave the
    /// cleanup with a narrower keep set than an unfiltered run.
    #[test]
    fn out_of_scope_paths_never_reach_the_plan() {
        let files = vec![
            SophonChunkFile {
                file: "game/foo.dat".into(),
                chunks: vec![],
                is_folder: false,
                size: 1,
                md5: "m1".into(),
            },
            SophonChunkFile {
                file: "game/bar.dat".into(),
                chunks: vec![],
                is_folder: false,
                size: 1,
                md5: "m2".into(),
            },
        ];
        let per = vec![("http://x".to_string(), files)];
        let inc = vec!["game/foo.dat".to_string()];
        let plan = build_plan(
            per,
            &HashSet::new(),
            "5.0",
            &PathScope::new(&inc, &[]).unwrap(),
        );
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].rel, "game/foo.dat");
        assert_eq!(plan.expected_paths(), ["game/foo.dat".to_string()].into());
        assert!(
            !plan.url_prefix_by_file.contains_key("game/bar.dat"),
            "a dropped path must not keep a prefix entry either"
        );
    }
}
