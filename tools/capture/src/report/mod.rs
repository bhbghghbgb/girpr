//! The stdout data plane: one or more [`Record`]s per run, one line each.
//!
//! A record is a label plus ordered fields, **held once and rendered twice** —
//! `--output text` prints `SUMMARY event=capture tool=gircap …` and
//! `--output json` prints `{"event":"capture","tool":"gircap",…}` from the same
//! field list. **The two are renderings of one value, not two implementations**,
//! so they cannot drift.
//!
//! Every run ends with exactly one `SUMMARY`: the terminal record, carrying the
//! result and the exit code. A run may emit records *before* it — `list` writes
//! one `BUNDLE` per bundle it found, then the `SUMMARY` — so a consumer can read
//! a stream until the summary rather than expecting one line and parsing it.
//!
//! `--output json` is newline-delimited: one object per line, no enclosing
//! array, so it can be streamed and still recovered in order. Within one object
//! the keys are *names*; read them by name, never by position.
//!
//! This module owns the only `println!` in the crate. Diagnostics go to stderr
//! through [`crate::logging`]; nothing here writes to stderr and nothing there
//! writes here, which is what lets `--output json` be piped straight into a
//! parser.

use std::io::Write as _;
use std::sync::Mutex;

use crate::VERSION;
use crate::encoding;

// `percent_decode` is part of this module's public API; its definition now
// lives in `crate::encoding` and is re-exported here so the path is stable.
pub use crate::encoding::percent_decode;
/// How a record is rendered on stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum OutputFormat {
    /// `SUMMARY k=v k=v`, human-readable.
    #[default]
    Text,
    /// `{"event":"summary", …}`, one object per line, machine-readable.
    Json,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "text" => Ok(OutputFormat::Text),
            "json" => Ok(OutputFormat::Json),
            other => Err(format!(
                "unknown output format {other:?}; expected text or json"
            )),
        }
    }
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            OutputFormat::Text => "text",
            OutputFormat::Json => "json",
        })
    }
}

/// One value on a summary line.
///
/// The three cases exist so that the JSON rendering carries native numbers and
/// booleans rather than quoted strings. A consumer summing `bytes_downloaded`
/// should not have to parse it.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    /// A string. `null` and `""` are different facts and stay different.
    Str(String),
    /// A signed integer: a byte count, a file count, an exit code.
    Int(i64),
    /// A flag.
    Bool(bool),
}

impl Val {
    /// The `k=v` rendering, used by the text form.
    ///
    /// A value that would break tokenisation — whitespace, a control character,
    /// or a literal `%` that would be ambiguous once encoded — is percent-encoded.
    /// Quoting does not work here: `k="two words"` still splits into three
    /// whitespace-separated tokens, so a quoting rule would move the ambiguity
    /// rather than remove it.
    ///
    /// Encoding is lossless and reversible, affects no ordinary value (a path on
    /// Windows has no whitespace unless its directories do, and `%` is rare in
    /// one), and keeps the whole line splittable on whitespace — which is the
    /// only reason to prefer text over JSON in the first place.
    fn text(&self) -> String {
        match self {
            Val::Str(s) => encoding::percent_encode(s),
            Val::Int(i) => i.to_string(),
            Val::Bool(b) => b.to_string(),
        }
    }
}

impl From<&str> for Val {
    fn from(v: &str) -> Self {
        Val::Str(v.to_string())
    }
}
impl From<String> for Val {
    fn from(v: String) -> Self {
        Val::Str(v)
    }
}
impl From<&String> for Val {
    fn from(v: &String) -> Self {
        Val::Str(v.clone())
    }
}
impl From<&std::path::Path> for Val {
    fn from(v: &std::path::Path) -> Self {
        Val::Str(v.display().to_string())
    }
}
impl From<&std::ffi::OsStr> for Val {
    fn from(v: &std::ffi::OsStr) -> Self {
        Val::Str(v.to_string_lossy().into_owned())
    }
}
impl From<&[u8]> for Val {
    /// A non-UTF-8 path is not silently mangled: it is base16'd, which is the
    /// only spelling of an arbitrary byte string that round-trips.
    fn from(v: &[u8]) -> Self {
        if let Ok(s) = std::str::from_utf8(v) {
            return Val::Str(s.to_string());
        }
        Val::Str(crate::hash::hex_bytes(v))
    }
}
impl From<i64> for Val {
    fn from(v: i64) -> Self {
        Val::Int(v)
    }
}
impl From<u64> for Val {
    fn from(v: u64) -> Self {
        // Counts and byte totals are `u64` while a field value is `i64`. No real
        // run gets near `i64::MAX`, so clamping is the honest answer for one
        // that does: the record stays readable instead of wrapping negative.
        Val::Int(i64::try_from(v).unwrap_or(i64::MAX))
    }
}
impl From<usize> for Val {
    fn from(v: usize) -> Self {
        // Same rule as `From<u64>`: a count that does not fit `i64` clamps.
        Val::Int(i64::try_from(v).unwrap_or(i64::MAX))
    }
}
impl From<u32> for Val {
    fn from(v: u32) -> Self {
        Val::Int(i64::from(v))
    }
}
impl From<i32> for Val {
    fn from(v: i32) -> Self {
        Val::Int(i64::from(v))
    }
}
impl From<bool> for Val {
    fn from(v: bool) -> Self {
        Val::Bool(v)
    }
}

/// One record on the stdout data plane: a label plus ordered fields.
///
/// Field order is report order, so a line reads in the order a reader wants it
/// (what was asked for → what came back → what it cost) rather than in hash
/// order, and two runs of the same command diff cleanly.
#[derive(Debug, Clone)]
pub struct Record {
    /// The text label: `SUMMARY` for the terminal record, `BUNDLE` for one
    /// bundle found by `list`.
    label: &'static str,
    /// Which command produced it: `capture`, `verify`, `materialize`, `list`.
    pub event: &'static str,
    fields: Vec<(&'static str, Val)>,
}

impl Record {
    /// The terminal record every run ends with: label `SUMMARY`, for `event`.
    ///
    /// `tool` and `version` are pushed first and unconditionally: a summary that
    /// does not say which tool wrote it is not an answer to anything.
    pub fn summary(event: &'static str) -> Self {
        Self {
            label: "SUMMARY",
            event,
            fields: vec![
                ("event", Val::Str(event.to_string())),
                ("tool", Val::Str("gircap".to_string())),
                ("version", Val::Str(VERSION.to_string())),
            ],
        }
    }

    /// A non-terminal record: label `label`, carrying the same `tool`/`version`
    /// head so a line read in isolation still says what produced it.
    ///
    /// Not carrying `event` is deliberate: `list` emits `BUNDLE` records for one
    /// command, so an `event` per record would say `list` several times and say
    /// nothing the record's own label does not.
    pub fn labeled(label: &'static str) -> Self {
        Self {
            label,
            event: "",
            fields: vec![
                ("tool", Val::Str("gircap".to_string())),
                ("version", Val::Str(VERSION.to_string())),
            ],
        }
    }

    /// The text label, e.g. `SUMMARY` or `BUNDLE`.
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// Append one field. Duplicate names are kept, not merged: two numbers with
    /// the same name are two facts, and silently collapsing them would make the
    /// line lie.
    pub fn set(mut self, key: &'static str, value: impl Into<Val>) -> Self {
        self.fields.push((key, value.into()));
        self
    }

    /// Append many fields at once, **in the order given**.
    ///
    /// The same as a chain of [`Record::set`], and that is the point: it is how
    /// `capture` builds its 56-key summary. The order is the order a reader sees
    /// on the line, so taking an ordered iterable rather than a map is
    /// deliberate - a `BTreeMap` would sort the keys and silently rewrite the
    /// output.
    ///
    /// The alternative was leaving the chain in the orchestrator, where 56 calls
    /// buried the actual control flow. The key list is still written out once,
    /// here, where [`Val`] and the rest of the record's vocabulary live.
    pub fn set_all<I>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = (&'static str, Val)>,
    {
        self.fields.extend(fields);
        self
    }

    /// Look up a field by name, for tests and for callers that want to assert
    /// on the value rather than the rendering.
    pub fn get(&self, key: &str) -> Option<&Val> {
        self.fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    /// Every field name, **in the order they were appended**.
    ///
    /// [`Record::get`] answers "what is this field's value"; this answers "what
    /// fields are on the line, and in what order", which is a different question
    /// and one a caller parsing the output has to be able to ask. Duplicate
    /// names appear as many times as they were set - see [`Record::set`].
    pub fn keys(&self) -> impl Iterator<Item = &'static str> {
        self.fields.iter().map(|(k, _)| *k)
    }

    /// The rendered text line: `SUMMARY event=capture tool=gircap …`.
    pub fn to_text_line(&self) -> String {
        let mut s = String::from(self.label);
        for (k, v) in &self.fields {
            s.push(' ');
            s.push_str(k);
            s.push('=');
            s.push_str(&v.text());
        }
        s
    }

    /// The rendered JSON object, one line, keys in the same order.
    ///
    /// Written pair by pair rather than through a `serde_json::Map` because that
    /// map is a `BTreeMap` unless the `preserve_order` feature is on, and turning
    /// it on is not this crate's call: cargo unifies features across the whole
    /// workspace build, so enabling it here would silently reorder the JSON
    /// another tool in this repository emits. The fields are few and all
    /// primitives, so the encoding cost is a `to_string` per key and per value.
    ///
    /// **`event` always comes first**, so a consumer can dispatch on it while
    /// still streaming rather than after the line is parsed. Its value is the
    /// record's own `event` field when it has one — which for the terminal record
    /// is the command, `capture`/`verify`/`list`/`materialize` — and otherwise the
    /// label lowercased, so a non-terminal record (`BUNDLE` → `bundle`) is still
    /// self-identifying. The field is not written twice.
    pub fn to_json_line(&self) -> String {
        // Borrowed, not cloned. A `Val::Str` is a `String`, and there is no
        // reason to copy one to render a field of a record already held by
        // reference.
        let fallback;
        let event = match self.fields.iter().find(|(k, _)| *k == "event") {
            Some((_, v)) => v,
            None => {
                fallback = Val::Str(self.json_event());
                &fallback
            }
        };
        let mut s = String::with_capacity(64 + 24 * self.fields.len());
        s.push('{');
        push_json_key(&mut s, "event");
        s.push(':');
        push_json_value(&mut s, event);
        for (k, v) in self.fields.iter().filter(|(k, _)| *k != "event") {
            s.push(',');
            push_json_key(&mut s, k);
            s.push(':');
            push_json_value(&mut s, v);
        }
        s.push('}');
        s
    }

    /// The label lowercased: the `event` a record without one of its own carries.
    fn json_event(&self) -> String {
        self.label.to_ascii_lowercase()
    }

    /// Render and write the line to stdout, and log the same record.
    pub fn emit(&self, format: OutputFormat) {
        emit(&match format {
            OutputFormat::Text => self.to_text_line(),
            OutputFormat::Json => self.to_json_line(),
        });
    }
}

/// Push `"key"` onto `out`.
///
/// **Written straight into the buffer**, because a key is a `&'static str` of
/// this crate's own choosing — `event`, `exit_code`, `result` — and every one
/// of them is plain ASCII. `json_string` allocated a `String` per key, twice
/// over for `event`, purely to be appended and dropped. The escape path is
/// still there for the one case that needs it: a key that is *not* from this
/// crate.
fn push_json_key(out: &mut String, k: &str) {
    out.push('"');
    if is_plain_ascii_json(k) {
        out.push_str(k);
    } else {
        out.push_str(&json_string(k));
    }
    out.push('"');
}

/// Whether `s` can go into a JSON string literal unescaped.
///
/// `serde_json` would escape `"` and `\`; both are excluded here so the two
/// paths agree.
fn is_plain_ascii_json(s: &str) -> bool {
    s.bytes()
        .all(|b| b >= 0x20 && b != b'"' && b != b'\\' && b.is_ascii())
}

/// A JSON string literal, with the escapes a raw quote would need.
///
/// Only reached by a key or a [`Val::Str`] carrying a `"` or a control
/// character — a Windows artifact path has backslashes, which JSON allows
/// unescaped, so the common case is a memcpy.
fn json_string(s: &str) -> String {
    if s.bytes().all(|b| b >= 0x20 && b != b'"' && b != b'\\') {
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
fn push_json_value(out: &mut String, v: &Val) {
    use std::fmt::Write as _;
    match v {
        // The common case, written in place. A Windows artifact path is full of
        // backslashes but JSON allows those unescaped, so most values take
        // this branch and allocate nothing.
        Val::Str(s) if is_plain_ascii_json(s) => {
            out.push('"');
            out.push_str(s);
            out.push('"');
        }
        Val::Str(s) => out.push_str(&json_string(s)),
        Val::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Val::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
    }
}

/// Writes one stdout line and logs the matching event, as a single unit.
///
/// A mutex rather than a bare `println!` because a record can be emitted from a
/// worker as well as from the dispatcher; the lock is held for exactly one line
/// and never across work.
static STDOUT_LOCK: Mutex<()> = Mutex::new(());

/// Everything `gircap capture` puts on its data plane.
///
/// **The orchestrator's last job, moved here.** This line has 56 chained
/// [`Record::set`] calls in it, and the run's entire reporting contract — every
/// key name, every order — belongs beside the other summaries, not in the middle
/// of the function that decides what a capture *does*, where a reader looking
/// for either had to find the other first.
///
/// Grouped by what the numbers are about, not by where they came from. The
/// diff, reuse and manifest groups come from three different walks over three
/// different collections; putting them together is the point, because a reader
/// asking "what did this capture measure" should not have to know that.
pub struct CaptureSummary<'a> {
    /// `result`, always `"ok"` - a failed run does not reach this.
    pub result: &'a str,
    /// `biz` as the flag spelled it.
    pub biz: &'a str,
    /// The resolved old tag.
    pub old_tag: &'a str,
    /// The resolved new tag.
    pub new_tag: &'a str,
    /// The `package_id` the version pair resolved to.
    pub package_id: &'a str,
    /// The old side's build id.
    pub old_build_id: &'a str,
    /// The new side's build id.
    pub new_build_id: &'a str,
    /// The artifact root.
    pub artifact: &'a std::path::Path,
    /// How many `api/` documents were written.
    pub api_documents: u64,
    /// The `matching_field`s that were scored, joined.
    pub selection_pool: &'a str,
    /// Everything about what was admitted and what was refused.
    pub selection: SelectionStats,
    /// Everything about chunk blobs.
    pub chunks: ChunkStats,
    /// Everything about patchmap assembly.
    pub assembly: AssemblyStats,
    /// Everything about the manifests that were written.
    pub manifests: ManifestStats,
    /// The headline diff result.
    pub diff: DiffStats,
    /// The cross-file reuse measurement.
    pub reuse: ReuseStats,
    /// The transport's own totals.
    pub http: HttpStats,
    /// The tag probe's totals.
    pub probe: ProbeStats,
    /// How many major versions behind the launcher is, or `0` if it answered.
    pub legacy_endpoint_majors_behind: u64,
    /// The process exit code.
    pub exit_code: i32,
}

/// The selection's own numbers.
pub struct SelectionStats {
    /// Candidates the manifests offered.
    pub candidates_scored: u64,
    /// Captures the budget funded.
    pub captures_admitted: u64,
    /// Captures the budget refused.
    pub captures_over_budget: u64,
    /// `--files` paths the run was asked for.
    pub forced_files_requested: u64,
    /// Of those, the manifests do not carry.
    pub forced_files_skipped: u64,
    /// Of those, present in the manifests but answering no matrix class.
    pub forced_files_unscored: u64,
    /// What admission charged.
    pub capture_bytes_admitted: u64,
    /// Whether `--budget` capped the run at all.
    pub budget_unlimited: bool,
    /// The cap, or `0` when there was none.
    pub budget_limit: u64,
    /// Required classes the selection met.
    pub classes_satisfied: u64,
    /// Required classes it did not.
    pub classes_unmet: u64,
}

/// The chunk store's numbers.
pub struct ChunkStats {
    /// Chunks the tree asked for.
    pub wanted: u64,
    /// Of those, stored.
    pub stored: u64,
    /// The projected cost of the whole store.
    pub bytes_projected: u64,
    /// What the CDN actually sent for it.
    pub bytes_fetched: u64,
    /// Fetches that failed.
    pub fetches_failed: u64,
    /// Blobs failing A6.
    pub a6_failing: u64,
    /// Chunks the `patch_fetch` set names.
    pub patch_fetch_chunks: u64,
}

/// The assembly loop's numbers.
pub struct AssemblyStats {
    /// Patchmaps written.
    pub patchmaps_written: u64,
    /// Selfcheck rows the assembly produced.
    pub selfcheck_rows: u64,
    /// Of those, the failures.
    pub selfcheck_failures: u64,
    /// Chunks reused from the old side.
    pub patch_reuse_chunks: u64,
    /// Chunks actually fetched to build a patchmap.
    pub patch_fetch_chunks: u64,
    /// Chunks moved by those patchmaps.
    pub patch_moved_chunks: u64,
}

/// The written manifests' numbers, from one walk over all of them.
pub struct ManifestStats {
    /// How many manifests.
    pub captured: u64,
    /// Their stored bytes, summed.
    pub bytes: u64,
    /// Their entry counts, summed.
    pub entries: u64,
    /// Their chunk counts, summed.
    pub chunks: u64,
    /// The largest compressed chunk in any of them.
    pub max_chunk_compressed_bytes: u64,
}

/// The diff's headline result.
pub struct DiffStats {
    /// Paths in both.
    pub common: u64,
    /// Of those, unchanged.
    pub identical: u64,
    /// Of those, changed.
    pub changed: u64,
    /// Only in the new.
    pub added: u64,
    /// Only in the old.
    pub removed: u64,
    /// Patch bytes across the pair.
    pub patch_bytes: u64,
}

/// Cross-file reuse, from the diff's pool measurement.
pub struct ReuseStats {
    /// New chunks with an MD5 present somewhere in the old manifest.
    pub md5_present_in_old: u64,
    /// New chunks absent from their own old file.
    pub cross_file_only_chunks: u64,
    /// The bytes those would have saved.
    pub cross_file_bytes_forgone: u64,
}

/// What the transport did.
pub struct HttpStats {
    /// Documents written.
    pub documents_written: u64,
    /// Requests issued.
    pub requests: u64,
    /// Bytes received.
    pub bytes_downloaded: u64,
}

/// What the tag probe found.
pub struct ProbeStats {
    /// Tags the probe asked about.
    pub probed: u64,
    /// Tags the API served.
    pub served: u64,
    /// Of those, served despite not being in `diff_tags`.
    pub served_outside_diff_tags: u64,
}

/// The `stage` a `capture` summary carries.
///
/// **A named constant, because it is part of the data plane.** Spelling it as
/// a milestone — `m8-artifact` — would tell a reader about the order the crate
/// was built in and nothing about the run in front of them. It names the stage
/// now: every record this builder emits is written after the artifact, so the
/// value is the same on every capture and its job is to identify *which*
/// record this is.
pub const STAGE_CAPTURE: &str = "artifact";

/// Build the `capture` summary record.
///
/// **The key list is the contract.** `girpr` and anything else reading this
/// line parse these names in this order; a rename or a reorder is a breaking
/// change. That is why this is one function with a flat, commented key list
/// rather than something derived - a derived key set would be tidier right up
/// until the day it silently renamed a field a caller depended on.
///
/// The order below is the order the numbers were appended in, not a grouping: it
/// is preserved so that a diff of two summaries over the same run shows only
/// what actually changed.
///
/// `stage` says how far the run got, and it is the field that tells this record
/// apart from the *failure* record a failing capture also emits: the run's own
/// record carries the numbers it had already measured plus `stage=`, and the
/// failure's carries `result: "error"`, `exit_code` and `artifact_trustworthy`.
/// A harness reading the **last** record gets the verdict either way; one
/// matching on `stage=` wants the run's own numbers.
pub fn capture_summary(s: &CaptureSummary<'_>) -> Record {
    Record::summary("capture").set_all([
        ("result", Val::from(s.result)),
        // Every capture that reaches this line has written its artifact, so the
        // value names the stage rather than counting one: `artifact`, in the same
        // vocabulary as `MANIFEST.json`'s `capture.complete`.
        ("stage", Val::from(STAGE_CAPTURE)),
        ("biz", Val::from(s.biz)),
        ("old_tag", Val::from(s.old_tag)),
        ("new_tag", Val::from(s.new_tag)),
        ("package_id", Val::from(s.package_id)),
        ("old_build_id", Val::from(s.old_build_id)),
        ("new_build_id", Val::from(s.new_build_id)),
        ("artifact", Val::from(s.artifact)),
        ("api_documents", Val::from(s.api_documents)),
        ("selection_pool", Val::from(s.selection_pool)),
        // Selection.
        (
            "candidates_scored",
            Val::from(s.selection.candidates_scored),
        ),
        (
            "captures_admitted",
            Val::from(s.selection.captures_admitted),
        ),
        (
            "captures_over_budget",
            Val::from(s.selection.captures_over_budget),
        ),
        (
            "forced_files_requested",
            Val::from(s.selection.forced_files_requested),
        ),
        (
            "forced_files_skipped",
            Val::from(s.selection.forced_files_skipped),
        ),
        (
            "forced_files_unscored",
            Val::from(s.selection.forced_files_unscored),
        ),
        // Chunks.
        ("chunk_blobs_wanted", Val::from(s.chunks.wanted)),
        ("chunk_blobs_stored", Val::from(s.chunks.stored)),
        ("chunk_bytes_projected", Val::from(s.chunks.bytes_projected)),
        ("chunk_bytes_fetched", Val::from(s.chunks.bytes_fetched)),
        ("chunk_fetches_failed", Val::from(s.chunks.fetches_failed)),
        ("chunk_a6_failing", Val::from(s.chunks.a6_failing)),
        ("patch_fetch_chunks", Val::from(s.chunks.patch_fetch_chunks)),
        // Assembly.
        ("patchmaps_written", Val::from(s.assembly.patchmaps_written)),
        (
            "assembly_selfcheck_rows",
            Val::from(s.assembly.selfcheck_rows),
        ),
        (
            "assembly_selfcheck_failures",
            Val::from(s.assembly.selfcheck_failures),
        ),
        (
            "patch_reuse_chunks",
            Val::from(s.assembly.patch_reuse_chunks),
        ),
        (
            "patch_fetch_chunks_actually",
            Val::from(s.assembly.patch_fetch_chunks),
        ),
        (
            "patch_moved_chunks",
            Val::from(s.assembly.patch_moved_chunks),
        ),
        // Selection, continued. These were interleaved with the chunk block in
        // the original chain, and the order is preserved.
        (
            "capture_bytes_admitted",
            Val::from(s.selection.capture_bytes_admitted),
        ),
        ("budget_unlimited", Val::from(s.selection.budget_unlimited)),
        ("budget_limit", Val::from(s.selection.budget_limit)),
        (
            "classes_satisfied",
            Val::from(s.selection.classes_satisfied),
        ),
        ("classes_unmet", Val::from(s.selection.classes_unmet)),
        // Manifests.
        ("manifests_captured", Val::from(s.manifests.captured)),
        ("manifest_bytes", Val::from(s.manifests.bytes)),
        ("manifest_entries", Val::from(s.manifests.entries)),
        ("manifest_chunks", Val::from(s.manifests.chunks)),
        (
            "max_chunk_compressed_bytes",
            Val::from(s.manifests.max_chunk_compressed_bytes),
        ),
        // The diff: the run's headline result.
        ("diff_common", Val::from(s.diff.common)),
        ("diff_identical", Val::from(s.diff.identical)),
        ("diff_changed", Val::from(s.diff.changed)),
        ("diff_added", Val::from(s.diff.added)),
        ("diff_removed", Val::from(s.diff.removed)),
        ("diff_patch_bytes", Val::from(s.diff.patch_bytes)),
        // Reuse.
        (
            "reuse_md5_present_in_old",
            Val::from(s.reuse.md5_present_in_old),
        ),
        (
            "reuse_cross_file_only_chunks",
            Val::from(s.reuse.cross_file_only_chunks),
        ),
        (
            "reuse_cross_file_bytes_forgone",
            Val::from(s.reuse.cross_file_bytes_forgone),
        ),
        // Close-out and transport.
        ("documents_written", Val::from(s.http.documents_written)),
        ("requests", Val::from(s.http.requests)),
        ("bytes_downloaded", Val::from(s.http.bytes_downloaded)),
        // The tag probe.
        ("tags_probed", Val::from(s.probe.probed)),
        ("tags_served", Val::from(s.probe.served)),
        (
            "tags_served_outside_diff_tags",
            Val::from(s.probe.served_outside_diff_tags),
        ),
        (
            "legacy_endpoint_majors_behind",
            Val::from(s.legacy_endpoint_majors_behind),
        ),
        ("exit_code", Val::from(s.exit_code)),
    ])
}

/// The one place in the crate that writes to stdout.
///
/// Everything else on the data plane goes through [`Record::emit`], so there is
/// no second writer for a record to escape from.
pub fn emit(line: &str) {
    // `writeln!` into an explicitly-locked handle, rather than `println!`.
    //
    // `println!` takes the stdout lock itself and then checks whether the
    // handle is a terminal on every single call, so it pays for a "maybe a user
    // is watching" decision once per record. `list` is the only unbounded
    // producer — one `BUNDLE` per bundle under a root — and for the
    // single-record commands this is noise. The handle is flushed by the
    // `LineWriter` behind stdout on `drop`, and `exit_with` flushes too.
    let _guard = STDOUT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    if let Some(rest) = line.strip_prefix("SUMMARY ") {
        tracing::debug!(summary = rest, "summary");
    } else {
        tracing::debug!(line, "stdout");
    }
}

#[cfg(test)]
mod tests;
