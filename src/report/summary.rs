use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use super::record::Record;
use crate::repair::scope::ledger::Meter;

/// A byte ceiling whose "no ceiling" sentinel is `u64::MAX`.
///
/// `AtomicU64`'s derived `Default` is **zero**, so a `Summary` that derived
/// `Default` would start every run at a ceiling of zero — refusing the first
/// byte. This newtype owns the sentinel so the rest of [`Summary`] can derive
/// its `Default`.
#[derive(Debug)]
pub struct Ceiling(AtomicU64);

impl Default for Ceiling {
    fn default() -> Self {
        Self(AtomicU64::new(u64::MAX))
    }
}

impl Ceiling {
    /// The stored ceiling; `u64::MAX` means "no ceiling".
    pub(crate) fn load(&self, order: Ordering) -> u64 {
        self.0.load(order)
    }

    /// Install a ceiling; `u64::MAX` means "no ceiling".
    pub(crate) fn store(&self, value: u64, order: Ordering) {
        self.0.store(value, order);
    }

    /// A plain-value copy, like [`Summary::snapshot`].
    fn snapshot(&self) -> Self {
        Self(AtomicU64::new(self.load(Ordering::Relaxed)))
    }
}

/// Run counters. The atomic fields are shared with the per-file tasks, hence
/// [`Summary::snapshot`] to read them back out once the tasks are joined.
#[derive(Debug, Default)]
pub struct Summary {
    pub files_total: u64,
    pub files_skipped: AtomicU64,
    pub files_repaired: AtomicU64,
    pub files_failed: AtomicU64,
    /// The network meter: the bytes that crossed the wire (**live**) and the
    /// bytes of the files that were actually repaired (**settled**).
    ///
    /// The reported `download_bytes` is the **settled** figure: it is a
    /// function of the install, not of the transport, because an attempt that
    /// failed and was retried settles nothing. The `--max-download-bytes`
    /// ceiling bounds the **live** figure, which is the traffic the run
    /// actually caused — the bytes are spent whether they turn out to be good
    /// or not.
    ///
    /// See [`Meter`] for the rule that tells the two apart.
    pub download: Meter,
    /// Unified cleanup counter (Collapse files-cleanup parity): every byte
    /// deleted by `--purge-before` / `--purge-after` (temps, orphans,
    /// deprecated files) lands here. No split accounting.
    pub deleted_extra_bytes: AtomicU64,
    /// Conditional keys — see [`SummaryFlags`]. Set once, before the tasks
    /// start; plain values, so no atomics are needed.
    pub flags: SummaryFlags,
    /// Which runtime ceiling bound, if any. `0` = none; the numbers are
    /// [`crate::repair::scope::ledger::StoppedBy`]'s. A single marker rather
    /// than a flag per ceiling, because the *value* the `SUMMARY` reports is
    /// the flag that bound — see [`SummaryFlags::stopped`].
    pub stopped_by: AtomicU8,
    /// The disk meter: the bytes that landed in the install (**live**) and the
    /// bytes written from sources that passed their gate (**settled**).
    ///
    /// The reported `write_bytes` is the **settled** figure: it is a function
    /// of the install, not of the transport, because a chunk whose digest
    /// fails contributes nothing. The `--max-write-bytes` ceiling bounds the
    /// **live** figure, which is the disk the run actually took: a chunk that
    /// fails its digest has already been written, and the bytes are spent
    /// whether they are good or not.
    ///
    /// See [`Meter`] for the rule that tells the two apart.
    pub write: Meter,
    /// `--fail-fast` armed.
    pub fail_fast: AtomicBool,
    /// `--fail-fast` has tripped: stop starting files, stop retrying.
    pub aborted: AtomicBool,
    /// `--max-ram-bytes`; see [`Ceiling`] for the `u64::MAX` sentinel.
    pub ram_limit: Ceiling,
    /// RAM currently reserved for staging the bytes a file has to move
    /// inside itself. Live, not cumulative: a file gives its reservation
    /// back when it is done.
    pub ram_reserved: AtomicU64,
    /// The peak staged payload of the run — the most RAM that was staged at
    /// once. This is the number the `SUMMARY ram_bytes=` key reports.
    pub ram_bytes: AtomicU64,
    /// Files repaired **in place**: the target was written where it lies.
    pub files_inplace: AtomicU64,
    /// Files repaired through `{path}_tmp`.
    pub files_temp: AtomicU64,
    /// Bytes the file's size says the repair did **not** have to write: the
    /// ranges that were already correct in place, and a `{path}_tmp` prefix a
    /// resumed run kept instead of rewriting. The disk claim of the run, in one
    /// number.
    ///
    /// A kept `_tmp` prefix is **not verified yet** — it is unverified bytes the
    /// repair chose not to rewrite, and only the final whole-file MD5 gate
    /// proves them. A file whose prefix fails that gate is deleted and rebuilt,
    /// so those bytes were counted here but did not survive. In-place kept
    /// ranges, by contrast, were checked as they were read.
    ///
    /// It is the complement of the **settled** writes — the bytes that landed
    /// in the install from a source that passed its gate — and the settled
    /// writes are the *file's*, accumulated across its attempts: a chunk an
    /// abandoned attempt wrote is in the install, so the next attempt finds it
    /// correct and does not write it again. It was the last attempt's figure
    /// that made this counter over-report, and a run that had to retry claimed
    /// it had touched fewer bytes than it had.
    pub kept_bytes: AtomicU64,
}

impl Summary {
    /// A run with no counters moved and no ceiling requested.
    pub fn new() -> Self {
        Self::default()
    }

    /// Plain-value copy, for handing a shared `Summary` back to the caller.
    pub fn snapshot(&self) -> Self {
        Self {
            files_total: self.files_total,
            files_skipped: AtomicU64::new(self.files_skipped.load(Ordering::Relaxed)),
            files_repaired: AtomicU64::new(self.files_repaired.load(Ordering::Relaxed)),
            files_failed: AtomicU64::new(self.files_failed.load(Ordering::Relaxed)),
            download: self.download.snapshot(),
            deleted_extra_bytes: AtomicU64::new(self.deleted_extra_bytes.load(Ordering::Relaxed)),
            flags: self.flags,
            // A snapshot must not lose the ceiling or the stop marker: the
            // caller reads both *after* the tasks have been joined, which is
            // the only moment either can be meaningful.
            stopped_by: AtomicU8::new(self.stopped_by.load(Ordering::Relaxed)),
            write: self.write.snapshot(),
            fail_fast: AtomicBool::new(self.fail_fast.load(Ordering::Relaxed)),
            aborted: AtomicBool::new(self.aborted.load(Ordering::Relaxed)),
            ram_limit: self.ram_limit.snapshot(),
            ram_reserved: AtomicU64::new(self.ram_reserved.load(Ordering::Relaxed)),
            ram_bytes: AtomicU64::new(self.ram_bytes.load(Ordering::Relaxed)),
            files_inplace: AtomicU64::new(self.files_inplace.load(Ordering::Relaxed)),
            files_temp: AtomicU64::new(self.files_temp.load(Ordering::Relaxed)),
            kept_bytes: AtomicU64::new(self.kept_bytes.load(Ordering::Relaxed)),
        }
    }
}

/// The **conditional** `SUMMARY` keys — see
/// [`docs/04-scope-limitation-spec.md`](../docs/04-scope-limitation-spec.md).
///
/// Every one is emitted *only* when its flag is active — the same discipline
/// `girsync` used for `identical=N`: an unflagged run's output is byte-identical
/// to a run that never passes the flags at all. That is what keeps the machine
/// contract non-breaking when this surface grows.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SummaryFlags {
    /// Any resource ceiling active (`--max-files`, `--max-download-bytes`, …).
    pub limited: bool,
    /// Any path scope active (`--include` / `--exclude`).
    pub scoped: bool,
    /// `--max-download-bytes`'s estimate for the (filtered) plan.
    pub plan_download_bytes: Option<u64>,
    /// The bytes that crossed the wire — the **live** download figure.
    ///
    /// Emitted only when `--max-download-bytes` is active, beside the settled
    /// `download_bytes`: the two are the same quantity in the happy path and
    /// differ exactly when the transport misbehaved, and a reader who has only
    /// one of them cannot tell a clean run from a lucky one.
    pub download_bytes_spent: Option<u64>,
    /// Bytes written into the install, when `--max-write-bytes` is active.
    pub write_bytes: Option<u64>,
    /// The bytes that landed in the install — the **live** write figure.
    ///
    /// Emitted only when `--max-write-bytes` is active, beside the settled
    /// `write_bytes`: the live one is what the ceiling bounds (the disk the
    /// run took), the settled one is what the install needed.
    pub write_bytes_spent: Option<u64>,
    /// Whether the **local** build's manifests loaded (`getBuild(local)`).
    /// `None` when no local version was known, so the key is absent on a fresh
    /// install and on a run that never had a version to look up.
    ///
    /// This is the one degradation that is otherwise invisible and yet changes
    /// the numbers: with no local manifest the reuse map is empty, so every
    /// chunk downloads, `download_bytes` rises, and `est_download_bytes` rises
    /// with it — while the run still exits `0`. A run that reused 8 chunks and
    /// fetched 2 would otherwise look exactly like one that fetched all 10. The
    /// fallback itself is correct (a tag the server no longer serves must cost
    /// more bytes, never give a wrong result); this key just makes the
    /// degradation assertable by a machine consumer instead of discoverable only
    /// by reading a `warn` line.
    pub local_manifest: Option<&'static str>,
    /// What a **runtime** hard stop cut short, vs a pre-flight refusal which
    /// wrote nothing at all.
    ///
    /// The value is the **flag** that bound — `--max-download-bytes`,
    /// `--max-write-bytes`, `--max-delete-bytes` — rather than a vocabulary
    /// word: it is self-describing, it scales with the number of ceilings, and
    /// it is the shape a script already has to write.
    pub stopped: Option<&'static str>,
    /// `config.ini` was deliberately not bumped, because a scoped (or
    /// `--purge-only`) run has deliberately not put the bytes in place.
    pub config_bumped_zero: bool,
    /// `--repair-mode`: the mode the user asked for, and with it the
    /// numbers that say which paths the run used. Emitted **only** when the
    /// flag was passed — the same discipline every other conditional key
    /// follows.
    ///
    /// The four keys travel together because they answer one question: "how
    /// did this run touch the disk?". `repair_mode` says what was asked
    /// for, `inplace` and `temp` say what actually happened, and
    /// `kept_bytes` says how many bytes did not have to be written at all.
    pub repair_mode: Option<&'static str>,
    /// Files repaired in place.
    pub files_inplace: Option<u64>,
    /// Files repaired through `{path}_tmp`.
    pub files_temp: Option<u64>,
    /// Bytes the repair did not rewrite: in-place ranges that were already
    /// correct, plus a resumed, not-yet-verified `{path}_tmp` prefix.
    ///
    /// The complement of the **settled** writes: see
    /// [`Summary::kept_bytes`].
    pub kept_bytes: Option<u64>,
    /// The peak staged payload — the most RAM that was staged at once.
    /// Emitted only when `--max-ram-bytes` was passed.
    pub ram_bytes: Option<u64>,
    /// The chunks that are wrong where they belong, over the whole plan.
    /// Emitted only when `--scan` was passed: without the scan nobody knows
    /// which chunks are damaged, only which files are.
    pub damaged_chunks: Option<u64>,
}

/// One conditional `SUMMARY` entry.
///
/// A [`Record`] field rather than a second rendering: appending to the record
/// puts the key on the text line and into the JSON object in the same step, so
/// the conditional tail cannot be extended on one plane and forgotten on the
/// other.
#[derive(Debug, Clone, Copy)]
pub enum Extra {
    Num(&'static str, u64),
    Word(&'static str, &'static str),
}

impl Extra {
    /// Only the rendering paths are used in production; `key` exists so a test
    /// can assert the fixed order of the conditional tail.
    #[cfg(test)]
    pub(crate) fn key(&self) -> &'static str {
        match self {
            Extra::Num(k, _) | Extra::Word(k, _) => k,
        }
    }

    pub(crate) fn put(self, rec: Record) -> Record {
        match self {
            Extra::Num(k, v) => rec.put(k, v),
            Extra::Word(k, v) => rec.put(k, v),
        }
    }
}

/// The conditional tail under construction.
///
/// The push order is the order the `SUMMARY` line prints — a contract — so it
/// is written down once, in [`conditional_extras`], with the `Extra` variant
/// noise left to these two methods.
#[derive(Default)]
struct Extras(Vec<Extra>);

impl Extras {
    fn num(&mut self, key: &'static str, value: u64) {
        self.0.push(Extra::Num(key, value));
    }

    fn word(&mut self, key: &'static str, value: &'static str) {
        self.0.push(Extra::Word(key, value));
    }
}

/// The conditional tail of a `SUMMARY`, in a fixed order so two runs' lines
/// diff cleanly. Empty (and therefore invisible) on an unflagged run.
pub(crate) fn conditional_extras(sum: &Summary) -> Vec<Extra> {
    let f = &sum.flags;
    let mut out = Extras::default();
    if f.limited {
        out.num("limited", 1);
    }
    if f.scoped {
        out.num("scoped", 1);
    }
    if f.scoped || f.config_bumped_zero {
        // A scoped (or `--purge-only`) run deliberately did not put the bytes
        // in place, so it does not advance config.ini. Emitted as a key
        // rather than left to be discovered by reading the file.
        out.num("config_bumped", 0);
    }
    if f.limited || f.scoped {
        out.num("plan_files", sum.files_total);
    }
    if let Some(n) = f.plan_download_bytes {
        out.num("plan_download_bytes", n);
    }
    // The spent figure comes before the settled one it belongs to: the pair
    // is one story, and the order says "what it spent, what it got".
    if let Some(n) = f.download_bytes_spent {
        out.num("download_bytes_spent", n);
    }
    if let Some(n) = f.write_bytes {
        out.num("write_bytes", n);
    }
    if let Some(n) = f.write_bytes_spent {
        out.num("write_bytes_spent", n);
    }
    // The repair-mode group: what was asked for, what happened, and how
    // many bytes did not have to be written.
    if let Some(mode) = f.repair_mode {
        out.word("repair_mode", mode);
        out.num("inplace", f.files_inplace.unwrap_or(0));
        out.num("temp", f.files_temp.unwrap_or(0));
        out.num("kept_bytes", f.kept_bytes.unwrap_or(0));
    }
    if let Some(n) = f.ram_bytes {
        out.num("ram_bytes", n);
    }
    if let Some(n) = f.damaged_chunks {
        out.num("damaged_chunks", n);
    }
    if let Some(s) = f.stopped {
        out.word("stopped", s);
    }
    // Last: this is a fact about the run, not a verdict about it, and it is the
    // key a reader needs in order to trust the two byte counts above it.
    if let Some(s) = f.local_manifest {
        out.word("local_manifest", s);
    }
    out.0
}
