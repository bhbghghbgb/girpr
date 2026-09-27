//! The case-fixing rename pass.
//!
//! In insensitive mode a path that exists on both sides under different casing
//! must be renamed on dst *before* the diff runs, so the two maps line up on
//! exact keys. After this pass the diff can be taken case-sensitively.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::info;

/// Rename dst paths to src's casing, updating the dst cache and `dm` in step.
///
/// Returns the number of renames performed (or that a dry run would perform).
///
/// `dm` is always re-keyed to src's casing, including under `dry_run` — the plan
/// computed after this pass reads `dm`, so leaving it on the old casing would
/// make a dry run report a case-only difference as both a copy and a delete.
/// `dry_run` therefore suppresses only the filesystem and cache writes.
pub(super) fn rename_to_src_casing(
    src: &HashMap<String, crate::effective::EffRec>,
    dst_root: &Path,
    dst_db: &sled::Db,
    dm: &mut HashMap<String, crate::effective::EffRec>,
    dry_run: bool,
) -> Result<usize> {
    // lower -> src rel
    let mut slow: HashMap<String, &String> = HashMap::new();
    for k in src.keys() {
        slow.insert(k.to_lowercase(), k);
    }
    let mut renames: Vec<(PathBuf, PathBuf, String, String)> = Vec::new();
    for (lk, srel) in &slow {
        // find dst key with same lower but different case
        if let Some(drel) = dm.keys().find(|k| k.to_lowercase() == *lk && *k != *srel) {
            renames.push((
                dst_root.join(drel),
                dst_root.join(*srel),
                (*drel).clone(),
                (*srel).clone(),
            ));
        }
    }
    info!(pending = renames.len(), "rename pass");
    let mut renamed = 0usize;
    for (from, to, drel, srel) in renames {
        println!("RENAME {} -> {}", drel, srel);
        info!(from = %drel, to = %srel, "rename");
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
        if let Ok(Some(raw)) = dst_db.get(drel.as_bytes()) {
            dst_db.remove(drel.as_bytes())?;
            dst_db.insert(srel.as_bytes(), raw)?;
        }
    }
    Ok(renamed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effective::EffRec;

    fn file_rec() -> EffRec {
        EffRec {
            kind: "file".into(),
            size: 4,
            mtime_ns: 0,
            hashes: HashMap::new(),
        }
    }

    /// Regression: a dry run must still re-key the dst map to src's casing.
    /// Leaving it on the old casing made the plan report a case-only
    /// difference as both `COPY` and `DELETE`.
    #[test]
    fn dry_run_rekeys_the_dst_map_without_touching_disk() {
        let root = std::env::temp_dir().join(format!("girsync_rename_dry_{}", std::process::id()));
        let dst = root.join("dst");
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(dst.join("data.txt"), b"data").unwrap();

        let mut sm = HashMap::new();
        sm.insert("Data.txt".to_string(), file_rec());
        let mut dm = HashMap::new();
        dm.insert("data.txt".to_string(), file_rec());
        let db = sled::Config::new().temporary(true).open().unwrap();

        let renamed = rename_to_src_casing(&sm, &dst, &db, &mut dm, true).unwrap();
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

        std::fs::remove_dir_all(&root).ok();
    }
}
