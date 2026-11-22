//! [`CommonOpts`], [`LogCtx`] and [`ScanMode`]: the settings a run shares.
//!
//! Everything here is common to **every** subcommand, which is what keeps the
//! flags one flag. `CommonOpts` is built once by `TryFrom<CommonArgs>` — so the
//! patterns and algorithms are already compiled and validated by the time a
//! command runs — and `ScanMode` is the same idea one level down: how a single
//! folder scan treats the cache and the filesystem.

use anyhow::{Result, bail};
use std::num::NonZeroUsize;
use std::path::PathBuf;

use girpr_log::Level;
use girpr_value_ranges::{RangeFact, check_in_range};

use crate::cache::{CommitLimits, TestCacheBackend};
use crate::cli::{CommonArgs, RwArgs};
use crate::filter::{Filter, compile_patterns};
use crate::hash::{HashAlg, parse_any_of, parse_hash_list};
use crate::planner::{HashMode, StatTrust};
use crate::report::{OutputFormat, Report, ReportSink};
use crate::rw::RwLimits;

/// The process-global test hooks, gathered into one value that a caller can
/// inject instead of reaching for the environment.
///
/// Every field here was once read straight from a `GIRSYNC_TEST_*` variable,
/// and several were `OnceLock`-cached, which is why a test that needed one had
/// to spawn a child. Resolving them once into a struct — from the environment
/// for a production/CLI run, or built by hand for an in-process test — removes
/// that reason: a caller that already drives a command in-process can now
/// inject a failure, a gate or a small threshold as data.
///
/// [`Default`] is all-off: no failure, no gate, no injected commit policy, no
/// heartbeat override, and `no_fsync` false (a copied file is flushed). That is
/// the safe answer for a hand-built hook value. [`TestHooks::from_env`] is what
/// the CLI uses, and it preserves the old environment behaviour — including
/// `.config/nextest-test-env` setting `GIRSYNC_TEST_NO_FSYNC` and
/// `GIRSYNC_TEST_DURABILITY` — by reading the same variables the old hook sites
/// read.
///
/// The production code paths read only these fields, never `std::env`, so an
/// injected value cannot be blocked by a stale `OnceLock` and two runs in one
/// process cannot see each other's hooks.
#[derive(Debug, Clone, Default)]
pub struct TestHooks {
    /// Relative path whose copy must fail before it writes anything
    /// (`GIRSYNC_TEST_FAIL_COPY`). `None` matches nothing.
    pub fail_copy: Option<String>,
    /// Relative path whose post-copy flush must fail
    /// (`GIRSYNC_TEST_FAIL_FLUSH`). `None` matches nothing.
    pub fail_flush: Option<String>,
    /// Skip the per-file `sync_all` after a copy (`GIRSYNC_TEST_NO_FSYNC`).
    pub no_fsync: bool,
    /// Relative path whose case-fix rename fails before its temporary is
    /// claimed (`GIRSYNC_TEST_FAIL_RENAME`).
    pub fail_rename_before_claim: Option<String>,
    /// Relative path whose case-fix rename fails at step 1
    /// (`GIRSYNC_TEST_FAIL_RENAME_STEP1`).
    pub fail_rename_step1: Option<String>,
    /// Relative path whose case-fix rename fails at step 2
    /// (`GIRSYNC_TEST_FAIL_RENAME_STEP2`).
    pub fail_rename_step2: Option<String>,
    /// The `<wait>:<on>` copy rendezvous (`GIRSYNC_TEST_GATE_COPY`).
    pub gate_copy: Option<String>,
    /// The injected commit policy for the copy phase's batched cache write.
    /// `None` leaves it to the environment (see `CommitLimits::from_env_opt`).
    pub commit: Option<CommitLimits>,
    /// The walk's heartbeat period (`GIRSYNC_TEST_HEARTBEAT_ENTRIES`), folded
    /// in from the knob T15 exposed on [`ScanMode`]. `None` leaves the period to
    /// the environment (and the production constant when that is unset).
    pub heartbeat_entries: Option<usize>,
}

impl TestHooks {
    /// Resolve the hooks from the environment — the fallback that keeps the
    /// CLI and `.config/nextest-test-env` working exactly as before.
    pub fn from_env() -> Self {
        let truthy = |name: &str| {
            matches!(
                std::env::var(name).as_deref(),
                Ok("1") | Ok("true") | Ok("yes") | Ok("on")
            )
        };
        let positive = |name: &str| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|&n| n > 0)
        };
        Self {
            fail_copy: std::env::var("GIRSYNC_TEST_FAIL_COPY").ok(),
            fail_flush: std::env::var("GIRSYNC_TEST_FAIL_FLUSH").ok(),
            no_fsync: truthy("GIRSYNC_TEST_NO_FSYNC"),
            fail_rename_before_claim: std::env::var("GIRSYNC_TEST_FAIL_RENAME").ok(),
            fail_rename_step1: std::env::var("GIRSYNC_TEST_FAIL_RENAME_STEP1").ok(),
            fail_rename_step2: std::env::var("GIRSYNC_TEST_FAIL_RENAME_STEP2").ok(),
            gate_copy: std::env::var("GIRSYNC_TEST_GATE_COPY").ok(),
            commit: CommitLimits::from_env_opt(),
            heartbeat_entries: positive("GIRSYNC_TEST_HEARTBEAT_ENTRIES"),
        }
    }

    /// Fail the copy of `rel`.
    pub fn with_fail_copy(mut self, rel: &str) -> Self {
        self.fail_copy = Some(rel.to_string());
        self
    }

    /// Fail the flush of `rel`.
    pub fn with_fail_flush(mut self, rel: &str) -> Self {
        self.fail_flush = Some(rel.to_string());
        self
    }

    /// Choose whether copies skip their per-file flush.
    pub fn with_no_fsync(mut self, no_fsync: bool) -> Self {
        self.no_fsync = no_fsync;
        self
    }

    /// Fail the case-fix rename of `rel` before its temporary is claimed.
    pub fn with_fail_rename_before_claim(mut self, rel: &str) -> Self {
        self.fail_rename_before_claim = Some(rel.to_string());
        self
    }

    /// Fail the case-fix rename of `rel` at step 1.
    pub fn with_fail_rename_step1(mut self, rel: &str) -> Self {
        self.fail_rename_step1 = Some(rel.to_string());
        self
    }

    /// Fail the case-fix rename of `rel` at step 2.
    pub fn with_fail_rename_step2(mut self, rel: &str) -> Self {
        self.fail_rename_step2 = Some(rel.to_string());
        self
    }

    /// Ask for the `<wait>:<on>` copy rendezvous.
    pub fn with_gate_copy(mut self, spec: &str) -> Self {
        self.gate_copy = Some(spec.to_string());
        self
    }

    /// Inject the copy phase's commit policy.
    pub fn with_commit(mut self, limits: CommitLimits) -> Self {
        self.commit = Some(limits);
        self
    }

    /// Fix the walk's heartbeat period.
    pub fn with_heartbeat_entries(mut self, entries: usize) -> Self {
        self.heartbeat_entries = Some(entries);
        self
    }
}

/// How the run reports itself: the log knobs and the stdout format.
///
/// Carried to every command for two reasons and no others. The `--log-*` values
/// exist so the per-command span can stamp them, and `output` is the one thing a
/// command needs in order to build its [`Report`] — a report format chosen here
/// and rebuilt per command would be four independent reads of the same flag.
///
/// The values come from the resolved [`girpr_log::LogPlan`], not from the raw
/// flags: the file's path is only known once the flag, the environment
/// variable and the timestamped default have been weighed against each other,
/// and a span that named a different file from the one being written would be
/// a span lying.
///
/// The two levels are [`Level`]s rather than strings: a span that stamped
/// `level=INFO` beside a filter that said `info` would be two spellings of one
/// fact, and a reader would have to know they were the same.
#[derive(Debug, Clone)]
pub struct LogCtx {
    /// The console's verbosity, echoed into spans.
    pub level: Level,
    /// The log file's verbosity, echoed into spans.
    pub file_level: Level,
    /// Where the log file went, echoed into spans — or `None`, when there is no
    /// log file.
    ///
    /// A debug build writes no file unless it is asked to, and `--no-log-file`
    /// turns one off in any build, so "no file" is a state a span has to be
    /// able to say out loud rather than a path that is always there.
    pub file: Option<PathBuf>,
    /// `--output`: how stdout records are rendered.
    pub output: OutputFormat,
    /// Where the command's records go. `Stdout` in production; a test that
    /// drives the CLI in-process sets a capture sink so it can assert on the
    /// records a parser would otherwise read from stdout.
    pub report_sink: ReportSink,
}

impl LogCtx {
    /// Span-friendly rendering of the log file's path.
    pub fn file_display(&self) -> String {
        match &self.file {
            Some(path) => path.display().to_string(),
            None => "none".to_string(),
        }
    }

    /// The stdout writer for this run, in the requested format and sink.
    pub fn report(&self) -> Report {
        Report::with_sink(self.output, self.report_sink.clone())
    }
}

/// Hash/filter/case/walk settings shared by `update`, `compare`, and `sync`.
///
/// Built from raw CLI strings via [`CommonOpts::try_from`], so the patterns and
/// algorithms are already compiled and validated by the time a command runs.
#[derive(Debug, Clone)]
pub struct CommonOpts {
    /// Algorithms to record; empty means `--hash-all-of none` (size+mtime only).
    ///
    /// **A list, not a set, and deliberately.** `--hash-any-of`'s tie-break is the
    /// user's flag order, so the request has to keep the order it was typed in;
    /// [`crate::hash::AlgoSet`] is the per-path *answer* and has no order of its
    /// own. It is one value per run, so keeping it ordered costs nothing — the
    /// per-path problem is solved by the set below it, not by flattening this.
    pub algos: Vec<HashAlg>,
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
    /// `--why`: put a reason tag on every `CHANGED` record.
    ///
    /// A reporting knob and nothing else: it reaches [`crate::report::visit_verdict`] and
    /// nothing downstream, so it cannot move the exit code. It lives beside the other
    /// output-shaping settings rather than in the commands that use it, because the
    /// question "does this run explain itself?" is a property of the run and not of the
    /// command that happened to be invoked.
    pub why: bool,
    /// `--show-identical`: also report the pairs that came out equal.
    ///
    /// Threaded to [`crate::diff::diff_maps`] rather than used at the printing site,
    /// because the equal set is a property of the *diff*: it is the classification the
    /// diff already reached, and deciding what to print about it at the reporter would
    /// mean asking a question the diff has already answered. It is off by default
    /// because it is one record per equal file.
    pub show_identical: bool,
    /// `--include` / `--exclude` globs, compiled and ready to match.
    ///
    /// The patterns and the options they are matched with live together, so a
    /// path is always tested against the rules this run was configured with.
    /// Empty includes mean "everything"; the excludes win over them.
    pub filter: Filter,
    /// Default false: insensitive, with rename/abort rules.
    ///
    /// **A run-wide mode, not a filter property.** It also selects the cache's
    /// case flag, the planner's pairing, the rename pass and the mkdir check,
    /// so it stays here rather than inside [`Filter`], where it would look as
    /// though it belonged to globbing alone.
    ///
    /// The filter's matching options are built from this value, so the two have
    /// to agree: use [`CommonOpts::set_case_sensitive`] to change the mode
    /// rather than assigning the field.
    pub case_sensitive: bool,
    /// Walk depth cap for the whole tree (default 10).
    pub max_depth: usize,
    /// Back up + delete the cache before opening, forcing a rebuild.
    pub ignore_cache: bool,
    /// What a run copies aside, and how much of it it keeps.
    ///
    /// A policy rather than a fixed step, because the copies are a recovery
    /// aid rather than part of the answer: the live cache is authoritative,
    /// and nothing reads a backup or an `old-*` snapshot back. See
    /// [`BackupPolicy`].
    pub backup: BackupPolicy,
    /// `--dry-run`: write nothing at all.
    ///
    /// Lives here rather than on a command because it applies to all four and
    /// there must be exactly one definition. `sync` additionally threads it into
    /// its rename and apply phases, which are the only filesystem writes anywhere
    /// in the crate.
    pub dry_run: bool,
    /// `--rw-threads*`: how many file reads and writes may be in flight.
    ///
    /// A run-wide *resource*, so it lives beside the other run-wide settings
    /// rather than on a single command. Phase C and the copy phase are the two
    /// places that consume it, and both are reached from every subcommand.
    pub rw: RwLimits,
    /// Test-only: `Memory` opens folder caches in redb's in-memory backend
    /// instead of the real `<root>/girpr-cache` file. `File` (default) opens
    /// the real file, which production and cache-focused tests keep.
    ///
    /// **The field is the answer.** `GIRSYNC_TEST_CACHE=memory` is read once,
    /// at command start, by [`CommonOpts::try_from`] — the CLI path — and
    /// nowhere else; a caller that builds these options by hand gets exactly
    /// what it put here, whatever the environment holds.
    pub test_cache: TestCacheBackend,
}

impl CommonOpts {
    /// Set the run's case mode, and rebuild the filter's matching options to
    /// match it.
    ///
    /// The filter caches the options it matches with, and the case mode is one
    /// of them, so the two move together: changing the mode without rebuilding
    /// the filter would leave the filter matching under the old one.
    pub fn set_case_sensitive(&mut self, case_sensitive: bool) {
        self.case_sensitive = case_sensitive;
        self.filter = Filter::new(
            self.filter.includes().to_vec(),
            self.filter.excludes().to_vec(),
            case_sensitive,
        );
    }
}

/// What a run copies aside, and how much of it it keeps.
///
/// A **policy**, not a fixed step, because the copies are a recovery aid and
/// not part of the answer: the live `<root>/girpr-cache` is authoritative, and
/// a `girpr-cache-backup-*` copy or a `girpr-cache-old-*` snapshot is never
/// read back automatically. So this decides what a run *leaves behind*, and
/// nothing about what it reports.
///
/// The default is "back up, keep everything" — the copies are cheap insurance
/// and nobody has to think about them. `--no-backup` turns the copying off
/// and `--backup-retain N` bounds what accumulates; both are opt-in, because
/// a run that quietly stopped keeping a recovery point would be a surprise in
/// the direction that hurts.
#[derive(Debug, Clone, Copy, Default)]
pub struct BackupPolicy {
    /// `--no-backup`: copy nothing aside.
    pub disabled: bool,
    /// `--backup-retain N`: bound each prefix to N copies.
    ///
    /// The oldest copies are removed until the folder holds N of them. A
    /// copy another run is still using is kept as well, and it does not
    /// spend the removal budget: the bound walks on to the next copy in age
    /// order, and that one may be among the newest. The copies a folder
    /// keeps are therefore not necessarily its newest, and a folder holds
    /// more than N while there are not enough copies the bound may remove
    /// to make the count up.
    ///
    /// `None` keeps them all, which is the default.
    pub retain: Option<NonZeroUsize>,
}

impl BackupPolicy {
    /// Whether this run copies a cache aside at all.
    ///
    /// The one question every backup site asks, so that "no backups" is
    /// answered in one place rather than at each site that would otherwise
    /// have to remember the flag.
    pub fn wants_backup(&self) -> bool {
        !self.disabled
    }
}

impl TryFrom<RwArgs> for RwLimits {
    type Error = anyhow::Error;

    /// Resolve the flag combination into the two shapes the limiter has.
    ///
    /// | flags given | limits |
    /// | --- | --- |
    /// | *(none)* | `Shared(1)` |
    /// | `--rw-threads N` | `Shared(N)` |
    /// | `--rw-dual-drive` | `Split { src: 1, dst: 1 }` |
    /// | `--rw-threads-src S` | `Split { src: S, dst: 1 }` |
    /// | `--rw-threads-dst D` | `Split { src: 1, dst: D }` |
    /// | `--rw-threads-src S --rw-threads-dst D` | `Split { src: S, dst: D }` |
    ///
    /// The combinations that are not rows of that table are already refused by
    /// clap's `conflicts_with_all`, so they exit `2` before reaching here. The
    /// match still has to be total, because this is a public `TryFrom` and a
    /// caller can hand it an `RwArgs` it built by hand — so the impossible arm
    /// names the conflict rather than being an `unreachable!` that panics on a
    /// caller who did nothing wrong.
    ///
    /// `0` is refused here rather than by clap, so it exits `3` with a message
    /// naming the flag — the code the README documents for a runtime usage
    /// mistake, as distinct from the `2` a parse error gets. A clap range validator
    /// would have made it exit `2`, which reads as "you typed it wrong" rather
    /// than "you asked for zero of something".
    fn try_from(a: RwArgs) -> Result<Self> {
        let one = RwLimits::one();
        let nz = |flag: &str, n: usize| -> Result<NonZeroUsize> {
            let bad = || {
                anyhow::anyhow!(
                    "{flag} must be >= 1 (got {n}); it is how many reads or writes may be in flight at once"
                )
            };
            match check_in_range(n, 1, None) {
                RangeFact::OutOfRange { .. } => Err(bad()),
                RangeFact::InRange => NonZeroUsize::new(n).ok_or_else(bad),
            }
        };
        match (
            a.rw_threads,
            a.rw_threads_src,
            a.rw_threads_dst,
            a.rw_dual_drive,
        ) {
            (None, None, None, false) => Ok(RwLimits::Shared(one)),
            (Some(n), None, None, false) => Ok(RwLimits::Shared(nz("--rw-threads", n)?)),
            (None, None, None, true) => Ok(RwLimits::Split { src: one, dst: one }),
            (None, Some(s), None, false) => Ok(RwLimits::Split {
                src: nz("--rw-threads-src", s)?,
                dst: one,
            }),
            (None, None, Some(d), false) => Ok(RwLimits::Split {
                src: one,
                dst: nz("--rw-threads-dst", d)?,
            }),
            (None, Some(s), Some(d), false) => Ok(RwLimits::Split {
                src: nz("--rw-threads-src", s)?,
                dst: nz("--rw-threads-dst", d)?,
            }),
            (threads, src, dst, dual) => {
                let show =
                    |v: Option<usize>| v.map(|n| n.to_string()).unwrap_or_else(|| "-".into());
                anyhow::bail!(
                    "--rw-threads={}, --rw-threads-src={}, --rw-threads-dst={}, \
                     --rw-dual-drive={} cannot be combined: name either one counter \
                     for both sides (--rw-threads N) or one per side \
                     (--rw-threads-src S --rw-threads-dst D), or assert two physical \
                     drives (--rw-dual-drive)",
                    show(threads),
                    show(src),
                    show(dst),
                    dual,
                )
            }
        }
    }
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
        // The test-only cache backend, resolved once, here, at command start.
        //
        // This is the only place the environment is read for it: every later
        // reader asks the field, so a run cannot change its mind halfway
        // through, and two cache opens in one run cannot disagree. The
        // subprocess suites set `GIRSYNC_TEST_CACHE=memory` on the *child*,
        // and the child runs the CLI — which comes through here — so they
        // still get the in-memory backend.
        //
        // Anything other than exactly `memory` means the real file, which is
        // what production uses and what the cache-focused suites need.
        let test_cache = if std::env::var("GIRSYNC_TEST_CACHE").as_deref() == Ok("memory") {
            TestCacheBackend::Memory
        } else {
            TestCacheBackend::File
        };
        Ok(Self {
            algos,
            hash_mode,
            stat,
            why: a.why,
            show_identical: a.show_identical,
            filter: Filter::new(
                compile_patterns(&a.include)?,
                compile_patterns(&a.exclude)?,
                a.case_sensitive,
            ),
            case_sensitive: a.case_sensitive,
            max_depth: a.max_depth,
            ignore_cache: a.ignore_cache,
            backup: backup_policy(a.no_backup, a.backup_retain)?,
            dry_run: a.dry_run,
            rw: RwLimits::try_from(a.rw)?,
            test_cache,
        })
    }
}

/// Resolve the two backup flags into one policy.
///
/// `--backup-retain 0` is refused here rather than by clap, so it exits `3`
/// with a message naming the flag — the code the README documents for a
/// runtime usage mistake, as distinct from the `2` a parse error gets.
/// Keeping *none* of the copies is not a retention request, it is a
/// contradiction: the user asked for a bound and named a bound of zero, and
/// `--no-backup` is the flag that says what they meant.
fn backup_policy(disabled: bool, retain: Option<usize>) -> Result<BackupPolicy> {
    let retain = match retain {
        Some(n) => match NonZeroUsize::new(n) {
            Some(nz) => Some(nz),
            None => bail!(
                "--backup-retain must be >= 1 (got {n}); it is how many backups or \
                 snapshots to keep per prefix, and --no-backup is how you ask for none"
            ),
        },
        None => None,
    };
    Ok(BackupPolicy { disabled, retain })
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
    /// The walk's heartbeat period, when a caller injects one.
    ///
    /// `None` (the production default from [`ScanMode::new`]) leaves the period
    /// to the environment, which with the test hook unset is
    /// [`crate::scan::HEARTBEAT_ENTRIES`]. `Some(n)` fixes it for this mode so a small fixture
    /// can trip the heartbeat. It is a *period*, not a count: the walk treats
    /// `Some(0)` as `1`, so the modulo cannot divide by zero.
    pub heartbeat_entries: Option<usize>,
}

impl ScanMode {
    /// Both decisions are explicit: neither has a default, so a caller cannot
    /// omit the dry-run half and silently persist a run meant to write nothing.
    ///
    /// The heartbeat period is left to the environment (`None`), so production
    /// and the child-process suites that set `GIRSYNC_TEST_HEARTBEAT_ENTRIES`
    /// keep exactly the behaviour this constructor always had.
    pub const fn new(no_trust_cached_hashes: bool, dry_run: bool) -> Self {
        Self {
            no_trust_cached_hashes,
            dry_run,
            heartbeat_entries: None,
        }
    }

    /// Fix this mode's walk heartbeat period, overriding the environment.
    ///
    /// The production path never calls this: `None` keeps the environment
    /// fallback (and therefore [`crate::scan::HEARTBEAT_ENTRIES`] when unset). A test that
    /// injects a small period reads the counters off the returned
    /// [`crate::effective::SideScan`] instead of parsing log events.
    pub fn with_heartbeat_entries(mut self, heartbeat_entries: usize) -> Self {
        self.heartbeat_entries = Some(heartbeat_entries);
        self
    }
}
