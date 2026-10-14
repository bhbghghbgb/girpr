//! Resource ceilings (docs/04 Group B).
//!
//! **Every ceiling here is an enforcement bound, not a selector.** A run whose
//! plan exceeds one *refuses before it does the work*. There is deliberately no
//! admission: "take the first N" / "take the cheapest N" is a selection, not an
//! enforcement, and it wants its own name ([`04` §1.5](04-scope-limitation-options.md)).
//!
//! That reframing is what makes the two tiers complementary rather than
//! redundant: `--include` **selects**, a ceiling **verifies the selection**. With
//! `--include "<the 8 you meant>" --max-files 8`, a correct glob repairs 8 files
//! and exits `0`, and a typo exits `5` having repaired nothing — instead of
//! quietly repairing 8 *wrong* files and exiting `0`.
//!
//! Two shapes of enforcement exist, and they are not interchangeable:
//!
//! | shape | when | suits |
//! |---|---|---|
//! | **pre-flight refusal** | after the plan exists, before any mutation | anything predictable from the plan: file count, download bytes, delete bytes |
//! | **runtime hard stop** | mid-flight, when a counter crosses the line | anything not predictable from the plan: bytes written into `*_tmp` |
//!
//! Both produce exit `5`. Both are implemented here as pure predicates over a
//! finished plan; the impure side (the ledger the runtime stops read) lives in
//! [`ledger`] from M4/M6.

use crate::repair::plan::{PurgePlan, RepairPlan};

/// The manually requested ceilings. `None` = not requested, which is what keeps
/// an unflagged run byte-identical to an un-limited one.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// `--max-files`: planned file count.
    pub files: Option<u64>,
    /// `--max-download-bytes`: estimated chunk downloads for the plan.
    pub download_bytes: Option<u64>,
    /// `--max-write-bytes`: cumulative bytes written into `*_tmp` (M6).
    pub write_bytes: Option<u64>,
    /// `--max-delete-bytes`: bytes the files-cleanup would delete.
    pub delete_bytes: Option<u64>,
}

impl Limits {
    /// `true` when any ceiling was requested → `SUMMARY limited=1`.
    pub fn is_active(&self) -> bool {
        self.files.is_some()
            || self.download_bytes.is_some()
            || self.write_bytes.is_some()
            || self.delete_bytes.is_some()
    }

    /// Every tripped **pre-flight** ceiling, in flag order.
    ///
    /// Checked against the **filtered** plan, never the raw one: `--max-files N`
    /// must mean "the files the scope selected", so filtering first is what
    /// makes the flag usable at all instead of refusing on every real install.
    pub fn check_preflight(&self, plan: &RepairPlan, purge: Option<&PurgePlan>) -> Vec<LimitHit> {
        let mut hits = Vec::new();
        if let Some(limit) = self.files {
            let actual = plan.files.len() as u64;
            if actual > limit {
                hits.push(LimitHit {
                    what: "--max-files",
                    key: "plan_files",
                    actual,
                    limit,
                });
            }
        }
        if let Some(limit) = self.download_bytes {
            let actual = plan.est_download_bytes();
            if actual > limit {
                hits.push(LimitHit {
                    what: "--max-download-bytes",
                    key: "plan_download_bytes",
                    actual,
                    limit,
                });
            }
        }
        if let Some(limit) = self.delete_bytes {
            let actual = purge.map_or(0, |p| p.total_bytes);
            if actual > limit {
                hits.push(LimitHit {
                    what: "--max-delete-bytes",
                    key: "purge_bytes",
                    actual,
                    limit,
                });
            }
        }
        hits
    }
}

/// One tripped ceiling: what was requested, what the plan actually wanted, and
/// the gap. The shape is lifted from `gircap`'s budget for the `describe()`
/// habit, not for the code — the crates share nothing by design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitHit {
    /// The flag as the user typed it, so the message is actionable.
    pub what: &'static str,
    /// Metric name, from the `SUMMARY` vocabulary of docs/04 §4.6.
    pub key: &'static str,
    pub actual: u64,
    pub limit: u64,
}

impl LimitHit {
    /// One line naming the flag, the shortfall, and the way out.
    pub fn describe(&self) -> String {
        format!(
            "{} exceeded: {}={} limit={} (refusing before doing any work; raise the limit or narrow --include)",
            self.what, self.key, self.actual, self.limit
        )
    }
}

impl std::fmt::Display for LimitHit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repair::plan::{PlannedFile, PurgeCandidate};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn plan(n: usize) -> RepairPlan {
        RepairPlan {
            latest: "9.9".into(),
            files: (0..n)
                .map(|i| PlannedFile {
                    rel: format!("f{i}.dat"),
                    size: 1,
                    md5: String::new(),
                    chunks: vec![],
                })
                .collect(),
            url_prefix_by_file: HashMap::new(),
        }
    }

    fn purge(total: u64) -> PurgePlan {
        PurgePlan {
            files: vec![PurgeCandidate {
                rel: "orphan.dat".into(),
                path: PathBuf::from("orphan.dat"),
                bytes: total,
            }],
            dirs: vec![],
            total_bytes: total,
            hidden: 0,
        }
    }

    #[test]
    fn no_limits_means_nothing_is_active_and_nothing_trips() {
        let l = Limits::default();
        assert!(!l.is_active());
        assert!(
            l.check_preflight(&plan(3_000), Some(&purge(10_000)))
                .is_empty()
        );
    }

    #[test]
    fn the_boundary_does_not_trip() {
        // "> N" not ">= N": a run that exactly fits must be allowed.
        let l = Limits {
            files: Some(8),
            ..Default::default()
        };
        assert!(l.check_preflight(&plan(8), None).is_empty());
        assert_eq!(l.check_preflight(&plan(9), None).len(), 1);
    }

    #[test]
    fn max_files_is_checked_against_the_plan_not_the_manifest() {
        let l = Limits {
            files: Some(0),
            ..Default::default()
        };
        assert_eq!(l.check_preflight(&plan(0), None).len(), 0);
        let hits = l.check_preflight(&plan(1), None);
        assert_eq!(hits[0].what, "--max-files");
        assert_eq!((hits[0].actual, hits[0].limit), (1, 0));
    }

    #[test]
    fn delete_bytes_is_zero_without_a_purge_plan() {
        // No purge flag → nothing is a delete candidate, so any ceiling holds.
        let l = Limits {
            delete_bytes: Some(10),
            ..Default::default()
        };
        assert!(l.check_preflight(&plan(1), None).is_empty());
        assert_eq!(l.check_preflight(&plan(1), Some(&purge(11))).len(), 1);
        assert!(l.check_preflight(&plan(1), Some(&purge(10))).is_empty());
    }

    #[test]
    fn every_tripped_ceiling_is_reported_not_just_the_first() {
        let l = Limits {
            files: Some(1),
            download_bytes: Some(0),
            delete_bytes: Some(1),
            write_bytes: None,
        };
        let mut p = plan(5);
        // Give one file a chunk so the estimate is non-zero.
        p.files[0].chunks.push(crate::sophon::SophonChunk {
            id: "c".into(),
            uncompressed_md5: "m".into(),
            offset: 0,
            compressed_size: 7,
            uncompressed_size: 7,
            unknown: 0,
            compressed_md5: String::new(),
        });
        let hits = l.check_preflight(&p, Some(&purge(9)));
        let names: Vec<&str> = hits.iter().map(|h| h.what).collect();
        assert_eq!(
            names,
            vec!["--max-files", "--max-download-bytes", "--max-delete-bytes"]
        );
    }

    #[test]
    fn a_hit_describes_itself_in_the_words_a_user_acted_on() {
        let hit = LimitHit {
            what: "--max-files",
            key: "plan_files",
            actual: 3000,
            limit: 8,
        };
        assert_eq!(
            hit.describe(),
            "--max-files exceeded: plan_files=3000 limit=8 \
             (refusing before doing any work; raise the limit or narrow --include)"
        );
        assert_eq!(hit.to_string(), hit.describe());
    }

    #[test]
    fn is_active_covers_every_flag() {
        for l in [
            Limits {
                files: Some(1),
                ..Default::default()
            },
            Limits {
                download_bytes: Some(1),
                ..Default::default()
            },
            Limits {
                write_bytes: Some(1),
                ..Default::default()
            },
            Limits {
                delete_bytes: Some(1),
                ..Default::default()
            },
        ] {
            assert!(l.is_active(), "{l:?}");
        }
    }
}
