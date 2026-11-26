//! The values of the files-cleanup decision.
//!
//! The facade owns the walk that discovers candidates and the delete that acts
//! on them. What it decides over lives here: a [`PurgePlan`] is a complete,
//! unexecuted cleanup decision, and [`PurgeKeys`] is the folded, immutable keep
//! input both purge passes read. Keeping them beside the repair plan in this
//! leaf is what lets the resource-ceilings module consume both plans without
//! the plan modules pointing back at the scope tier.

use std::collections::HashSet;
use std::path::PathBuf;

use girpr_util_path::path::path_key;

use crate::plan::RepairPlan;

/// One file the files-cleanup has decided to delete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurgeCandidate {
    /// `/`-separated rel path in the on-disk (walk) spelling. Classification
    /// compares its `path_key`, so the spelling can differ in case from the
    /// manifest entry that kept it.
    pub rel: String,
    /// Absolute-or-game-relative path to remove.
    pub path: PathBuf,
    pub bytes: u64,
}

/// The complete, unexecuted files-cleanup decision.
///
/// Built by the facade's walk, consumed by the facade's delete. Everything the
/// run will delete is here before the first byte is deleted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PurgePlan {
    /// Delete candidates, sorted by path so a rerun's log is comparable.
    pub files: Vec<PurgeCandidate>,
    /// Directories the purge empties, deepest first; `game_dir` excluded.
    pub dirs: Vec<PathBuf>,
    /// `files` summed — the number a delete-bytes ceiling is checked against.
    pub total_bytes: u64,
    /// On-disk paths (files **and** directories) kept **only** because the
    /// path scope hides them. Zero on an unflagged run; the "scope and a purge
    /// flag agree" warn is driven off this.
    pub hidden: u64,
    /// Walk or metadata failures that made the candidate list incomplete.
    ///
    /// A purge that could not read part of the install still deletes what it
    /// can see, but `total_bytes` is then a lower bound: the caller must not
    /// treat it as the whole delete set. `--max-delete-bytes` therefore
    /// refuses an incomplete plan instead of comparing that lower bound. Zero
    /// for a complete plan, and for the documented "`game_dir` is not there
    /// yet" case, which is an empty plan rather than a failure.
    pub errors: u64,
}

/// The plan-derived keep inputs to the files-cleanup, folded once per run.
///
/// `expected` comes from [`RepairPlan::expected_rels`], `server_keep` from the
/// facade's keep set, and `blacklist` from the launcher's `blacklist_dir`.
/// All three are keyed with [`path_key`] so the walked rel (folded once per
/// file) meets them in the same spelling.
///
/// [`RepairPlan`] is immutable after construction and shared by `Arc` across
/// the before/after passes, so this cache cannot go stale within one run. The
/// walked candidates are deliberately **not** part of it: the filesystem can
/// change between passes, so each pass recomputes them.
#[derive(Debug, Clone)]
pub struct PurgeKeys {
    expected: HashSet<String>,
    server_keep: HashSet<String>,
    blacklist: HashSet<String>,
}

impl PurgeKeys {
    /// Fold the three keep inputs once for a run.
    pub fn new(
        plan: &RepairPlan,
        server_keep: &HashSet<String>,
        blacklist: &HashSet<String>,
    ) -> Self {
        // The expected set is keyed straight from the plan's rels: an owned
        // raw-spelling set would only be re-keyed into a second one.
        Self {
            expected: plan.expected_rels().map(path_key).collect(),
            server_keep: server_keep.iter().map(|r| path_key(r)).collect(),
            blacklist: blacklist.iter().map(|r| path_key(r)).collect(),
        }
    }

    /// Add more paths that must survive the cleanup, keyed the same way as
    /// the base plan's rels. Used by the persistent tier: a hotfix path, a
    /// manifest mirror or a verification marker is just as expected as a base
    /// file, and a cleanup must not delete it.
    pub fn extend_expected<'a>(&mut self, rels: impl IntoIterator<Item = &'a str>) {
        self.expected.extend(rels.into_iter().map(path_key));
    }

    /// The planned rels, keyed with [`path_key`].
    pub fn expected(&self) -> &HashSet<String> {
        &self.expected
    }

    /// The manifest-external keep set, keyed with [`path_key`].
    pub fn server_keep(&self) -> &HashSet<String> {
        &self.server_keep
    }

    /// The launcher's blacklist, keyed with [`path_key`].
    pub fn blacklist(&self) -> &HashSet<String> {
        &self.blacklist
    }
}
