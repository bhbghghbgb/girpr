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
/// **No boolean.** Every flag-shaped fact in the contract is a `1`/`0` integer
/// (`scoped=1`, `config_bumped=0`, `limited=1`), with `SUMMARY` the documented
/// shape. Leaving `Bool` out is what stops a `true`-in-one-object /
/// `1`-in-the-other collision from being reintroduced.
///
/// The two integer widths exist so a `u64` count is never squeezed through an
/// `i64`: a saturated estimate is exactly `u64::MAX`, and `as i64` would print
/// it as `-1` — a total that is not merely wrong but has the wrong sign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Val {
    Str(String),
    /// An unsigned count or byte total.
    UInt(u64),
    /// A signed number: the process exit code.
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
        Val::UInt(v)
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
        Val::UInt(u) => u.to_string(),
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
        Val::UInt(u) => u.to_string(),
        Val::Int(i) => i.to_string(),
    }
}
