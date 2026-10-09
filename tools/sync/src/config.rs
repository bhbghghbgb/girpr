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
use crate::hash::{parse_any_of, parse_hash_list};
use crate::planner::{HashMode, StatTrust};
use crate::report::{OutputFormat, Report};

/// How the run reports itself: the log knobs and the stdout format.
///
/// Carried to every command for two reasons and no others. The `--log-*` values
/// exist so the per-command span can stamp them, and `output` is the one thing a
/// command needs in order to build its [`Report`] — a report format chosen here
/// and rebuilt per command would be four independent reads of the same flag.
#[derive(Debug, Clone)]
pub struct LogCtx {
    /// Raw `--log-level` string, echoed into spans as the console level.
    pub level: String,
    /// Raw `--log-file` path, echoed into spans.
    pub file: Option<PathBuf>,
    /// `--output`: how stdout records are rendered.
    pub output: OutputFormat,
}

impl LogCtx {
    /// Span-friendly rendering of `--log-file`.
    pub fn file_display(&self) -> String {
        self.file
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(none)".to_string())
    }

    /// The stdout writer for this run, in the requested format.
    pub fn report(&self) -> Report {
        Report::new(self.output)
    }
}

/// Hash/filter/case/walk settings shared by `update`, `compare`, and `sync`.
///
/// Built from raw CLI strings via [`CommonOpts::try_from`], so the patterns and
/// algorithms are already compiled and validated by the time a command runs.
#[derive(Debug, Clone)]
pub struct CommonOpts {
    /// Algorithms to record; empty means `--hash-all-of none` (size+mtime only).
    pub algos: Vec<String>,
    /// Which stat fields may settle a pair without a digest.
    ///
    /// See [`StatTrust`]. Trusting both is the default and is what makes the laziness
    /// work; the flags take one field away at a time, and both together are "disable the
    /// short circuit".
    pub stat: StatTrust,
    /// Whether every one of `algos` must be available, or only one.
    ///
    /// Beside `algos` rather than inside it, because it is a *rule about the
    /// request* and not part of it: the same list is meaningful under both modes,
    /// and folding the mode into the list would make "which algorithms" and "how
    /// many of them" the same value.
    pub hash_mode: HashMode,
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
    /// `--dry-run`: write nothing at all.
    ///
    /// Lives here rather than on a command because it applies to all four and
    /// there must be exactly one definition. `sync` additionally threads it into
    /// its rename and apply phases, which are the only filesystem writes anywhere
    /// in the crate.
    pub dry_run: bool,
}

impl TryFrom<CommonArgs> for CommonOpts {
    type Error = anyhow::Error;

    fn try_from(a: CommonArgs) -> Result<Self> {
        // One list, one mode. The two flags are mutually exclusive at the clap
        // layer, so which one the user typed is decided by which is non-empty
        // rather than by re-checking for a conflict here — a second check would be a
        // second place for the two to disagree.
        //
        // The asymmetry is deliberate: `parse_any_of` rejects `none`, and
        // `parse_hash_list` does not, because the rules differ. Under all-of an
        // empty request is a mode; under any-of it is not a request at all.
        let (hash_mode, algos) = if !a.hash_any_of.is_empty() {
            (HashMode::AnyOf, parse_any_of(&a.hash_any_of)?)
        } else {
            (HashMode::AllOf, parse_hash_list(&a.hash_all_of)?)
        };
        // Two independent switches on the short circuit's two inputs, and both
        // default to trusting. Taken as flags rather than as a tri-state, so
        // "trust nothing" is the composition and there is no third spelling to keep
        // in sync with the first two.
        let stat = StatTrust::default()
            .without_size_if(a.no_trust_size)
            .without_mtime_if(a.no_trust_mtime);
        // Distrusting a stat field is a request to decide the pair some **other** way.
        // With no algorithm requested there is no other way: no digest is computed, so
        // `Required` is empty, so the diff has nothing to consult and a difference in a
        // distrusted field has nowhere to go. The pair would then be reported **equal**
        // having had its content compared by nobody — and exit 0 on a changed file is
        // the one failure worse than refusing to run.
        //
        // Refused rather than warned, which is the opposite of how `update` treats flags
        // it cannot honour. The distinction is whether the verdict stays right: ignoring
        // a flag that does not apply still produces a correct answer, whereas ignoring
        // this one produces a wrong one, and a warning would then be describing a
        // choice the user believes protects them.
        //
        // Checked here, once, for every subcommand — the combination is a property of the
        // *request* rather than of the command receiving it, so a script passing the
        // flags everywhere should learn about it the same way whichever one it hit.
        if algos.is_empty() && stat.distrusts_any() {
            let mut named: Vec<&str> = Vec::new();
            if a.no_trust_size {
                named.push("--no-trust-size");
            }
            if a.no_trust_mtime {
                named.push("--no-trust-mtime");
            }
            anyhow::bail!(
                "{} says a stat field is not evidence of a difference, which only \
                 means something if something else can decide the pair. \
                 --hash-all-of none requests no digest, so there is nothing else: the \
                 difference would have nothing to fall back on and the pair would be \
                 reported equal without its content having been compared by anyone.\n\n\
                 To ask for this, one of:\n  \
                   - name a digest, which is what would settle the pair: \
                 --hash-all-of md5\n  \
                   - or trust stat, which is what --hash-all-of none is for: drop \
                 {}",
                named.join(" and "),
                named.join(" and "),
            );
        }
        Ok(Self {
            algos,
            hash_mode,
            stat,
            includes: compile_patterns(&a.include)?,
            excludes: compile_patterns(&a.exclude)?,
            case_sensitive: a.case_sensitive,
            max_depth: a.max_depth,
            ignore_cache: a.ignore_cache,
            dry_run: a.dry_run,
        })
    }
}

/// How a single folder scan treats the cache and the filesystem.
///
/// One flag for the cache, not two: "reuse a matching entry" and "re-read it" are
/// the same decision, and a caller asking for both would be asking one question
/// twice.
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
    /// - *decisions* are identical — a dry run still stats the tree, still reads
    ///   cached digests, and still hashes whatever the planner cannot settle from
    ///   stat alone. `can_hash_from_disk` stays true for exactly this reason:
    ///   suppressing those reads would change the verdict, not merely the side
    ///   effects.
    /// - *writes* are absent — no cache mutation, no cache creation, no backup,
    ///   and for a command that moves files, no filesystem change.
    ///
    /// The wording matters because the first half is the point. A dry run that
    /// skipped the hashing would be cheaper and *wrong*: it would report the
    /// answer to a different question.
    ///
    /// This field is the cache half. A command that also moves files threads its
    /// own `dry_run` into those phases (`sync` does); a command that never writes
    /// files has no second half, and this field is the whole of it — which is why
    /// `compare-self` sets it unconditionally rather than reading a flag.
    ///
    /// How the cache is *opened* under this flag is
    /// [`crate::effective::open_folder_cache`]'s decision, so no command can
    /// honour it incorrectly by opening the wrong kind of handle.
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
    /// Copy worker threads; must be >= 1.
    pub jobs: usize,
    /// `--dry-run` is `common.dry_run`. Read it from there rather than keeping a
    /// copy here: the cache half and the filesystem half are the same flag, and
    /// two fields for one flag is how they drift apart.
    pub common: CommonOpts,
}
