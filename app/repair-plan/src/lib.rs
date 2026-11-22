//! Repair-plan values and the pure decisions over them.
//!
//! This is an acyclic leaf: it owns the work-list and cleanup-plan values, the
//! path-scope value, and the pure figures the plan phase derives from them. It
//! emits no bytes, runs no filesystem walk, fetches nothing and publishes
//! nothing. The facade keeps those policies and consumes these values directly.
//!
//! `Mode` is deliberately not here: the `girpr-repair-policy` leaf already
//! owns it, and both the disk estimate and the writer consume it from there.
//! Keeping it there means this crate adds no second home for the write-mode
//! decision.

use std::collections::HashMap;

use girpr_fetch::sophon::FileReuse;

pub mod path_scope;
pub mod plan;
pub mod purge_values;

pub use path_scope::{PathScope, compile_patterns};
pub use plan::{
    FileId, Layout, PlannedFile, RepairPlan, build_plan, est_file_download_bytes,
    validate_chunk_ranges,
};
pub use purge_values::{PurgeCandidate, PurgeKeys, PurgePlan};

/// `path -> FileReuse`, built from the local build's manifests. Keyed by
/// path only: reuse is same-file-only.
///
/// The type alias used to live in the facade's manifest module. It sits here
/// because [`RepairPlan::est_download_bytes`] and the facade's scan both name
/// it, so both now point at this one leaf instead of a module in the crate
/// being split.
pub type LocalChunkMap = HashMap<String, FileReuse>;

/// The download estimate: `Σ compressed_size` over the chunks the local reuse
/// map cannot supply, split per file.
///
/// The split is computed once, per file, and both consumers fold it: the `PLAN`
/// total and the `--plan-out` document. A second derivation would be a second
/// answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadEstimate {
    pub by_file: Vec<u64>,
    pub bytes: u64,
}

/// The plan-phase values the estimate derives for `SUMMARY`.
///
/// Kept apart from [`DownloadEstimate`] so that type stays about bytes: these
/// are the report-shaped facts, not figures the write path uses. The facade
/// writes them into `SUMMARY` in one place, under the same flag conditions as
/// before; this crate only decides their values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanSummaryValues {
    /// `Some(bytes)` when `limits.download_bytes` asked for the figure, so the
    /// caller can log and report it without re-deriving the condition.
    pub reported_bytes: Option<u64>,
    /// The scan's damaged-chunk count, when a scan ran.
    pub damaged_chunks: Option<u64>,
    /// The plan size, for `SUMMARY plan_files`.
    pub files_total: u64,
    /// Whether any ceiling was requested, for `SUMMARY limited=1`.
    pub limited: bool,
}

/// An `--include` that selected nothing at all.
///
/// The rule below returns this instead of a process error so the facade keeps
/// its own error type and exit-code mapping. The message is the whole point of
/// the failure — it names the typo — and is owned here verbatim so the rule
/// and its diagnosis cannot drift apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSelectionError {
    message: String,
}

impl ScopeSelectionError {
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ScopeSelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The "scope selected something" rule.
///
/// An `--include` that selected **nothing at all** is a typo, and a typo that
/// costs a full install's worth of downloads to discover is the failure mode
/// the feature exists to remove. "Nothing at all" spans both plans: an empty
/// repair plan alone is the ordinary "removed upstream" case, so a purge
/// candidate or an absent `--include` clears the rule. The caller decides how
/// to surface the failure; this function only decides whether it applies and
/// what it says.
pub fn check_scope_selection(
    scope: &PathScope,
    plan: &RepairPlan,
    purge_candidates: usize,
    purge_plan: Option<&PurgePlan>,
) -> Result<(), ScopeSelectionError> {
    if !scope.has_includes() || !plan.is_empty() || purge_candidates != 0 {
        return Ok(());
    }
    let message = if purge_plan.is_some() {
        format!(
            "--include '{}' selected nothing: plan_files=0 and the files-cleanup has no \
             candidates either; check the glob spelling (`*` also crosses `/`) or the \
             installed version",
            scope.describe_includes(),
        )
    } else {
        format!(
            "--include '{}' matched no file in the live manifest (plan_files=0); \
             check the glob spelling (`*` also crosses `/`) or the installed version",
            scope.describe_includes(),
        )
    };
    Err(ScopeSelectionError { message })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// The rule fires only for an empty repair plan, a present `--include`, and
    /// no purge candidates — and the two messages keep their published wording.
    #[test]
    fn scope_selection_rule_and_wording() {
        let includes = vec!["missing/*".to_string()];
        let scoped = PathScope::new(&includes, &[]).unwrap();
        let open = PathScope::default();
        let empty = RepairPlan::from_files("9.9", Vec::new());
        let one = RepairPlan::from_files(
            "9.9",
            vec![PlannedFile {
                rel: "a.dat".into(),
                size: 1,
                md5: String::new(),
                url_prefix: Arc::from(""),
                chunks: vec![],
            }],
        );
        let purge = PurgePlan::default();

        assert!(check_scope_selection(&open, &empty, 0, None).is_ok());
        assert!(check_scope_selection(&scoped, &one, 0, None).is_ok());
        assert!(check_scope_selection(&scoped, &empty, 1, None).is_ok());

        let err = check_scope_selection(&scoped, &empty, 0, None).unwrap_err();
        assert_eq!(
            err.message(),
            "--include 'missing/*' matched no file in the live manifest (plan_files=0); \
             check the glob spelling (`*` also crosses `/`) or the installed version"
        );
        assert_eq!(err.to_string(), err.message());

        let err = check_scope_selection(&scoped, &empty, 0, Some(&purge)).unwrap_err();
        assert_eq!(
            err.message(),
            "--include 'missing/*' selected nothing: plan_files=0 and the files-cleanup has no \
             candidates either; check the glob spelling (`*` also crosses `/`) or the \
             installed version"
        );
    }
}
