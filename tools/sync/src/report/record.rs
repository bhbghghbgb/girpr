//! [`Record`]: one stdout record, held once and rendered twice.
//!
//! The event name, the fields, and the layout that turns them into a text line.
//! Nothing here knows about the diff or about any command — a record is a value
//! first and a line second.

use std::fmt::Write as _;

use serde_json::{Map, Value};

use super::{OutputFormat, Report};

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
        let mut out = String::with_capacity(self.label.len());
        self.push_event(&mut out);
        out
    }

    /// Append the `event` name, lowercased but not JSON-quoted.
    fn push_event(&self, out: &mut String) {
        for c in self.label.chars() {
            out.push(c.to_ascii_lowercase());
        }
    }

    /// Append the JSON `event` name as a quoted string.
    fn push_json_event(&self, out: &mut String) {
        out.push('"');
        for c in self.label.chars() {
            push_json_char(out, c.to_ascii_lowercase());
        }
        out.push('"');
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

    /// Write the JSON object for this record into `out`, with no trailing
    /// newline.
    ///
    /// Keys are sorted alphabetically, `event` among them, so the bytes match
    /// the object [`Record::json`] builds. Values are written straight into
    /// `out` instead of being cloned into a map first.
    pub fn write_json(&self, out: &mut String) {
        // A record has a handful of fields, so sorting a small vector of
        // borrows beats anything that allocates per key.
        let mut fields: Vec<(&str, &Value)> = self.fields.iter().map(|(k, v)| (*k, v)).collect();
        fields.sort_unstable_by_key(|(k, _)| *k);
        let event_at = fields.partition_point(|(k, _)| *k < "event");

        out.push('{');
        let mut first = true;
        for (k, v) in &fields[..event_at] {
            push_json_member(out, &mut first, k);
            push_json_value(out, v);
        }
        push_json_member(out, &mut first, "event");
        self.push_json_event(out);
        for (k, v) in &fields[event_at..] {
            push_json_member(out, &mut first, k);
            push_json_value(out, v);
        }
        out.push('}');
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
        let mut out = String::new();
        self.push_text(&mut out);
        out
    }

    /// Append the human line for this record.
    fn push_text(&self, out: &mut String) {
        out.push_str(self.label);
        match self.layout {
            Layout::Positional => {
                for (i, (k, v)) in self.fields.iter().enumerate() {
                    out.push(' ');
                    if i > 0 {
                        out.push_str(k);
                        out.push('=');
                    }
                    push_bare(out, v);
                }
            }
            Layout::Keyed => {
                for (k, v) in &self.fields {
                    out.push(' ');
                    out.push_str(k);
                    out.push('=');
                    push_bare(out, v);
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
                    push_bare(out, v);
                }
            }
        }
    }
}

/// Append a value as it appears inside a text line: bare, never quoted, arrays
/// joined with `,` inside `[]`.
fn push_bare(out: &mut String, v: &Value) {
    match v {
        Value::String(s) => out.push_str(s),
        Value::Array(a) => {
            out.push('[');
            for (i, item) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_bare(out, item);
            }
            out.push(']');
        }
        other => {
            let _ = write!(out, "{other}");
        }
    }
}

/// Append one `"key":` member prefix, with a separating comma when it is not
/// the first.
fn push_json_member(out: &mut String, first: &mut bool, key: &str) {
    if !*first {
        out.push(',');
    }
    *first = false;
    push_json_string(out, key);
    out.push(':');
}

/// Append a value as JSON.
fn push_json_value(out: &mut String, v: &Value) {
    match v {
        Value::String(s) => push_json_string(out, s),
        // Numbers, booleans, null and nested containers are already rendered
        // exactly this way by the value's own formatter.
        other => {
            let _ = write!(out, "{other}");
        }
    }
}

/// Append `value` as a quoted JSON string, escaped the way `serde_json`
/// escapes it.
fn push_json_string(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        push_json_char(out, c);
    }
    out.push('"');
}

/// Append one character of a JSON string, escaped if it needs to be.
fn push_json_char(out: &mut String, c: char) {
    match c {
        '"' => out.push_str("\\\""),
        '\\' => out.push_str("\\\\"),
        '\u{08}' => out.push_str("\\b"),
        '\u{0c}' => out.push_str("\\f"),
        '\n' => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        c if c < ' ' => {
            let _ = write!(out, "\\u{:04x}", u32::from(c));
        }
        c => out.push(c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::update_summary;
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
            update_summary(&PathBuf::from("C:/w"), 3, 2, &[crate::hash::HashAlg::Md5]).text(),
            format!(
                "update {} files=3 dirs=2 algos=[md5]",
                PathBuf::from("C:/w").display()
            )
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

    /// The direct writer is the same bytes as serialising the value form:
    /// keys sorted, `event` in its sorted place, and the signed/unsigned
    /// distinction intact.
    #[test]
    fn write_json_writes_the_same_bytes_as_the_value_form() {
        let rec = Record::keyed("SUMMARY")
            .put("renamed", 2usize)
            .put("delta", -1i64)
            .put("total", 1u64);
        let mut out = String::new();
        rec.write_json(&mut out);
        assert_eq!(out, rec.json().to_string());
        assert_eq!(
            out,
            r#"{"delta":-1,"event":"summary","renamed":2,"total":1}"#
        );
        assert_eq!(rec.text(), "SUMMARY renamed=2 delta=-1 total=1");
    }

    /// Strings and arrays are escaped byte for byte like the value form, and
    /// the text form still leaves them bare.
    #[test]
    fn write_json_escapes_like_the_value_form() {
        let awkward = "a\"b\\c\nd\te\r\u{08}\u{0c}\u{1f}\u{7f}é";
        let rec = Record::positional("update", "dir", awkward)
            .put("files", 3usize)
            .put("algos", vec!["md5", "sha1"]);
        let mut out = String::new();
        rec.write_json(&mut out);
        assert_eq!(out, rec.json().to_string());
        assert_eq!(
            rec.text(),
            format!("update {awkward} files=3 algos=[md5,sha1]")
        );
    }
}
