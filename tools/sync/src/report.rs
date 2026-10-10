//! The stdout data plane: one structured record per line, rendered as text or
//! JSON.
//!
//! Everything this tool *reports* — a difference, a plan action, a summary — is a
//! [`Record`]: an event name plus named fields, held once and rendered twice.
//! `--output text` prints the human line and `--output json` prints the same
//! record as a JSON object, so the two modes cannot drift apart. That is the
//! whole reason the printer is here rather than a `println!` at each call site:
//! there is one definition of what a `MISSING` line means, and both the user-facing
//! text and the machine-readable form are read out of it.
//!
//! Diagnostics are a different channel and do not come through here. They are
//! `tracing` events on stderr (and optionally a JSON log file) — see
//! [`crate::logging`]. A record is logged *as well as* printed, as one structured
//! `debug` event carrying the same `event` name and fields, so the log file
//! carries the same data plane a caller parsing stdout would see.
//!
//! ## The two renderings
//!
//! | record | `--output text` | `--output json` |
//! | --- | --- | --- |
//! | `missing` | `MISSING a.txt` | `{"event":"missing","path":"a.txt"}` |
//! | `case-mismatch` | `CASE-MISMATCH a.txt <=> A.txt` | `{"event":"case-mismatch","src":"a.txt","dst":"A.txt"}` |
//! | `rename` | `RENAME data.txt -> Data.txt` | `{"event":"rename","from":"data.txt","to":"Data.txt"}` |
//! | `copy` | `COPY a.txt` | `{"event":"copy","path":"a.txt"}` |
//! | `update` | `update C:/w files=3 dirs=2 algos=[md5]` | `{"event":"update","dir":"C:/w","files":3,...}` |
//! | `summary` (diff) | `SUMMARY missing=0 ... total_diff=0` | `{"event":"summary","missing":0,...,"total_diff":0}` |
//!
//! The event name is the text label lowercased, so `MISSING`/`missing` and
//! `FIX-DIR`/`fix-dir` cannot be spelled two ways. `SUMMARY` is `summary` in both
//! the diff report and the sync report: a run emits exactly one, so a consumer
//! reads "the last line is the summary" without knowing which command it ran.
//!
//! JSON is newline-delimited — one object per line, no enclosing array — so a
//! caller can stream it and still recover the exact ordering the plan applies in.
//! Within one object the keys are *names*: read them by name, never by position.

use serde_json::{Map, Value};
use tracing::debug;

use crate::diff::Diff;

/// How stdout records are rendered. Selected by the global `--output` flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum OutputFormat {
    /// One human-readable line per record: `MISSING a.txt`, `COPY a.txt`,
    /// `SUMMARY renamed=2 mkdir=1 ...`. The default, and the format the docs
    /// and the golden tests are written against.
    #[default]
    Text,
    /// One JSON object per line (NDJSON), same records, same order:
    /// `{"event":"missing","path":"a.txt"}`.
    Json,
}

impl OutputFormat {
    /// The value as it appears in `--output`, and in a span field.
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

/// How a record's fields are laid out on a text line.
///
/// The JSON rendering does not care — a record is always an object of named
/// fields — but the text format has three shapes in the wild (a bare path, a
/// `k=v` summary, and a joined pair), and the shape belongs to the record rather
/// than to the printer so a new record cannot pick one by accident.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    /// `LABEL <first> k=v k=v` — the first field is bare, the rest keyed. Used
    /// by the one-path actions (`COPY a.txt`) and by `update`, whose text line
    /// leads with the directory and then counts.
    Positional,
    /// `LABEL k=v k=v` — every field keyed. Used by both `SUMMARY` lines.
    Keyed,
    /// `LABEL <first> SEP <second>` — two bare fields joined by `SEP`, with a
    /// space on each side of it. Used by `RENAME` (`->`) and `CASE-MISMATCH`
    /// (`<=>`).
    Pair(&'static str),
}

/// One stdout record: an event name plus its fields, held once and rendered
/// twice.
///
/// Field order is report order — the order the records are emitted in is the
/// order the plan applies in, and a `SUMMARY`'s fields read in the order a reader
/// expects them to.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    /// Text label, e.g. `MISSING`. Lowercased to get the JSON `event`.
    label: &'static str,
    layout: Layout,
    fields: Vec<(&'static str, Value)>,
}

impl Record {
    /// A record whose first field is bare and the rest keyed: `COPY a.txt`.
    pub fn path(label: &'static str, path: &str) -> Self {
        Self::positional(label, "path", path)
    }

    /// A record leading with a bare field under a chosen name.
    ///
    /// `path` is the common case and gets its own spelling; `update` needs a
    /// `dir`, and naming its field after what it actually is beats reusing
    /// `path` for a directory.
    pub fn positional(label: &'static str, first: &'static str, value: &str) -> Self {
        Self {
            label,
            layout: Layout::Positional,
            fields: Vec::new(),
        }
        .put(first, value)
    }

    /// A record of two bare fields joined by `sep`: `RENAME a -> b`.
    pub fn pair(
        label: &'static str,
        sep: &'static str,
        first: &'static str,
        first_value: &str,
        second: &'static str,
        second_value: &str,
    ) -> Self {
        Self {
            label,
            layout: Layout::Pair(sep),
            fields: Vec::new(),
        }
        .put(first, first_value)
        .put(second, second_value)
    }

    /// A record whose every field is keyed: `SUMMARY missing=0 ...`.
    pub fn keyed(label: &'static str) -> Self {
        Self {
            label,
            layout: Layout::Keyed,
            fields: Vec::new(),
        }
    }

    /// Append a field. Declaration order is report order, which is what the text
    /// line renders; JSON consumers read by key.
    pub fn put(mut self, key: &'static str, value: impl Into<Value>) -> Self {
        self.fields.push((key, value.into()));
        self
    }

    /// The text label, e.g. `MISSING`.
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// The JSON `event` name: the text label lowercased.
    pub fn event(&self) -> String {
        self.label.to_ascii_lowercase()
    }

    /// The fields, in report order, without the `event` name.
    pub fn fields(&self) -> &[(&'static str, Value)] {
        &self.fields
    }

    /// The JSON object for this record: the `event` name plus every field.
    ///
    /// Read by key, not by position. The record *order* in the stream is the
    /// contract — it is the order the plan applies in — while the keys within one
    /// object are names, and a JSON consumer must not depend on where they land.
    pub fn json(&self) -> Value {
        let mut m = Map::new();
        m.insert("event".into(), Value::String(self.event()));
        for (k, v) in &self.fields {
            m.insert((*k).to_string(), v.clone());
        }
        Value::Object(m)
    }

    /// The JSON object for this record's fields alone, as compact JSON — the
    /// payload the matching `tracing` event carries.
    pub fn fields_json(&self) -> String {
        let mut m = Map::new();
        for (k, v) in &self.fields {
            m.insert((*k).to_string(), v.clone());
        }
        Value::Object(m).to_string()
    }

    /// Print and log this record in `format`.
    ///
    /// A shorthand for the one-off record — the single line `update` reports —
    /// where threading a [`Report`] through would be noise. Anywhere that emits
    /// more than one record, hold a `Report` and call [`Report::emit`] instead,
    /// so the format is read once.
    pub fn emit(self, format: OutputFormat) {
        Report::new(format).emit(self);
    }

    /// The human line for this record, with no trailing newline.
    pub fn text(&self) -> String {
        let mut out = self.label.to_string();
        match self.layout {
            Layout::Positional => {
                for (i, (k, v)) in self.fields.iter().enumerate() {
                    out.push(' ');
                    if i > 0 {
                        out.push_str(k);
                        out.push('=');
                    }
                    out.push_str(&bare(v));
                }
            }
            Layout::Keyed => {
                for (k, v) in &self.fields {
                    out.push(' ');
                    out.push_str(k);
                    out.push('=');
                    out.push_str(&bare(v));
                }
            }
            Layout::Pair(sep) => {
                for (i, v) in self.fields.iter().map(|(_, v)| v).enumerate() {
                    if i > 0 {
                        out.push(' ');
                        out.push_str(sep);
                        out.push(' ');
                    } else {
                        out.push(' ');
                    }
                    out.push_str(&bare(v));
                }
            }
        }
        out
    }
}

/// A value as it appears inside a text line: bare, never quoted, arrays joined
/// with `,` inside `[]`.
fn bare(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => format!("[{}]", a.iter().map(bare).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

/// Emits stdout records in the run's chosen [`OutputFormat`].
///
/// Cheap to construct and to copy (`&Report` threads through), and the only
/// writer of stdout in the crate: a command reaches the data plane only by
/// handing a `Record` to [`Report::emit`].
#[derive(Clone, Copy, Debug)]
pub struct Report {
    format: OutputFormat,
}

impl Report {
    pub fn new(format: OutputFormat) -> Self {
        Self { format }
    }

    /// Print one record, and log it.
    ///
    /// The `tracing` event is emitted here rather than beside each call site so
    /// that "what was printed" and "what was logged" are the same event, and a
    /// record whose payload has no natural fields of its own still lands in the
    /// log file. It is `debug`: the data plane is the answer, the log is the
    /// narration around it.
    pub fn emit(&self, rec: Record) {
        debug!(event = %rec.event(), fields = %rec.fields_json(), "report");
        println!("{}", self.render(&rec));
    }

    /// The line this record prints in the run's format, without the newline.
    pub fn render(&self, rec: &Record) -> String {
        match self.format {
            OutputFormat::Text => rec.text(),
            OutputFormat::Json => rec.json().to_string(),
        }
    }
}

/// Every record `compare` and `compare-self` report for a [`Diff`], in report
/// order, ending with the summary.
///
/// The list is data, not lines: [`Report`] renders it as text or JSON, and
/// `report_diff` is the only caller that prints it. Printing and asserting read
/// the same list, so what a test states and what a user sees cannot drift apart
/// — there is one definition of a `CHANGED` line, not one in the printer and
/// another in each test.
///
/// ## `why`
///
/// `why` adds a `why=<tag>` field to the `CHANGED` records and changes nothing
/// else — not the record count, not the order, not the summary, and not the exit
/// code, which is computed from [`Diff`] before this function is called.
///
/// It is a parameter rather than a second list of records because the two would
/// have to agree, and two definitions of the same document is the thing this
/// module exists to prevent. `MISSING`, `EXTRA`, `TYPE-CONFLICT` and
/// `CASE-MISMATCH` never carry one: the bucket name is already the explanation,
/// and a tag there would be an invention.
///
/// A `CHANGED` path with no recorded reason prints without the field rather than
/// with an empty one, so `why=` is never a value a parser has to special-case.
pub fn verdict(diff: &Diff, why: bool) -> Vec<Record> {
    let mut out: Vec<Record> = Vec::new();
    for r in &diff.missing {
        out.push(Record::path("MISSING", r));
    }
    for r in &diff.extra {
        out.push(Record::path("EXTRA", r));
    }
    for r in &diff.changed {
        let rec = Record::path("CHANGED", r);
        out.push(match diff.why_tag(r) {
            Some(tag) if why => rec.put("why", tag),
            _ => rec,
        });
    }
    for r in &diff.type_conflict {
        out.push(Record::path("TYPE-CONFLICT", r));
    }
    for (a, b) in &diff.case_mismatch {
        out.push(Record::pair("CASE-MISMATCH", "<=>", "src", a, "dst", b));
    }
    out.push(
        Record::keyed("SUMMARY")
            .put("missing", diff.missing.len())
            .put("extra", diff.extra.len())
            .put("changed", diff.changed.len())
            .put("type_conflict", diff.type_conflict.len())
            .put("case_mismatch", diff.case_mismatch.len())
            .put("total_diff", diff.total()),
    );
    out
}

/// The one record `update` reports: what the folder holds now, under the
/// algorithms this run computed.
///
/// `dir` leads the text line and is a named field in JSON, so a caller reading
/// either can attribute the counts.
pub fn update_summary(
    dir: &std::path::Path,
    files: usize,
    dirs: usize,
    algos: &[String],
) -> Record {
    Record::positional("update", "dir", &dir.display().to_string())
        .put("files", files)
        .put("dirs", dirs)
        .put("algos", algos.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn text_and_json_are_two_renderings_of_one_record() {
        let rec = Record::path("MISSING", "a b.txt");
        assert_eq!(rec.text(), "MISSING a b.txt");
        assert_eq!(
            rec.json().to_string(),
            r#"{"event":"missing","path":"a b.txt"}"#,
            "the JSON form is the same record, and the event is the label lowercased"
        );
    }

    #[test]
    fn layouts_render_the_documented_lines() {
        assert_eq!(
            Record::pair("RENAME", "->", "from", "data.txt", "to", "Data.txt").text(),
            "RENAME data.txt -> Data.txt"
        );
        assert_eq!(
            Record::pair("CASE-MISMATCH", "<=>", "src", "a.txt", "dst", "A.txt").text(),
            "CASE-MISMATCH a.txt <=> A.txt"
        );
        assert_eq!(
            Record::keyed("SUMMARY")
                .put("missing", 0usize)
                .put("total_diff", 0usize)
                .text(),
            "SUMMARY missing=0 total_diff=0"
        );
        assert_eq!(
            update_summary(&PathBuf::from("C:/w"), 3, 2, &["md5".to_string()]).text(),
            format!(
                "update {} files=3 dirs=2 algos=[md5]",
                PathBuf::from("C:/w").display()
            )
        );
    }

    /// The leading field is named for what it is. `update` reports a directory,
    /// and `dir` in JSON is worth a constructor rather than reusing `path` for a
    /// path that is not one.
    #[test]
    fn a_leading_field_is_named_for_what_it_holds() {
        let rec = update_summary(&PathBuf::from("C:/w"), 3, 2, &["md5".to_string()]);
        assert_eq!(
            rec.json(),
            json!({"event": "update", "dir": r"C:/w", "files": 3, "dirs": 2,
                   "algos": ["md5"]})
        );
    }

    /// Fields keep the order they were put in, which is the report order the text
    /// line depends on. JSON key order is not asserted: an object is read by name,
    /// so pinning where a key lands would pin an encoder detail rather than the
    /// contract.
    #[test]
    fn fields_keep_declaration_order() {
        let rec = Record::keyed("SUMMARY")
            .put("renamed", 2usize)
            .put("mkdir", 1usize)
            .put("copied", 6usize);
        assert_eq!(
            rec.fields().iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            vec!["renamed", "mkdir", "copied"]
        );
        assert_eq!(
            rec.json(),
            json!({"event": "summary", "renamed": 2, "mkdir": 1, "copied": 6})
        );
        assert_eq!(rec.text(), "SUMMARY renamed=2 mkdir=1 copied=6");
    }

    /// A path with a quote or a backslash must not break the JSON. It is escaped
    /// by the encoder, and the text form is unaffected because it never quotes.
    #[test]
    fn json_escapes_what_text_leaves_bare() {
        let rec = Record::path("COPY", r#"a"b\c.txt"#);
        assert_eq!(rec.text(), r#"COPY a"b\c.txt"#);
        assert_eq!(rec.json(), json!({"event": "copy", "path": r#"a"b\c.txt"#}));
    }
}
