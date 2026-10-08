//! Path-set diffing between two effective maps.

use std::collections::{HashMap, HashSet};

use crate::effective::EffRec;
use crate::planner::{Required, StatTrust};

/// Per-class diff buckets, each sorted by relative path.
///
/// Derives `PartialEq` so a test can state a whole expected verdict at once
/// rather than five fields one at a time; it has no interior state, so this
/// costs nothing.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Diff {
    /// src-only.
    pub missing: Vec<String>,
    /// dst-only.
    pub extra: Vec<String>,
    /// Present on both but size/mtime/hash differ.
    pub changed: Vec<String>,
    /// Same path, different kind (file vs dir).
    pub type_conflict: Vec<String>,
    /// Same path ignoring case but different casing (insensitive mode only).
    pub case_mismatch: Vec<(String, String)>,
}

impl Diff {
    /// Number of differences that force a nonzero exit, ignoring case mismatches
    /// (which `compare` counts separately but which are still a difference).
    pub fn total(&self) -> usize {
        self.missing.len() + self.extra.len() + self.changed.len() + self.type_conflict.len()
    }

    /// True when nothing at all differs.
    pub fn is_empty(&self) -> bool {
        self.total() == 0 && self.case_mismatch.is_empty()
    }
}

/// Compare two effective maps.
///
/// In insensitive mode keys are matched by lowercase, so a casing-only
/// difference surfaces as `case_mismatch` instead of missing+extra. Dirs are
/// compared by presence alone.
///
/// `required` is the planner's per-path answer to "which algorithms settle this
/// pair", keyed by **src** path. Passing it rather than an algorithm list is what
/// lets one run settle different pairs by different algorithms: under
/// [`crate::planner::HashMode::AllOf`] every entry is the whole requested list, so
/// this is a superset of what this function used to be handed, not a different
/// mechanism. Consulted only after stat has been consulted and found unconvincing,
/// which is the same boundary the planner used — so a path that reaches it always has
/// an entry.
///
/// `stat` decides **which stat fields may settle a pair here**, and it must be the same
/// value the planner was given. That is the load-bearing part: this function's
/// short-circuit on size+mtime and the planner's are the same rule, and if they
/// disagree then one of two silent things happens — the planner hashes a pair whose
/// digest is never read, or (worse) a pair the planner called decided turns out to need
/// a digest nobody computed. Sharing one predicate is what makes that unrepresentable
/// rather than merely tested-for.
pub fn diff_maps(
    src: &HashMap<String, EffRec>,
    dst: &HashMap<String, EffRec>,
    required: &Required,
    stat: StatTrust,
    case_sensitive: bool,
) -> Diff {
    let mut d = Diff::default();
    if case_sensitive {
        let mut keys: HashSet<&String> = HashSet::new();
        for k in src.keys().chain(dst.keys()) {
            keys.insert(k);
        }
        for k in keys {
            match (src.get(k), dst.get(k)) {
                (Some(_s), None) => {
                    d.missing.push(k.clone());
                }
                (None, Some(_)) => {
                    d.extra.push(k.clone());
                }
                (Some(s), Some(t)) => {
                    if s.kind != t.kind {
                        d.type_conflict.push(k.clone());
                    } else if s.kind == "dir" {
                        // presence only
                    } else if stat.settles(s.size, s.mtime_ns, t.size, t.mtime_ns)
                        || hashes_differ(&s.hashes, &t.hashes, required.of(k))
                    {
                        d.changed.push(k.clone());
                    }
                }
                (None, None) => {}
            }
        }
    } else {
        // lower -> original (mixed-case already validated absent within each side)
        let mut slow: HashMap<String, &String> = HashMap::new();
        let mut dlow: HashMap<String, &String> = HashMap::new();
        for k in src.keys() {
            slow.insert(k.to_lowercase(), k);
        }
        for k in dst.keys() {
            dlow.insert(k.to_lowercase(), k);
        }
        let mut keys: HashSet<String> = HashSet::new();
        for k in slow.keys().chain(dlow.keys()) {
            keys.insert(k.clone());
        }
        for lk in keys {
            match (slow.get(&lk), dlow.get(&lk)) {
                (Some(srel), None) => d.missing.push((*srel).clone()),
                (None, Some(trel)) => d.extra.push((*trel).clone()),
                (Some(srel), Some(trel)) => {
                    if *srel != *trel {
                        d.case_mismatch.push(((*srel).clone(), (*trel).clone()));
                    }
                    let s = &src[*srel];
                    let t = &dst[*trel];
                    if s.kind != t.kind {
                        d.type_conflict.push((*srel).clone());
                    } else if s.kind == "dir" {
                    } else if stat.settles(s.size, s.mtime_ns, t.size, t.mtime_ns)
                        || hashes_differ(&s.hashes, &t.hashes, required.of(srel))
                    {
                        d.changed.push((*srel).clone());
                    }
                }
                (None, None) => {}
            }
        }
    }
    d.missing.sort();
    d.extra.sort();
    d.changed.sort();
    d.type_conflict.sort();
    d.case_mismatch.sort();
    d
}

/// True if any of the algorithms that settle this pair has both sides present and
/// disagreeing.
///
/// If either side lacks one of them this stays silent — the caller has already
/// compared size+mtime. That silence is safe *because* the planner chose these
/// algorithms: every one of them is obtainable on both sides, so it has either
/// been computed or was already cached. A digest that is absent here is a digest
/// nobody asked for, which is what makes this a comparison rather than the
/// degradation stage 5b closed.
pub fn hashes_differ(
    a: &HashMap<String, Vec<u8>>,
    b: &HashMap<String, Vec<u8>>,
    algos: &[String],
) -> bool {
    for algo in algos {
        // If either side lacks the hash (e.g. --hash none history), fall back to
        // size+mtime which the caller already compared; do not force differ here.
        if let (Some(x), Some(y)) = (a.get(algo), b.get(algo))
            && x != y
        {
            return true;
        }
    }
    false
}
