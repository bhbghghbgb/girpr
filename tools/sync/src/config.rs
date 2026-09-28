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
    /// Plan only: no cache writes and no filesystem changes.
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
