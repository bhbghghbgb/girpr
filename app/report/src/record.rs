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
//! `tracing` events on stderr (and optionally a `TRACE` log file — see the
//! consuming binary's `logging` module).

use std::borrow::Cow;

use serde::ser::{Serialize, Serializer};

use girpr_report_core::{Order, Record as CoreRecord};

/// How often the background reporter emits `PROGRESS` during long phases.
pub const PROGRESS_INTERVAL_SECS: u64 = 10;

/// How stdout records are rendered. Selected by the global `--output` flag.
///
/// The value set, spelling and text rendering now live in `girpr-report-core`,
/// alongside the writer this plane builds on. The possible-value descriptions
/// this binary publishes in `--help` stay here: they describe *these* records
/// and are supplied to the shared CLI flag as data.
pub use girpr_report_core::OutputFormat;

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

impl Serialize for Val {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // One JSON value per variant, in this declaration order: `Str` is a
        // string, `UInt` an unsigned integer, `Int` a signed integer. No bool,
        // so a flag-shaped fact can never render `true` here and `1` there.
        match self {
            Val::Str(s) => serializer.serialize_str(s),
            Val::UInt(u) => serializer.serialize_u64(*u),
            Val::Int(i) => serializer.serialize_i64(*i),
        }
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
pub struct Record(CoreRecord<Val>);

impl Record {
    /// A record whose every field is keyed: `SUMMARY total=0 skipped=0 …`.
    ///
    /// Every `girpr` record has this shape. There is no positional variant
    /// because there is no record that wants one, and a constructor that could
    /// produce a line whose leading value is unkeyed is a constructor that can
    /// be used to break the parse plane again.
    pub fn keyed(label: &'static str) -> Self {
        Self(CoreRecord::new(label))
    }

    /// Append a field. Declaration order is report order, which is what both
    /// renderings preserve.
    pub fn put(mut self, key: &'static str, value: impl Into<Val>) -> Self {
        self.0.push(key, value.into());
        self
    }

    /// The text label, e.g. `SUMMARY`.
    pub fn label(&self) -> &'static str {
        self.0.label()
    }

    /// The fields, in report order, without the `event` name.
    pub fn fields(&self) -> &[(&'static str, Val)] {
        self.0.fields()
    }

    /// The human line for this record, without the trailing newline:
    /// `SUMMARY total=0 skipped=0`.
    pub fn text(&self) -> String {
        let mut out = String::from(self.label());
        for (k, v) in self.fields() {
            out.push(' ');
            out.push_str(k);
            out.push('=');
            out.push_str(&bare(v));
        }
        out
    }

    /// The JSON object for this record, as one line: `{"event":"summary",…}`.
    ///
    /// Rendered through [`serde::Serialize`], delegating the ordered map to
    /// `girpr_report_core`. Keys keep report order without `serde_json`'s
    /// `preserve_order` feature — enabling that feature here would silently
    /// reorder the JSON another tool in this workspace emits. `serde_json` also
    /// owns escaping, so control characters are byte-for-byte what it produced
    /// before (the old fast path already fell back to it for any value that
    /// needed escaping). The core is given this crate's own `Val`s and a
    /// lowercased label, so it cannot introduce a boolean or a foreign order.
    pub fn json(&self) -> String {
        // Serializing to a `String` with this impl is infallible; the fallback
        // only keeps the signature panic-free, as the old renderer's did.
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// The line this record prints in `format`, without the newline.
    pub fn render(&self, format: OutputFormat) -> String {
        match format {
            OutputFormat::Text => self.text(),
            OutputFormat::Json => self.json(),
        }
    }
}

impl Serialize for Record {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // `event` first, then each field in report (declaration) order. The
        // core's `serialize_entry` yields entries in exactly this order, so no
        // `preserve_order` feature is needed — that feature only affects
        // `serde_json::Map`/`Value`, which this renderer never builds. The
        // event value is this label lowercased; no `Val` of this crate carries
        // a boolean, so a flag-shaped fact stays a `1`/`0` integer here.
        let event = self.label().to_ascii_lowercase();
        self.0
            .write_json(serializer, "event", &event, Order::EventFirst)
    }
}

/// A value as it appears inside a text line: bare, never quoted.
fn bare(v: &Val) -> Cow<'_, str> {
    match v {
        Val::Str(s) => Cow::Borrowed(s),
        Val::UInt(u) => Cow::Owned(u.to_string()),
        Val::Int(i) => Cow::Owned(i.to_string()),
    }
}
