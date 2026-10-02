//! Validated per-run configuration.
//!
//! The CLI hands over raw strings; every command needs the same hash/filter/case
//! vocabulary, so it is parsed once into [`CommonOpts`] and passed around as a
//! struct instead of a dozen positional arguments.

use anyhow::Result;
use glob::Pattern;
use std::path::PathBuf;

use crate::cli::{CommonArgs, TrustArgs, TrustSide};
use crate::filter::compile_patterns;
use crate::hash::parse_hash_list;

/// Logging knobs, carried only to stamp the per-command span.
#[derive(Debug, Clone)]
pub struct LogCtx {
    /// Raw `--log-level` string, echoed into spans as the console level.
    pub level: String,
    /// Raw `--log-file` path, echoed into spans.
    pub file: Option<PathBuf>,
}

impl LogCtx {
    /// Span-friendly rendering of `--log-file`.
    pub fn file_display(&self) -> String {
        self.file
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(none)".to_string())
    }
}

/// Hash/filter/case/walk settings shared by `update`, `compare`, and `sync`.
///
/// Built from raw CLI strings via [`CommonOpts::try_from`], so the patterns and
/// algorithms are already compiled and validated by the time a command runs.
#[derive(Debug, Clone)]
pub struct CommonOpts {
    /// Algorithms to record; empty means `--hash none` (size+mtime only).
    pub algos: Vec<String>,
    /// `--include` globs; empty means "everything".
    pub includes: Vec<Pattern>,
    /// `--exclude` globs; these win over `includes`.
    pub excludes: Vec<Pattern>,
    /// Default false: insensitive, with rename/abort rules.
    pub case_sensitive: bool,
    /// Walk depth cap for the whole tree (default 10).
    pub max_depth: usize,
    /// Back up + delete the cache before opening, forcing a rebuild.
    pub ignore_cache: bool,
}

impl TryFrom<CommonArgs> for CommonOpts {
    type Error = anyhow::Error;

    fn try_from(a: CommonArgs) -> Result<Self> {
        Ok(Self {
            algos: parse_hash_list(&a.hash)?,
            includes: compile_patterns(&a.include)?,
            excludes: compile_patterns(&a.exclude)?,
            case_sensitive: a.case_sensitive,
            max_depth: a.max_depth,
            ignore_cache: a.ignore_cache,
        })
    }
}

/// How a single folder scan treats the cache and the filesystem.
///
/// `fast` and `force_hash` used to be separate fields here, but they described
/// the same decision — whether a size+mtime-matching cache entry may stand in
/// for a digest — so they are one flag now.
#[derive(Debug, Clone, Copy)]
pub struct ScanMode {
    /// Rehash even when the cache holds a size+mtime match for this path.
    /// Set by `--no-trust-cached-hashes <side>`, and unconditionally by
    /// `update`, which is defined as a full repopulate.
    pub no_trust_cached_hashes: bool,
    /// No writes. Nothing this run computes is persisted.
    ///
    /// A dry run must produce **the same answer as the real run**, with the
    /// writes removed and nothing else. That is the whole contract, and it has
    /// two halves that are easy to confuse:
    ///
    /// - *decisions* must be identical — a dry run still stats the tree, still
    ///   reads cached digests, and still hashes whatever the planner cannot
    ///   settle from stat alone. `can_hash_from_disk` stays true precisely
    ///   because suppressing those reads would change the verdict, not just the
    ///   side effects.
    /// - *writes* must be absent — no cache mutation, no cache creation, no
    ///   backup, and for a command that moves files, no filesystem change.
    ///
    /// This field is the cache half. A command that also moves files threads its
    /// own `dry_run` into those phases (`sync` does); a command that never
    /// writes files has no second half and this field is the whole of it.
    ///
    /// How the cache is *opened* under this flag is
    /// [`crate::effective::open_folder_cache`]'s decision, so no command can
    /// honour the flag incorrectly by opening the wrong kind of handle.
    pub dry_run: bool,
}

/// Which sides of a two-sided run distrust cached digests.
#[derive(Debug, Clone, Copy, Default)]
pub struct TrustOpts {
    /// `--no-trust-cached-hashes src`
    pub no_trust_src: bool,
    /// `--no-trust-cached-hashes dst`
    pub no_trust_dst: bool,
}

impl From<TrustArgs> for TrustOpts {
    fn from(a: TrustArgs) -> Self {
        let mut o = Self::default();
        for side in a.no_trust_cached_hashes {
            match side {
                TrustSide::Src => o.no_trust_src = true,
                TrustSide::Dst => o.no_trust_dst = true,
            }
        }
        o
    }
}

/// `girsync update` inputs.
#[derive(Debug)]
pub struct UpdateOpts {
    pub dir: PathBuf,
    pub common: CommonOpts,
}

/// `girsync compare` inputs.
#[derive(Debug)]
pub struct CompareOpts {
    pub src: PathBuf,
    pub dst: PathBuf,
    /// Derived from `--no-trust-cached-hashes`.
    pub trust: TrustOpts,
    /// Write nothing at all: no cache updates, no cache creation, no backups.
    ///
    /// `compare` has no filesystem half to suppress — it never touches the trees
    /// — but a folder side does write its cache as it resolves, so without this
    /// there is no way to audit two folders and leave both caches exactly as
    /// they were. The report is identical either way; see
    /// [`ScanMode::dry_run`].
    pub dry_run: bool,
    pub common: CommonOpts,
}

/// `girsync compare-self` inputs.
#[derive(Debug)]
pub struct CompareSelfOpts {
    /// The folder to audit. Its `girpr-cache` is the record side.
    pub dir: PathBuf,
    /// A plain boolean, unlike the two-sided commands: there is only one scanned
    /// side here, so naming it `src` or `dst` would be a choice with no meaning.
    pub no_trust_cached_hashes: bool,
    pub common: CommonOpts,
}

/// `girsync sync` inputs.
#[derive(Debug)]
pub struct SyncOpts {
    pub src: PathBuf,
    pub dst: PathBuf,
    /// Derived from `--no-trust-cached-hashes`.
    pub trust: TrustOpts,
    /// Copy src-only files, skip content updates.
    pub missing_only: bool,
    /// Leave dst-only files alone instead of deleting them.
    pub keep_extra: bool,
    pub dry_run: bool,
    /// Copy worker threads; must be >= 1.
    pub jobs: usize,
    pub common: CommonOpts,
}
