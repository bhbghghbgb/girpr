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
/// `dm` is the dst effective map and is mutated so the in-memory view matches
/// the filesystem after the renames.
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
        if dry_run {
            renamed += 1;
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
        // move DB entry + in-memory entry
        if let Some(v) = dm.remove(&drel) {
            dm.insert(srel.clone(), v);
        }
        if let Ok(Some(raw)) = dst_db.get(drel.as_bytes()) {
            dst_db.remove(drel.as_bytes())?;
            dst_db.insert(srel.as_bytes(), raw)?;
        }
        renamed += 1;
    }
    Ok(renamed)
}
