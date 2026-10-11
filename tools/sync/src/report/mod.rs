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

mod record;

pub use record::Record;

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
///
/// ## `show_identical`
///
/// `show_identical` adds one `IDENTICAL` record per pair that came out equal,
/// each with its own reason, and an `identical=N` field on the `SUMMARY`. It
/// reports what is already in [`Diff::identical`] and computes nothing: the bucket
/// is filled by [`crate::diff::diff_maps`] only when asked, so an unflagged caller
/// has no equal set to print and this flag changes no verdict.
///
/// The field is conditional rather than always present so that an unflagged run
/// stays byte-identical to a run that never passes it. A consumer reading
/// `SUMMARY` must therefore treat `identical` as optional, which is the same
/// treatment `output`-format-dependent fields already get.
pub fn verdict(diff: &Diff, why: bool, show_identical: bool) -> Vec<Record> {
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
    // The equal set, between the differences and the summary. A directory on both
    // sides is in here too: leaving it out would make the flag's output look
    // arbitrarily partial, and "presence only" is a real answer to "are these in
    // step".
    //
    // Gated on `show_identical` rather than on the bucket being non-empty. The
    // bucket is empty whenever the diff was not asked for it, so the two usually
    // agree — but a caller that fills `Diff::identical` by hand and passes the flag
    // off must get the flag's answer, and a caller that passes the flag on against an
    // empty bucket must get `identical=0` rather than a missing field.
    if show_identical {
        for r in &diff.identical {
            let rec = Record::path("IDENTICAL", r);
            out.push(match diff.why_tag(r) {
                Some(tag) if why => rec.put("why", tag),
                _ => rec,
            });
        }
    }
    let mut summary = Record::keyed("SUMMARY")
        .put("missing", diff.missing.len())
        .put("extra", diff.extra.len())
        .put("changed", diff.changed.len())
        .put("type_conflict", diff.type_conflict.len())
        .put("case_mismatch", diff.case_mismatch.len());
    if show_identical {
        summary = summary.put("identical", diff.identical.len());
    }
    // `total_diff` last and last-but-one in the meaning too: it counts differences
    // only, so `identical` never reaches it.
    out.push(summary.put("total_diff", diff.total()));
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

    /// A path with a quote or a backslash must not break the JSON. It is escaped
    /// by the encoder, and the text form is unaffected because it never quotes.
    #[test]
    fn json_escapes_what_text_leaves_bare() {
        let rec = Record::path("COPY", r#"a"b\c.txt"#);
        assert_eq!(rec.text(), r#"COPY a"b\c.txt"#);
        assert_eq!(rec.json(), json!({"event": "copy", "path": r#"a"b\c.txt"#}));
    }
}