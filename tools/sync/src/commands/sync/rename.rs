//! The case-fixing rename pass.
//!
//! In insensitive mode a path that exists on both sides under different casing
//! must be renamed on dst *before* the diff runs, so the two maps line up on
//! exact keys. After this pass the diff can be taken case-sensitively.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::info;

use crate::report::{Record, Report};

/// The dst paths that must move to src's casing, as `(dst rel, src rel)` pairs.
///
/// The decision, with no filesystem or cache involved, so it can be stated as a
/// set. **A dst path is renamed exactly when src holds the same path under
/// different casing** — the pair whose lowercase key matches and whose exact key
/// does not.
///
/// Both sides are indexed by lowercase key and every src key is looked up, rather
/// than every src key scanning every dst key and lowercasing as it goes. The result
/// is the same set, and it is the same set because a lowercase key maps to exactly
/// one path per side: `check_mixed_case` has already made a mixed-case collision
/// within one side fatal, and this pass runs only in the insensitive mode that check
/// guards. Without that invariant the two forms could disagree on *which* of two
/// colliding keys they pick, which is why this is not a free substitution.
///
/// Ordered by lowercase src key, because these become `RENAME` records in report
/// order and `HashMap` iteration is seeded per process — an unsorted pass would
/// reshuffle between two runs over the same tree.
fn case_renames<'a>(
    src: &'a HashMap<String, crate::effective::EffRec>,
    dm: &'a HashMap<String, crate::effective::EffRec>,
) -> Vec<(&'a String, &'a String)> {
    // lower -> rel, per side.
    let mut slow: HashMap<String, &String> = HashMap::new();
    for k in src.keys() {
        slow.insert(k.to_lowercase(), k);
    }
    let mut dlow: HashMap<String, &String> = HashMap::new();
    for k in dm.keys() {
        dlow.insert(k.to_lowercase(), k);
    }
    let mut by_lower: Vec<(String, &String)> = slow.into_iter().collect();
    by_lower.sort();
    let mut out: Vec<(&String, &String)> = Vec::new();
    for (lk, srel) in &by_lower {
        // The same path on both sides is not a rename: it is already correct.
        if let Some(drel) = dlow.get(lk).copied().filter(|d| *d != *srel) {
            out.push((drel, srel));
        }
    }
    out
}

/// Rename dst paths to src's casing, updating the dst cache and `dm` in step.
///
/// Returns the number of renames performed (or that a dry run would perform).
///
/// `dm` is always re-keyed to src's casing, including under `dry_run` — the plan
/// computed after this pass reads `dm`, so a map left on dst's own casing would
/// make a dry run report a case-only difference as both a copy and a delete.
/// `dry_run` therefore suppresses only the filesystem and cache writes.
pub(super) fn rename_to_src_casing(
    src: &HashMap<String, crate::effective::EffRec>,
    dst_root: &Path,
    dst_db: &crate::cache::CacheDb,
    dm: &mut HashMap<String, crate::effective::EffRec>,
    dry_run: bool,
    report: &Report,
) -> Result<usize> {
    let mut renames: Vec<(PathBuf, PathBuf, String, String)> = Vec::new();
    for (drel, srel) in case_renames(src, dm) {
        renames.push((
            dst_root.join(drel),
            dst_root.join(srel),
            drel.clone(),
            srel.clone(),
        ));
    }
    info!(pending = renames.len(), "rename pass");
    let mut renamed = 0usize;
    for (from, to, drel, srel) in renames {
        // One record, one log event. `Report::emit` does both, so there is no
        // second hand-written `info!` stating the same rename — the count above and
        // the `SUMMARY` cover the narration, and the record covers the detail.
        report.emit(Record::pair("RENAME", "->", "from", &drel, "to", &srel));
        renamed += 1;

        // Re-key the in-memory map *before* the dry-run bail-out: the plan
        // computed after this pass reads `dm`, so a dry run that left it on the
        // old casing would report a case-only difference as both a copy and a
        // delete. This cannot fail, so doing it first is safe either way.
        if let Some(v) = dm.remove(&drel) {
            dm.insert(srel.clone(), v);
        }

        if dry_run {
            continue;
        }

        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("mkdir {}", parent.display()))?;
        }
        // two-step for Windows case-only rename
        let tmp = from.with_extension("girsync_case_tmp");
        std::fs::rename(&from, &tmp)
            .with_context(|| format!("rename {} (case fix step1)", from.display()))?;
        std::fs::rename(&tmp, &to)
            .with_context(|| format!("rename to {} (case fix step2)", to.display()))?;
        // move the DB entry to match
        if let Some(rec) = dst_db.get(&drel)? {
            let mut w = dst_db.begin_write()?;
            w.remove(&drel)?;
            w.put(&srel, &rec)?;
            w.commit()?;
        }
    }
    Ok(renamed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effective::EffRec;
    use crate::report::OutputFormat;

    fn file_rec() -> EffRec {
        EffRec {
            kind: "file".into(),
            size: 4,
            mtime_ns: 0,
            hashes: crate::hash::DigestSet::default(),
        }
    }

    fn side(rels: &[&str]) -> HashMap<String, EffRec> {
        rels.iter().map(|r| (r.to_string(), file_rec())).collect()
    }

    /// The rename set, as sorted `(dst, src)` string pairs.
    fn renames(src: &[&str], dst: &[&str]) -> Vec<(String, String)> {
        let (s, d) = (side(src), side(dst));
        let mut out: Vec<(String, String)> = case_renames(&s, &d)
            .into_iter()
            .map(|(d, s)| (d.clone(), s.clone()))
            .collect();
        out.sort();
        out
    }

    /// The set is exactly "dst holds it under different casing", stated as a set
    /// rather than as a transcript of the records — the ordering is a consequence
    /// of the sort, not the property being pinned.
    ///
    /// Three shapes in one tree: a pure case-only difference, a path already in
    /// step (which must *not* be renamed to itself), and a pair of paths that
    /// differ by more than case and so are not a rename at all.
    #[test]
    fn the_rename_set_is_exactly_the_case_only_differences() {
        assert_eq!(
            renames(
                &[
                    "Data.txt",
                    "same.txt",
                    "Alpha/Beta.txt",
                    "only_src.txt",
                    "only_dst.txt"
                ],
                &["data.txt", "same.txt", "alpha/beta.txt"],
            ),
            vec![
                ("alpha/beta.txt".to_string(), "Alpha/Beta.txt".to_string()),
                ("data.txt".to_string(), "Data.txt".to_string()),
            ],
        );
    }

    /// Zero renames is the common case and the one a lookup can get wrong: the
    /// dst path *is* the src path, so the lowercase key matches and only the
    /// exact-key check stands between the two and a spurious self-rename.
    #[test]
    fn a_tree_already_in_step_renames_nothing() {
        assert_eq!(
            renames(
                &["a.txt", "sub/B.txt", "c.txt"],
                &["a.txt", "sub/B.txt", "c.txt"],
            ),
            Vec::<(String, String)>::new(),
        );
    }

    /// Several renames at once, and the one-sided paths among them stay out of
    /// the set: a path only one side holds has no casing to align.
    #[test]
    fn several_case_only_differences_rename_independently() {
        assert_eq!(
            renames(
                &["One.txt", "Two.txt", "Three.txt", "src_only.txt"],
                &["one.txt", "two.txt", "three.txt", "dst_only.txt"],
            ),
            vec![
                ("one.txt".to_string(), "One.txt".to_string()),
                ("three.txt".to_string(), "Three.txt".to_string()),
                ("two.txt".to_string(), "Two.txt".to_string()),
            ],
        );
    }

    /// A path src does not hold is never renamed, whatever dst calls it — the
    /// pass moves dst onto *src's* casing, so a dst-only path keeps its own.
    #[test]
    fn a_dst_only_path_is_never_a_rename() {
        assert_eq!(
            renames(&["a.txt"], &["a.txt", "Orphan.txt"]),
            Vec::<(String, String)>::new(),
        );
    }

    /// The order is by lowercase src key, not `HashMap` order. These become
    /// `RENAME` records in both modes, and the seed is per process, so unsorted
    /// order would reshuffle between two runs over the same tree.
    #[test]
    fn the_rename_order_is_by_lowercase_key() {
        let s = side(&["Zeta.txt", "Alpha.txt", "Mid.txt"]);
        let d = side(&["zeta.txt", "alpha.txt", "mid.txt"]);
        let ordered: Vec<String> = case_renames(&s, &d)
            .into_iter()
            .map(|(_, s)| s.to_lowercase())
            .collect();
        assert_eq!(ordered, vec!["alpha.txt", "mid.txt", "zeta.txt"]);
    }

    /// A dry run must re-key the dst map to src's casing, not just report that it
    /// would: the plan computed afterwards reads `dm`, so a map left on dst's own
    /// casing reports a case-only difference as both `COPY` and `DELETE`.
    #[test]
    fn dry_run_rekeys_the_dst_map_without_touching_disk() {
        let root = crate::testtemp::TempDir::new("girsync_rename_dry", "rekeys_map");
        let dst = root.join("dst");
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(dst.join("data.txt"), b"data").unwrap();

        let mut sm = HashMap::new();
        sm.insert("Data.txt".to_string(), file_rec());
        let mut dm = HashMap::new();
        dm.insert("data.txt".to_string(), file_rec());
        let db = crate::cache::CacheDb::open_temp(false).unwrap();

        let report = Report::new(OutputFormat::Text);
        let renamed = rename_to_src_casing(&sm, &dst, &db, &mut dm, true, &report).unwrap();
        assert_eq!(renamed, 1);
        assert!(
            dm.contains_key("Data.txt"),
            "dst map follows src casing after a dry run"
        );
        assert!(
            !dm.contains_key("data.txt"),
            "stale casing dropped from the dst map"
        );
        assert!(
            dst.join("data.txt").is_file(),
            "a dry run still renames nothing on disk"
        );
    }
}
