//! The run's counters and every stdout line the automation contract promises.
//!
//! One rule lives here: **a stdout line is a [`Record`]**, not a string built at
//! the call site. A record is an event label plus named fields, held once and
//! rendered twice — `--output text` prints `REPORT total=…` and `--output json`
//! prints `{"event":"report","total":…}` from the same field list, so the two
//! planes cannot drift the way two independent `format!`s can.
//!
//! One record per line, rendered once, is what removes the possibility of a
//! record that is text on one plane and a differently-typed JSON object on the
//! other.
//!
//! Records reach stdout *and* the log, so a caller that captures either stream
//! sees the same `REPORT` / `PROGRESS` / `PLAN` / `SUMMARY` sequence. Building
//! records is separated from rendering them, so both planes are unit-testable
//! without a subscriber installed.
//!
//! Diagnostics are a different channel and do not come through here: they are
//! `tracing` events on stderr (and optionally a `TRACE` log file — see
//! [`crate::logging`]).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::hyp::{ChunkBuild, GameBranchPackage, GameConfig};
use crate::repair::scope::ledger::Meter;

/// How often the background reporter emits `PROGRESS` during long phases.
pub const PROGRESS_INTERVAL_SECS: u64 = 10;

/// How stdout records are rendered. Selected by the global `--output` flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum OutputFormat {
    /// One human-readable `key=value` line per record: `SUMMARY total=0 …`.
    /// The default, and the format every doc example and golden line is
    /// written against.
    #[default]
    Text,
    /// One JSON object per line (NDJSON), same records, same order:
    /// `{"event":"summary","total":0,…}`. No enclosing array, so a caller can
    /// stream it and recover the exact ordering.
    Json,
}

impl OutputFormat {
    /// The value as it appears in `--output`.
    pub fn as_str(&self) -> &'static str {
        match self {
            OutputFormat::Text => "text",
            OutputFormat::Json => "json",
        }
    }
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A field value on the data plane.
///
/// **Two variants, not three.** There is no boolean: every flag-shaped fact in
/// the contract is a `1`/`0` integer (`scoped=1`, `config_bumped=0`,
/// `limited=1`), with `SUMMARY` the documented shape. Leaving `Bool` out is what
/// stops a `true`-in-one-object / `1`-in-the-other collision from being
/// reintroduced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Val {
    Str(String),
    /// A count, a byte total, or the process exit code. `i64` rather than `u64`
    /// so one variant covers both the `u64` counters and the `i32` exit code;
    /// a counter that large would have exceeded the disk it is counting.
    Int(i64),
}

impl From<&str> for Val {
    fn from(s: &str) -> Self {
        Val::Str(s.to_string())
    }
}

impl From<String> for Val {
    fn from(s: String) -> Self {
        Val::Str(s)
    }
}

impl From<u64> for Val {
    fn from(v: u64) -> Self {
        Val::Int(v as i64)
    }
}

impl From<i32> for Val {
    fn from(v: i32) -> Self {
        Val::Int(v as i64)
    }
}

/// One stdout record: an event label plus its fields, held once and rendered
/// twice.
///
/// Field order is report order: it is the order a reader scans the text line in,
/// and the order the JSON keys are written in. Within one JSON object the keys
/// are still *names* — a consumer must read them by name, not by position — but
/// the rendering does not depend on that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Text label, e.g. `SUMMARY`. Lowercased to get the JSON `event`.
    label: &'static str,
    fields: Vec<(&'static str, Val)>,
}

impl Record {
    /// A record whose every field is keyed: `SUMMARY total=0 skipped=0 …`.
    ///
    /// Every `girpr` record has this shape. There is no positional variant
    /// because there is no record that wants one, and a constructor that could
    /// produce a line whose leading value is unkeyed is a constructor that can
    /// be used to break the parse plane again.
    pub fn keyed(label: &'static str) -> Self {
        Self {
            label,
            fields: Vec::new(),
        }
    }

    /// Append a field. Declaration order is report order, which is what both
    /// renderings preserve.
    pub fn put(mut self, key: &'static str, value: impl Into<Val>) -> Self {
        self.fields.push((key, value.into()));
        self
    }

    /// The text label, e.g. `SUMMARY`.
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// The JSON `event` name: the text label lowercased, so `SUMMARY`/`summary`
    /// and `PLAN`/`plan` cannot be spelled two ways.
    pub fn event(&self) -> String {
        self.label.to_ascii_lowercase()
    }

    /// The fields, in report order, without the `event` name.
    pub fn fields(&self) -> &[(&'static str, Val)] {
        &self.fields
    }

    /// The human line for this record, without the trailing newline:
    /// `SUMMARY total=0 skipped=0`.
    pub fn text(&self) -> String {
        let mut out = String::from(self.label);
        for (k, v) in &self.fields {
            out.push(' ');
            out.push_str(k);
            out.push('=');
            out.push_str(&bare(v));
        }
        out
    }

    /// The JSON object for this record, as one line: `{"event":"summary",…}`.
    ///
    /// Written pair by pair rather than through a `serde_json::Map` because that
    /// map is a `BTreeMap` unless the `preserve_order` feature is on, and turning
    /// it on is not this crate's call: cargo unifies features across the whole
    /// workspace build, so enabling it here would silently reorder the JSON
    /// another tool in this repository emits. The fields are few and all
    /// primitives, so the cost is a `to_string` per key and per value.
    pub fn json(&self) -> String {
        let mut s = String::with_capacity(64 + 24 * self.fields.len());
        s.push('{');
        s.push_str(&json_string("event"));
        s.push(':');
        s.push_str(&json_string(&self.event()));
        for (k, v) in &self.fields {
            s.push(',');
            s.push_str(&json_string(k));
            s.push(':');
            s.push_str(&json_value(v));
        }
        s.push('}');
        s
    }

    /// The line this record prints in `format`, without the newline.
    pub fn render(&self, format: OutputFormat) -> String {
        match format {
            OutputFormat::Text => self.text(),
            OutputFormat::Json => self.json(),
        }
    }
}

/// A value as it appears inside a text line: bare, never quoted.
fn bare(v: &Val) -> String {
    match v {
        Val::Str(s) => s.clone(),
        Val::Int(i) => i.to_string(),
    }
}

/// A JSON string literal, with the escapes a raw quote would need.
///
/// Only reached by a key or a [`Val::Str`] carrying a `"` or a control
/// character — a Windows artifact path has backslashes, which JSON allows
/// unescaped, so the common case is a memcpy.
fn json_string(s: &str) -> String {
    if s.bytes().all(|b| b >= 0x20 && b != b'"') {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        out.push_str(s);
        out.push('"');
        return out;
    }
    // `serde_json` is the authority on escaping; it cannot fail on a `str`.
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

/// A [`Val`] as a JSON literal, preserving its type.
fn json_value(v: &Val) -> String {
    match v {
        Val::Str(s) => json_string(s),
        Val::Int(i) => i.to_string(),
    }
}

/// Run counters. The atomic fields are shared with the per-file tasks, hence
/// [`Summary::snapshot`] to read them back out once the tasks are joined.
///
/// `Default` is written by hand rather than derived for exactly one reason: the
/// ceiling sentinel is `u64::MAX`, and a derived `Default` would start every run
/// at a ceiling of **zero** — i.e. refusing the first byte.
#[derive(Debug)]
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
    /// Set when the runtime download hard stop fired. An aborted run leaves
    /// the work it had done in place: the temp path's `_tmp` is resumed by
    /// length, and the in-place path's chunks that are already correct stay
    /// correct. Either way the next run is a smaller run, not a corrupt one.
    pub download_stopped: AtomicBool,
    /// `--max-write-bytes`, same `u64::MAX` sentinel as `download_limit`.
    pub write_limit: AtomicU64,
    /// The bytes written into the install **from sources that passed their
    /// gate** — a chunk's MD5, a reuse slice's MD5, a staged chunk's MD5, the
    /// scan's source verdict for an in-file copy, and the structural
    /// zero-fill.
    ///
    /// Reported as `write_bytes` **only** when `--max-write-bytes` is active —
    /// this is the cumulative counterpart of peak disk (`io_threads × largest
    /// file`), and it is invisible to `download_bytes`, which counts the
    /// network, not the disk.
    ///
    /// **It is a function of the input state.** A chunk whose digest fails
    /// contributes nothing: the bytes it landed are real, but they were never
    /// a source that passed a gate, and the run's answer to "how much did the
    /// repair rewrite" must not move because the network was flaky. The live
    /// counter that the ceiling reads is [`Self::write_bytes_spent`].
    pub write_bytes: AtomicU64,
    /// Every byte that landed in the install, gate or no gate.
    ///
    /// This is the number the `--max-write-bytes` ceiling bounds, because it
    /// is the disk the run actually took: a chunk that fails its digest has
    /// already been written, and the bytes are spent whether they are good or
    /// not. It is **not** reported — the reported figure is
    /// [`Self::write_bytes`], which counts only the bytes that passed their
    /// gate.
    pub write_bytes_spent: AtomicU64,
    /// Set when the runtime **write** hard stop fired (see `write_bytes`).
    pub write_stopped: AtomicBool,
    /// `--fail-fast` armed.
    pub fail_fast: AtomicBool,
    /// `--fail-fast` has tripped: stop starting files, stop retrying.
    pub aborted: AtomicBool,
    /// `--max-ram-bytes`, same `u64::MAX` sentinel as `download_limit`.
    pub ram_limit: AtomicU64,
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
    /// Bytes that were already correct where they belonged and were **not**
    /// written. The disk claim of the run, in one number: the bytes the
    /// repair did not have to touch.
    pub kept_bytes: AtomicU64,
}

impl Default for Summary {
    fn default() -> Self {
        Self::new()
    }
}

impl Summary {
    /// A run with no counters moved and no ceiling requested.
    pub fn new() -> Self {
        Self {
            files_total: 0,
            files_skipped: AtomicU64::new(0),
            files_repaired: AtomicU64::new(0),
            files_failed: AtomicU64::new(0),
            download: Meter::new(),
            deleted_extra_bytes: AtomicU64::new(0),
            flags: SummaryFlags::default(),
            download_stopped: AtomicBool::new(false),
            write_limit: AtomicU64::new(u64::MAX),
            write_bytes: AtomicU64::new(0),
            write_bytes_spent: AtomicU64::new(0),
            write_stopped: AtomicBool::new(false),
            fail_fast: AtomicBool::new(false),
            aborted: AtomicBool::new(false),
            ram_limit: AtomicU64::new(u64::MAX),
            ram_reserved: AtomicU64::new(0),
            ram_bytes: AtomicU64::new(0),
            files_inplace: AtomicU64::new(0),
            files_temp: AtomicU64::new(0),
            kept_bytes: AtomicU64::new(0),
        }
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
            // A snapshot must not lose the ceiling or the stop flag: the caller
            // reads both *after* the tasks have been joined, which is the only
            // moment either can be meaningful.
            download_stopped: AtomicBool::new(self.download_stopped.load(Ordering::Relaxed)),
            write_limit: AtomicU64::new(self.write_limit.load(Ordering::Relaxed)),
            write_bytes: AtomicU64::new(self.write_bytes.load(Ordering::Relaxed)),
            write_bytes_spent: AtomicU64::new(self.write_bytes_spent.load(Ordering::Relaxed)),
            write_stopped: AtomicBool::new(self.write_stopped.load(Ordering::Relaxed)),
            fail_fast: AtomicBool::new(self.fail_fast.load(Ordering::Relaxed)),
            aborted: AtomicBool::new(self.aborted.load(Ordering::Relaxed)),
            ram_limit: AtomicU64::new(self.ram_limit.load(Ordering::Relaxed)),
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
    /// Bytes written into `*_tmp`, when `--max-write-bytes` is active.
    pub write_bytes: Option<u64>,
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
    /// What a **runtime** hard stop cut short (`bytes` / `write` / `delete`), vs
    /// a pre-flight refusal which wrote nothing at all.
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
    /// Bytes that were already correct and were not rewritten.
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
enum Extra {
    Num(&'static str, u64),
    Word(&'static str, &'static str),
}

impl Extra {
    /// Only the rendering paths are used in production; `key` exists so a test
    /// can assert the fixed order of the conditional tail.
    #[cfg(test)]
    fn key(&self) -> &'static str {
        match self {
            Extra::Num(k, _) | Extra::Word(k, _) => k,
        }
    }

    fn put(self, rec: Record) -> Record {
        match self {
            Extra::Num(k, v) => rec.put(k, v),
            Extra::Word(k, v) => rec.put(k, v),
        }
    }
}

/// The conditional tail of a `SUMMARY`, in a fixed order so two runs' lines
/// diff cleanly. Empty (and therefore invisible) on an unflagged run.
fn conditional_extras(sum: &Summary) -> Vec<Extra> {
    let f = &sum.flags;
    let mut out = Vec::new();
    if f.limited {
        out.push(Extra::Num("limited", 1));
    }
    if f.scoped {
        out.push(Extra::Num("scoped", 1));
    }
    if f.scoped || f.config_bumped_zero {
        // A scoped (or `--purge-only`) run deliberately did not put the bytes
        // in place, so it does not advance config.ini. Emitted as a key
        // rather than left to be discovered by reading the file.
        out.push(Extra::Num("config_bumped", 0));
    }
    if f.limited || f.scoped {
        out.push(Extra::Num("plan_files", sum.files_total));
    }
    if let Some(n) = f.plan_download_bytes {
        out.push(Extra::Num("plan_download_bytes", n));
    }
    if let Some(n) = f.write_bytes {
        out.push(Extra::Num("write_bytes", n));
    }
    // The repair-mode group: what was asked for, what happened, and how
    // many bytes did not have to be written.
    if let Some(mode) = f.repair_mode {
        out.push(Extra::Word("repair_mode", mode));
        out.push(Extra::Num("inplace", f.files_inplace.unwrap_or(0)));
        out.push(Extra::Num("temp", f.files_temp.unwrap_or(0)));
        out.push(Extra::Num("kept_bytes", f.kept_bytes.unwrap_or(0)));
    }
    if let Some(n) = f.ram_bytes {
        out.push(Extra::Num("ram_bytes", n));
    }
    if let Some(n) = f.damaged_chunks {
        out.push(Extra::Num("damaged_chunks", n));
    }
    if let Some(s) = f.stopped {
        out.push(Extra::Word("stopped", s));
    }
    // Last: this is a fact about the run, not a verdict about it, and it is the
    // key a reader needs in order to trust the two byte counts above it.
    if let Some(s) = f.local_manifest {
        out.push(Extra::Word("local_manifest", s));
    }
    out
}

/// Emit one record to **both** stdout and the log in the run's chosen plane, so
/// unattended runs keep it regardless of which stream is captured.
///
/// The log carries the rendered line rather than the record, so a log file
/// written under `--output text` reads like the stdout a human saw, and one
/// written under `--output json` is parseable by the same code that reads
/// stdout. That is the point of logging it at all: the two streams carry the
/// same data plane, not merely the same facts.
/// NOTE(design-intent): the dual emission is deliberate (automation contract,
/// see README).
pub fn emit(record: &Record, format: OutputFormat) {
    let line = record.render(format);
    println!("{line}");
    tracing::info!("{line}");
}

/// The `PROGRESS` snapshot: where the run is, without touching the disk.
pub fn progress_record(sum: &Summary) -> Record {
    let skipped = sum.files_skipped.load(Ordering::Relaxed);
    let repaired = sum.files_repaired.load(Ordering::Relaxed);
    let failed = sum.files_failed.load(Ordering::Relaxed);
    let done = skipped + repaired + failed;
    let dl = sum.download.settled();
    Record::keyed("PROGRESS")
        .put("done", done)
        .put("total", sum.files_total)
        .put("skipped", skipped)
        .put("repaired", repaired)
        .put("failed", failed)
        .put("download_bytes", dl)
}

/// Emit the current progress snapshot.
pub fn emit_progress(sum: &Summary, format: OutputFormat) {
    emit(&progress_record(sum), format);
}

/// Background `PROGRESS` ticker for the repair phase. The first tick fires
/// immediately; it is skipped so we only report after a full interval.
///
/// Under `--output json` this is one more NDJSON line: the same record, same
/// shape, in the middle of the stream where the tick actually landed.
pub fn spawn_progress_reporter(
    sum: std::sync::Arc<Summary>,
    format: OutputFormat,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(PROGRESS_INTERVAL_SECS));
        interval.tick().await;
        loop {
            interval.tick().await;
            emit_progress(&sum, format);
        }
    })
}

/// The one-time begin report, written right after the metadata calls. Parsers
/// get it before any repair starts: `local_version` is the on-disk `config.ini`
/// value (`none` when absent), `latest_version` is the "will be updated to" tag;
/// the rest come from the HoYoPlay/Sophon APIs, not from config. `audio_langs`
/// is the effective set.
///
/// Provenance: Starward `GameInstallService.cs:788-849` (the same field set the
/// launcher shows the user before installing).
#[allow(clippy::too_many_arguments)] // stable report formatter; params-struct would churn CLI parsing
pub fn report_record(
    local_version: &Option<String>,
    latest_version: &str,
    biz: &str,
    cfg: &GameConfig,
    pkg: &GameBranchPackage,
    build: &ChunkBuild,
    audio_langs: &HashSet<String>,
) -> Record {
    let mut langs: Vec<&String> = audio_langs.iter().collect();
    langs.sort();
    let audio_joined: Vec<&str> = langs.iter().map(|s| s.as_str()).collect();
    let local = local_version.as_deref().unwrap_or("none");
    Record::keyed("REPORT")
        .put("local_version", local)
        .put("latest_version", latest_version)
        .put("biz", biz)
        .put("exe", cfg.exe_file_name.as_str())
        .put("download_mode", cfg.default_download_mode.as_str())
        .put("branch", pkg.branch.as_str())
        .put("package_id", pkg.package_id.as_str())
        .put("build_id", build.build_id.as_str())
        .put("audio_langs", audio_joined.join(","))
        .put("diff_tags", pkg.diff_tags.join(","))
}

/// The final `SUMMARY`, ending every run.
///
/// Trailing conditional keys (`limited`, `scoped`, `config_bumped`,
/// `plan_files`, …) are appended only when their flag is
/// active, so an unflagged run's line is byte-identical to a run that passes no
/// flags at all.
pub fn summary_record(sum: &Summary, exit: i32) -> Record {
    let skipped = sum.files_skipped.load(Ordering::Relaxed);
    let repaired = sum.files_repaired.load(Ordering::Relaxed);
    let failed = sum.files_failed.load(Ordering::Relaxed);
    let dl = sum.download.settled();
    let del = sum.deleted_extra_bytes.load(Ordering::Relaxed);
    let base = Record::keyed("SUMMARY")
        .put("total", sum.files_total)
        .put("skipped", skipped)
        .put("repaired", repaired)
        .put("failed", failed)
        .put("download_bytes", dl)
        .put("deleted_extra_bytes", del)
        .put("exit", exit);
    conditional_extras(sum)
        .into_iter()
        .fold(base, |rec, extra| extra.put(rec))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two planes are two renderings of one field list. This is the
    /// property the whole module exists to guarantee, so it is pinned on a
    /// record with a string field, a number, and a quoted value.
    #[test]
    fn text_and_json_are_two_renderings_of_one_record() {
        let rec = Record::keyed("SUMMARY")
            .put("total", 4u64)
            .put("word", "a b\"c");
        assert_eq!(rec.text(), "SUMMARY total=4 word=a b\"c");
        assert_eq!(
            rec.json(),
            r#"{"event":"summary","total":4,"word":"a b\"c"}"#,
            "the event name is the label lowercased, and the order is report order"
        );
        assert_eq!(rec.event(), "summary");
        assert_eq!(rec.label(), "SUMMARY");
        assert_eq!(
            rec.fields().iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            vec!["total", "word"],
            "field order is report order, which is what both renderings keep"
        );
        // One record, one line, whichever plane is active.
        assert_eq!(rec.render(OutputFormat::Text), rec.text());
        assert_eq!(rec.render(OutputFormat::Json), rec.json());
        assert_eq!(OutputFormat::default(), OutputFormat::Text);
        assert_eq!(OutputFormat::Json.as_str(), "json");
    }

    #[test]
    fn progress_record_counts_done_as_the_three_outcomes() {
        let s = Summary {
            files_total: 10,
            files_skipped: AtomicU64::new(3),
            files_repaired: AtomicU64::new(2),
            files_failed: AtomicU64::new(1),
            ..Summary::default()
        };
        s.download.settle(123);
        let rec = progress_record(&s);
        assert_eq!(
            rec.text(),
            "PROGRESS done=6 total=10 skipped=3 repaired=2 failed=1 download_bytes=123"
        );
        // `done` and `total` are two numbers on both planes. They are separate
        // fields rather than one pre-joined `6/10` string, because a pre-joined
        // string cannot be a JSON number.
        let v: serde_json::Value = serde_json::from_str(&rec.json()).unwrap();
        assert_eq!(v["event"], "progress");
        assert_eq!(v["done"], 6);
        assert_eq!(v["total"], 10);
    }

    #[test]
    fn summary_line_keyvalue_and_json_agree() {
        let s = Summary {
            files_total: 4,
            files_skipped: AtomicU64::new(1),
            files_repaired: AtomicU64::new(2),
            files_failed: AtomicU64::new(1),
            deleted_extra_bytes: AtomicU64::new(20),
            flags: SummaryFlags::default(),
            ..Summary::default()
        };
        s.download.settle(10);
        let rec = summary_record(&s, 3);
        assert_eq!(
            rec.text(),
            "SUMMARY total=4 skipped=1 repaired=2 failed=1 download_bytes=10 deleted_extra_bytes=20 exit=3"
        );
        let json = rec.json();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["event"], "summary");
        assert_eq!(v["total"], 4);
        assert_eq!(v["skipped"], 1);
        assert_eq!(v["repaired"], 2);
        assert_eq!(v["failed"], 1);
        assert_eq!(v["download_bytes"], 10);
        assert_eq!(v["deleted_extra_bytes"], 20);
        assert_eq!(v["exit"], 3);
        // An unflagged run gains no members at all — that is the whole
        // contract of the conditional keys.
        assert_eq!(v.as_object().unwrap().len(), 8, "{json}");
    }

    #[test]
    fn conditional_keys_appear_only_when_their_flag_is_active() {
        let mut s = Summary {
            files_total: 8,
            ..Summary::default()
        };
        s.flags.scoped = true;
        let rec = summary_record(&s, 0);
        let line = rec.text();
        for k in ["scoped=1", "config_bumped=0", "plan_files=8"] {
            assert!(line.contains(k), "{k} missing from {line}");
        }
        assert!(!line.contains("limited"), "{line}");
        assert!(!line.contains("plan_download_bytes"), "{line}");
        assert!(!line.contains("write_bytes"), "{line}");
        assert!(!line.contains("stopped"), "{line}");
        // Fixed fields are untouched; only a tail is appended.
        assert!(line.starts_with("SUMMARY total=8 skipped=0 repaired=0 failed=0 download_bytes=0 deleted_extra_bytes=0 exit=0 "));

        // The same run as JSON: same keys, right types.
        let v: serde_json::Value = serde_json::from_str(&rec.json()).unwrap();
        assert_eq!(v["scoped"], 1);
        assert_eq!(v["config_bumped"], 0);
        assert_eq!(v["plan_files"], 8);
        assert_eq!(v.as_object().unwrap().len(), 11);
    }

    #[test]
    fn every_conditional_key_has_a_known_name_and_typed_rendering() {
        let mut s = Summary {
            files_total: 3,
            ..Summary::default()
        };
        s.flags = SummaryFlags {
            limited: true,
            scoped: true,
            plan_download_bytes: Some(4242),
            write_bytes: Some(77),
            stopped: Some("bytes"),
            config_bumped_zero: false,
            local_manifest: Some("unavailable"),
            repair_mode: Some("inplace"),
            files_inplace: Some(3),
            files_temp: Some(1),
            kept_bytes: Some(900),
            ram_bytes: Some(4096),
            damaged_chunks: Some(7),
        };
        let rec = summary_record(&s, 5);
        assert_eq!(
            rec.text(),
            "SUMMARY total=3 skipped=0 repaired=0 failed=0 download_bytes=0 deleted_extra_bytes=0 \
             exit=5 limited=1 scoped=1 config_bumped=0 plan_files=3 plan_download_bytes=4242 \
             write_bytes=77 repair_mode=inplace inplace=3 temp=1 kept_bytes=900 ram_bytes=4096 \
             damaged_chunks=7 stopped=bytes local_manifest=unavailable"
        );
        // `stopped` and `local_manifest` are words, not numbers: the JSON plane
        // must say so.
        let v: serde_json::Value = serde_json::from_str(&rec.json()).unwrap();
        assert_eq!(v["stopped"], "bytes");
        assert_eq!(v["write_bytes"], 77);
        assert_eq!(v["plan_download_bytes"], 4242);
        assert_eq!(v["local_manifest"], "unavailable");
        assert_eq!(v["repair_mode"], "inplace");
        assert_eq!(v["inplace"], 3);
        assert_eq!(v["temp"], 1);
        assert_eq!(v["kept_bytes"], 900);
        assert_eq!(v["ram_bytes"], 4096);
        assert_eq!(v["damaged_chunks"], 7);
        // Key order is fixed, so two runs' lines diff cleanly.
        let keys: Vec<&str> = conditional_extras(&s).iter().map(Extra::key).collect();
        assert_eq!(
            keys,
            vec![
                "limited",
                "scoped",
                "config_bumped",
                "plan_files",
                "plan_download_bytes",
                "write_bytes",
                "repair_mode",
                "inplace",
                "temp",
                "kept_bytes",
                "ram_bytes",
                "damaged_chunks",
                "stopped",
                "local_manifest"
            ]
        );
    }

    /// `local_manifest` is about the run, not a verdict on it, so it must not
    /// appear when no local version was ever known — and must not drag
    /// `limited`/`scoped`/`plan_files` along with it.
    #[test]
    fn local_manifest_stands_alone() {
        for value in ["ok", "unavailable"] {
            let mut s = Summary {
                files_total: 2,
                ..Summary::default()
            };
            s.flags.local_manifest = Some(value);
            let rec = summary_record(&s, 0);
            let line = rec.text();
            assert!(line.contains(&format!("local_manifest={value}")), "{line}");
            assert!(!line.contains("limited"), "{line}");
            assert!(!line.contains("scoped"), "{line}");
            assert!(!line.contains("plan_files"), "{line}");
            assert!(!line.contains("config_bumped"), "{line}");
            let v: serde_json::Value = serde_json::from_str(&rec.json()).unwrap();
            assert_eq!(v["local_manifest"], value);
        }
        // Absent when there was no local version to look up.
        let s = Summary {
            files_total: 2,
            ..Summary::default()
        };
        let rec = summary_record(&s, 0);
        assert!(!rec.text().contains("local_manifest"));
        let v: serde_json::Value = serde_json::from_str(&rec.json()).unwrap();
        assert!(v.get("local_manifest").is_none(), "{rec:?}");
    }

    #[test]
    fn snapshot_copies_every_counter() {
        let s = Summary {
            files_total: 7,
            files_skipped: AtomicU64::new(1),
            ..Summary::default()
        };
        s.download.settle(9);
        let snap = s.snapshot();
        assert_eq!(snap.files_total, 7);
        assert_eq!(snap.files_skipped.load(Ordering::Relaxed), 1);
        assert_eq!(snap.download.settled(), 9);
        assert_eq!(snap.flags, SummaryFlags::default());
        // Mutating the source afterwards must not touch the snapshot.
        s.files_skipped.store(5, Ordering::Relaxed);
        assert_eq!(snap.files_skipped.load(Ordering::Relaxed), 1);
    }

    fn cfg_full() -> GameConfig {
        GameConfig {
            exe_file_name: "YuanShen.exe".into(),
            audio_pkg_scan_dir: "scan.txt".into(),
            audio_pkg_res_dir: "res".into(),
            audio_pkg_cache_dir: "cache".into(),
            default_download_mode: "DOWNLOAD_MODE_CHUNK".into(),
            res_category_dir: "rc.txt".into(),
            blacklist_dir: "bl.txt".into(),
            enable_resource_blacklist: true,
            game: None,
        }
    }

    #[test]
    fn report_record_reports_versions_and_api_fields() {
        let pkg = GameBranchPackage {
            package_id: "pkg1".into(),
            branch: "main".into(),
            password: String::new(),
            tag: "5.1.0".into(),
            diff_tags: vec!["5.0.0".into(), "5.1.0".into()],
        };
        let build = ChunkBuild {
            build_id: "build42".into(),
            tag: "5.1.0".into(),
            manifests: Vec::new(),
        };
        let audio: HashSet<String> = ["ja-jp".into(), "en-us".into()].into_iter().collect();
        let rec = report_record(
            &Some("4.0.0".to_string()),
            "5.1.0",
            "hk4e_global",
            &cfg_full(),
            &pkg,
            &build,
            &audio,
        );
        let line = rec.text();
        assert!(
            line.starts_with("REPORT local_version=4.0.0 latest_version=5.1.0 biz=hk4e_global"),
            "{line}"
        );
        assert!(line.contains("exe=YuanShen.exe"), "{line}");
        assert!(line.contains("download_mode=DOWNLOAD_MODE_CHUNK"), "{line}");
        assert!(line.contains("branch=main"), "{line}");
        assert!(line.contains("package_id=pkg1"), "{line}");
        assert!(line.contains("build_id=build42"), "{line}");
        assert!(line.contains("audio_langs=en-us,ja-jp"), "{line}");
        assert!(line.contains("diff_tags=5.0.0,5.1.0"), "{line}");
    }

    #[test]
    fn report_record_says_none_for_an_absent_local_version() {
        let cfg = GameConfig {
            exe_file_name: "GenshinImpact.exe".into(),
            ..cfg_full()
        };
        let pkg = GameBranchPackage::default();
        let build = ChunkBuild {
            build_id: "b1".into(),
            tag: "3.0.0".into(),
            manifests: Vec::new(),
        };
        let audio: HashSet<String> = HashSet::new();
        let rec = report_record(&None, "3.0.0", "hk4e_cn", &cfg, &pkg, &build, &audio);
        let v: serde_json::Value = serde_json::from_str(&rec.json()).unwrap();
        assert_eq!(v["event"], "report");
        assert_eq!(v["local_version"], "none");
        assert_eq!(v["latest_version"], "3.0.0");
        assert_eq!(v["biz"], "hk4e_cn");
        assert_eq!(v["exe"], "GenshinImpact.exe");
        assert_eq!(v["audio_langs"], "");
    }
}
