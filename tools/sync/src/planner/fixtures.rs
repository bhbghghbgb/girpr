//! Fixtures shared by this module's unit tests.
//!
//! The three sibling test modules — [`super::stat_trust`], [`super::pairs`] and
//! [`super::coverage`] — all drive [`crate::planner::plan_pairs`] and therefore all
//! need the same handful of entries. They live here rather than in one of the three
//! so that no test module owns a fixture the others depend on.
//!
//! Compiled only under `cfg(test)`; `#![allow(dead_code)]` because a given test
//! binary uses a subset, exactly as `tests/common/mod.rs` does for the integration
//! suite.

#![allow(dead_code)]

use anyhow::Result;
use std::collections::HashMap;

use crate::effective::{SideCapability, SideEntry};

use super::HashMode;
use super::pairs::{CasePairing, PairPlan, SideRequest, plan_pairs};
use super::stat_trust::StatTrust;

/// A file entry with an empty cache row.
pub(super) fn file(size: u64, mtime: i64) -> SideEntry {
    SideEntry {
        kind: "file".into(),
        size,
        mtime_ns: mtime,
        cached: HashMap::new(),
        fresh: false,
    }
}

/// A directory entry: presence only, never a digest.
pub(super) fn dir() -> SideEntry {
    SideEntry {
        kind: "dir".into(),
        size: 0,
        mtime_ns: 0,
        cached: HashMap::new(),
        fresh: true,
    }
}

/// A stat-equal file whose cache already holds `cached`.
pub(super) fn cached_entry(cached: &[&str]) -> SideEntry {
    SideEntry {
        kind: "file".into(),
        size: 10,
        mtime_ns: 100,
        cached: cached
            .iter()
            .map(|a| (a.to_string(), vec![1u8; 16]))
            .collect(),
        fresh: true,
    }
}

/// Build a side map from `(rel, entry)` pairs.
pub(super) fn map(entries: Vec<(&str, SideEntry)>) -> HashMap<String, SideEntry> {
    entries
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

/// Which side is a record, which algorithms were asked for, and the trust of
/// each side — everything a case needs to vary. Named rather than a long
/// argument list so a coverage case reads as "a record on the src side".
#[derive(Clone, Copy)]
pub(super) struct Shape {
    /// (src can hash from disk, dst can)
    pub hashable: (bool, bool),
    pub algos: &'static [&'static str],
    /// (no_trust_src, no_trust_dst)
    pub no_trust: (bool, bool),
    /// Which answerability rule the run asked for. Defaults to all-of, because
    /// that is the default and these cases are about coverage rather than the
    /// choice between the two.
    pub mode: HashMode,
}

impl Shape {
    /// Both sides folders: nothing can be short of coverage, which is what most
    /// laziness cases need and what makes `sync` safe to share this check.
    pub(super) const FOLDERS: Shape = Shape {
        hashable: (true, true),
        algos: &["md5"],
        no_trust: (false, false),
        mode: HashMode::AllOf,
    };

    /// A record on the src side. The common coverage shape: the record cannot
    /// fill a gap and the folder can.
    pub(super) fn record_src(algos: &'static [&'static str]) -> Shape {
        Shape {
            hashable: (false, true),
            algos,
            no_trust: (false, false),
            mode: HashMode::AllOf,
        }
    }

    /// Both sides records, for the case where either may be the short one.
    pub(super) fn both_records(algos: &'static [&'static str]) -> Shape {
        Shape {
            hashable: (false, false),
            algos,
            no_trust: (false, false),
            mode: HashMode::AllOf,
        }
    }

    /// Same, with each side's distrust set independently.
    pub(super) fn distrusting(mut self, src: bool, dst: bool) -> Shape {
        self.no_trust = (src, dst);
        self
    }

    pub(super) fn run(
        &self,
        s: &HashMap<String, SideEntry>,
        d: &HashMap<String, SideEntry>,
        sensitive: bool,
        stat: StatTrust,
    ) -> Result<PairPlan> {
        let cap = |ok: bool| SideCapability {
            can_hash_from_disk: ok,
            can_write_cache: ok,
        };
        let algos: Vec<String> = self.algos.iter().map(|a| a.to_string()).collect();
        plan_pairs(
            SideRequest {
                entries: s,
                algos: &algos,
                no_trust: self.no_trust.0,
                cap: cap(self.hashable.0),
                label: "src",
            },
            SideRequest {
                entries: d,
                algos: &algos,
                no_trust: self.no_trust.1,
                cap: cap(self.hashable.1),
                label: "dst",
            },
            CasePairing::from(sensitive),
            self.mode,
            stat,
        )
    }
}

/// The everyday case: both sides folders, `md5`, both trusting (or both not).
pub(super) fn plan(
    s: &HashMap<String, SideEntry>,
    d: &HashMap<String, SideEntry>,
    sensitive: bool,
    no_trust: bool,
) -> PairPlan {
    Shape::FOLDERS
        .distrusting(no_trust, no_trust)
        .run(s, d, sensitive, StatTrust::default())
        .unwrap()
}
