//! Validated per-run configuration.
//!
//! The CLI hands over raw strings; every command needs the same hash/filter/case
//! vocabulary, so it is parsed once into [`CommonOpts`] and passed around as a
//! struct instead of a dozen positional arguments.
//!
//! `common` holds the settings every subcommand shares, and the `TryFrom`
//! impls that validate the raw flags into them — including the two refusals that
//! are properties of the *request* rather than of the command receiving it. This
//! file holds the per-command input structs a caller fills in: one per
//! subcommand, each a path (or two) plus [`CommonOpts`].

mod common;

pub use common::{CommonOpts, LogCtx, ScanMode};

use crate::cli::{TrustArgs, TrustSide};

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
    pub dir: std::path::PathBuf,
    pub common: CommonOpts,
}

/// `girsync compare` inputs.
#[derive(Debug)]
pub struct CompareOpts {
    pub src: std::path::PathBuf,
    pub dst: std::path::PathBuf,
    /// Derived from `--no-trust-cached-hashes`.
    pub trust: TrustOpts,
    pub common: CommonOpts,
}

/// `girsync compare-self` inputs.
#[derive(Debug)]
pub struct CompareSelfOpts {
    /// The folder to audit. Its `girpr-cache` is the record side.
    pub dir: std::path::PathBuf,
    /// A plain boolean, unlike the two-sided commands: there is only one scanned
    /// side here, so naming it `src` or `dst` would be a choice with no meaning.
    pub no_trust_cached_hashes: bool,
    pub common: CommonOpts,
}

/// `girsync sync` inputs.
#[derive(Debug)]
pub struct SyncOpts {
    pub src: std::path::PathBuf,
    pub dst: std::path::PathBuf,
    /// Derived from `--no-trust-cached-hashes`.
    pub trust: TrustOpts,
    /// Copy src-only files, skip content updates.
    pub missing_only: bool,
    /// Leave dst-only files alone instead of deleting them.
    pub keep_extra: bool,
    /// `--dry-run` is `common.dry_run`. Read it from there rather than keeping a
    /// copy here: the cache half and the filesystem half are the same flag, and
    /// two fields for one flag is how they drift apart.
    ///
    /// `--rw-threads*` is likewise `common.rw` and has no copy here, for the same
    /// reason: the concurrency knob is a run-wide resource, not a `sync` one.
    pub common: CommonOpts,
}
