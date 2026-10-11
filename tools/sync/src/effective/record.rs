//! **Phase A** for a record side: the DB read as-is.
//!
//! The counterpart to [`crate::effective::scan_stat_only`] for a side with no
//! filesystem behind it. It lives in its own module rather than inside
//! `phase_a` because the two are not variants of one walk: one reads a directory,
//! the other reads a table, and sharing a module would invite a reader to assume
//! they can be swapped.

use anyhow::{Result, bail};
use std::collections::{HashMap, HashSet};

use crate::cache::CacheDb;
use crate::config::CommonOpts;
use crate::filter::is_excluded;
use crate::util::is_cache_rel;

use super::entry::{SideEntry, SideScan};

/// **Phase A** for a record side: the DB read as-is, with no FS access and no
/// writes. Cache rows and filtered paths are dropped.
///
/// Takes an already-open cache rather than a path so a caller needing the record
/// view *and* a disk scan of the same folder can do both from one handle. That is
/// not a lock requirement — two read-only handles share the file — it is a cost
/// one: each `CacheDb` builds its own copy of the file's tables, so opening twice
/// doubles the memory a large cache occupies. The same reasoning applies harder to
/// a writable handle, where the second open would be refused outright.
///
/// `label` names the record in error messages; the caller knows the path.
pub fn load_record_side_from(
    cache: &CacheDb,
    common: &CommonOpts,
    label: &str,
) -> Result<SideScan<SideEntry>> {
    let all = cache.load_all()?;
    let mut out: HashMap<String, SideEntry> = HashMap::new();
    for (rel, r) in all {
        if is_cache_rel(&rel) {
            continue;
        }
        if is_excluded(
            &rel,
            &common.includes,
            &common.excludes,
            common.case_sensitive,
        ) {
            continue;
        }
        out.insert(
            rel,
            SideEntry {
                kind: r.kind,
                size: r.size,
                mtime_ns: r.mtime_ns,
                cached: r.hashes,
                fresh: true,
            },
        );
    }
    if !common.case_sensitive {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for k in out.keys() {
            map.entry(k.to_lowercase()).or_default().push(k.clone());
        }
        for (_, v) in map {
            let uniq: HashSet<&String> = v.iter().collect();
            if uniq.len() > 1 {
                bail!(
                    "mixed-case collision in record {}: {}",
                    label,
                    v.join(" vs ")
                );
            }
        }
    }
    // A record's row *is* its stat — there is no disk to disagree with — so
    // every row is fresh and its digests are carried whole.
    Ok(SideScan::of_entries(out))
}
