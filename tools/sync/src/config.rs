//! Validated per-run configuration.
//!
//! The CLI hands over raw strings; every command needs the same hash/filter/case
//! vocabulary, so it is parsed once into [`CommonOpts`] and passed around as a
//! struct instead of a dozen positional arguments.

use anyhow::Result;
use glob::Pattern;
use std::path::PathBuf;

use crate::cli::CommonArgs;
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
/// These used to be three bare `bool`s on
/// [`crate::effective::build_effective_folder`], where the call sites had to be
/// read to know what a given combination meant.
#[derive(Debug, Clone, Copy)]
pub struct ScanMode {
    /// Trust cached hashes on a size+mtime hit; `false` rehashes everything.
    pub fast: bool,
    /// Rehash even when the cache is fresh and complete (`update` always sets
    /// this).
    pub force_hash: bool,
    /// Plan only: no cache writes and no filesystem changes.
    pub dry_run: bool,
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
    /// Derived from `--no-fast`.
    pub fast: bool,
    pub common: CommonOpts,
}

/// `girsync sync` inputs.
#[derive(Debug)]
pub struct SyncOpts {
    pub src: PathBuf,
    pub dst: PathBuf,
    /// Derived from `--no-fast`.
    pub fast: bool,
    /// Copy src-only files, skip content updates.
    pub missing_only: bool,
    /// Leave dst-only files alone instead of deleting them.
    pub keep_extra: bool,
    pub dry_run: bool,
    /// Copy worker threads; must be >= 1.
    pub jobs: usize,
    pub common: CommonOpts,
}
