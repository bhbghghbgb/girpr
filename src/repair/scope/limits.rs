//! Resource ceilings. See
//! [`docs/04-scope-limitation-spec.md`](../../../../docs/04-scope-limitation-spec.md).
//!
//! **Every ceiling here is an enforcement bound, not a selector.** A run whose
//! plan exceeds one *refuses before it does the work*. There is deliberately no
//! admission: "take the first N" / "take the cheapest N" is a selection, not an
//! enforcement, and it wants its own name.
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
//! [`ledger`](super::ledger).

use crate::repair::plan::{PurgePlan, RepairPlan};

/// What the pre-flight predicates are allowed to look at.
///
/// A struct rather than a three-argument call, because the inputs grow with
/// every ceiling and the caller should not have to remember which is which.
pub struct Preflight<'a> {
    /// The (already scope-filtered) repair plan.
    pub plan: &'a RepairPlan,
    /// Download estimate for `plan`, already computed against the local reuse
    /// map — see [`RepairPlan::est_download_bytes`]. Passed in rather than
    /// recomputed so the number the ceiling is checked against is the same
    /// number the `PLAN` line and `SUMMARY plan_download_bytes` report.
    pub est_download_bytes: u64,
    /// The purge plan, when a purge flag is set and this is not `--check-only`.
    /// `None` means "nothing is a delete candidate", which any delete ceiling
    /// trivially satisfies.
    pub purge: Option<&'a PurgePlan>,
}

/// The manually requested ceilings. `None` = not requested, which is what keeps
/// an unflagged run byte-identical to an un-limited one.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// `--max-files`: planned file count.
    pub files: Option<u64>,
    /// `--max-download-bytes`: estimated chunk downloads for the plan.
    pub download_bytes: Option<u64>,
    /// `--max-write-bytes`: cumulative bytes written into `*_tmp`.
    pub write_bytes: Option<u64>,
    /// `--max-ram-bytes`: the staging budget for the in-place repair.
    ///
    /// Not a pre-flight refusal: a file that cannot get its reservation is
    /// repaired by the ordered path instead, so the run does not fail — the
    /// same shape as `write_bytes`, which is also a runtime bound.
    pub ram_bytes: Option<u64>,
    /// `--max-delete-bytes`: bytes the files-cleanup would delete.
    pub delete_bytes: Option<u64>,
}

impl Limits {
    /// `true` when any ceiling was requested → `SUMMARY limited=1`.
    pub fn is_active(&self) -> bool {
        self.files.is_some()
            || self.download_bytes.is_some()
            || self.write_bytes.is_some()
            || self.ram_bytes.is_some()
            || self.delete_bytes.is_some()
    }

    /// Every tripped **pre-flight** ceiling, in flag order.
    ///
    /// Checked against the **filtered** plan, never the raw one: `--max-files N`
    /// must mean "the files the scope selected", so filtering first is what
    /// makes the flag usable at all instead of refusing on every real install.
    pub fn check_preflight(&self, pf: Preflight<'_>) -> Vec<LimitHit> {
        let mut hits = Vec::new();
        if let Some(limit) = self.files {
            let actual = pf.plan.files.len() as u64;
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
            let actual = pf.est_download_bytes;
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
            let actual = pf.purge.map_or(0, |p| p.total_bytes);
            if actual > limit {
                hits.push(delete_hit(limit, actual));
            }
        }
        hits
    }

    /// The delete ceiling against **one** purge plan.
    ///
    /// Used twice, and the distinction matters. The pre-flight
    /// [`Self::check_preflight`] call sees the up-front plan, which is the union
    /// of both passes for everything that exists when the run starts. This one
    /// is for the post-repair re-plan, whose candidates include `*_tmp` files
    /// the repair created and which therefore did not exist at assert time —
    /// they are only countable once the repair is done. Deletion is the one
    /// irreversible act in the tool, so both passes are checked rather than
    /// assumed.
    pub fn check_delete(&self, purge: &PurgePlan) -> Option<LimitHit> {
        let limit = self.delete_bytes?;
        (purge.total_bytes > limit).then(|| delete_hit(limit, purge.total_bytes))
    }
}

fn delete_hit(limit: u64, actual: u64) -> LimitHit {
    LimitHit {
        what: "--max-delete-bytes",
        key: "purge_bytes",
        actual,
        limit,
    }
}

/// One tripped ceiling: what was requested, what the plan actually wanted, and
/// the gap. The shape is lifted from `gircap`'s budget for the `describe()`
/// habit, not for the code — the crates share nothing by design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitHit {
    /// The flag as the user typed it, so the message is actionable.
    pub what: &'static str,
    /// Metric name, from the `SUMMARY` vocabulary.
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
            verdicts: None,
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

    /// The pre-flight context a test most often wants: a plan, a zero download
    /// estimate, and no purge.
    fn pf<'a>(plan: &'a RepairPlan) -> Preflight<'a> {
        Preflight {
            plan,
            est_download_bytes: 0,
            purge: None,
        }
    }

    fn pf_full<'a>(plan: &'a RepairPlan, est: u64, purge: &'a PurgePlan) -> Preflight<'a> {
        Preflight {
            plan,
            est_download_bytes: est,
            purge: Some(purge),
        }
    }

    #[test]
    fn no_limits_means_nothing_is_active_and_nothing_trips() {
        let l = Limits::default();
        let p = plan(3_000);
        let purge = purge(10_000);
        assert!(!l.is_active());
        assert!(l.check_preflight(pf_full(&p, 1 << 30, &purge)).is_empty());
    }

    #[test]
    fn the_boundary_does_not_trip() {
        // "> N" not ">= N": a run that exactly fits must be allowed.
        let l = Limits {
            files: Some(8),
            ..Default::default()
        };
        let p8 = plan(8);
        let p9 = plan(9);
        assert!(l.check_preflight(pf(&p8)).is_empty());
        assert_eq!(l.check_preflight(pf(&p9)).len(), 1);
    }

    #[test]
    fn max_files_is_checked_against_the_plan_not_the_manifest() {
        let l = Limits {
            files: Some(0),
            ..Default::default()
        };
        let p0 = plan(0);
        let p1 = plan(1);
        assert_eq!(l.check_preflight(pf(&p0)).len(), 0);
        let hits = l.check_preflight(pf(&p1));
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
        let p = plan(1);
        assert!(l.check_preflight(pf(&p)).is_empty());
        assert_eq!(l.check_preflight(pf_full(&p, 0, &purge(11))).len(), 1);
        assert!(l.check_preflight(pf_full(&p, 0, &purge(10))).is_empty());
    }

    #[test]
    fn download_bytes_uses_the_estimate_it_was_given() {
        // The estimate arrives pre-computed from the local reuse map, so the
        // ceiling, the PLAN line and SUMMARY all quote one number.
        let l = Limits {
            download_bytes: Some(100),
            ..Default::default()
        };
        let p = plan(3);
        assert!(l.check_preflight(pf_full(&p, 100, &purge(0))).is_empty());
        let hits = l.check_preflight(pf_full(&p, 101, &purge(0)));
        assert_eq!(hits[0].key, "plan_download_bytes");
        assert_eq!((hits[0].actual, hits[0].limit), (101, 100));
    }

    #[test]
    fn every_tripped_ceiling_is_reported_not_just_the_first() {
        let l = Limits {
            files: Some(1),
            download_bytes: Some(0),
            delete_bytes: Some(1),
            write_bytes: None,
            ram_bytes: None,
        };
        let p = plan(5);
        let hits = l.check_preflight(pf_full(&p, 7, &purge(9)));
        let names: Vec<&str> = hits.iter().map(|h| h.what).collect();
        assert_eq!(
            names,
            vec!["--max-files", "--max-download-bytes", "--max-delete-bytes"]
        );
    }

    #[test]
    fn check_delete_is_the_same_predicate_the_preflight_uses() {
        // The post-repair pass re-checks the ceiling against its own re-plan.
        // That re-plan can only shrink today (a successful repair promotes every
        // `_tmp` it creates), so there is no integration case that can drive
        // this branch — the predicate itself is pinned here instead, and
        // `post::run` carries the reasoning in a comment.
        let l = Limits {
            delete_bytes: Some(10),
            ..Default::default()
        };
        assert!(l.check_delete(&purge(10)).is_none(), "the boundary passes");
        let hit = l.check_delete(&purge(11)).expect("11 > 10");
        assert_eq!(hit.what, "--max-delete-bytes");
        assert_eq!(hit.key, "purge_bytes");
        assert_eq!((hit.actual, hit.limit), (11, 10));
        // Same verdict as the pre-flight form, on the same plan.
        let p = plan(1);
        assert_eq!(l.check_preflight(pf_full(&p, 0, &purge(11))).len(), 1);
        // No flag → no check at all.
        assert!(Limits::default().check_delete(&purge(1 << 30)).is_none());
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
                ram_bytes: Some(1),
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
