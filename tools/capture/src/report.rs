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

use std::sync::Mutex;

use crate::VERSION;

/// How a record is rendered on stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
            Val::Str(s) => percent_encode(s),
            Val::Int(i) => i.to_string(),
            Val::Bool(b) => b.to_string(),
        }
    }
}

/// Percent-encode exactly the bytes that would break `k=v` tokenisation.
///
/// The set is deliberately the minimum: everything else stays as typed, so the
/// common line reads as a human wrote it.
const TEXT_ESCAPES: &[char] = &[' ', '\t', '\n', '\r', '%'];

fn percent_encode(s: &str) -> String {
    if !s.contains(TEXT_ESCAPES) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if TEXT_ESCAPES.contains(&c) {
            // Non-ASCII is encoded too, as UTF-8 bytes, so the encoding is
            // reversible on any host and not just one with the right codepage.
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).as_bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The inverse of `percent_encode`. Exposed so a consumer — and a test — can
/// read a value back without re-implementing the rule.
pub fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &s[i + 1..i + 3];
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
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
        Val::Str(v.iter().map(|b| format!("{b:02x}")).collect())
    }
}
impl From<i64> for Val {
    fn from(v: i64) -> Self {
        Val::Int(v)
    }
}
impl From<u64> for Val {
    fn from(v: u64) -> Self {
        // Byte counts are `u64` in the API types; the wire form is JSON, where a
        // negative number cannot appear, so the narrowing is exact.
        Val::Int(v as i64)
    }
}
impl From<usize> for Val {
    fn from(v: usize) -> Self {
        Val::Int(v as i64)
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

    /// Look up a field by name, for tests and for callers that want to assert
    /// on the value rather than the rendering.
    pub fn get(&self, key: &str) -> Option<&Val> {
        self.fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
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
        let event = self
            .fields
            .iter()
            .find(|(k, _)| *k == "event")
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| Val::Str(self.json_event()));
        let mut s = String::with_capacity(64 + 24 * self.fields.len());
        s.push('{');
        s.push_str(&json_string("event"));
        s.push(':');
        s.push_str(&json_value(&event));
        for (k, v) in self.fields.iter().filter(|(k, _)| *k != "event") {
            s.push(',');
            s.push_str(&json_string(k));
            s.push(':');
            s.push_str(&json_value(v));
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
fn json_value(v: &Val) -> String {
    match v {
        Val::Str(s) => json_string(s),
        Val::Int(i) => i.to_string(),
        Val::Bool(b) => b.to_string(),
    }
}

/// Writes one stdout line and logs the matching event, as a single unit.
///
/// A mutex rather than a bare `println!` because the summary can be emitted from
/// a worker in later milestones; the lock is held for exactly one line and never
/// across work.
static STDOUT_LOCK: Mutex<()> = Mutex::new(());

/// The one place in the crate that writes to stdout.
///
/// Everything else on the data plane goes through [`Record::emit`], so there is
/// no second writer for a record to escape from.
pub fn emit(line: &str) {
    let _guard = STDOUT_LOCK.lock();
    println!("{line}");
    if let Some(rest) = line.strip_prefix("SUMMARY ") {
        tracing::debug!(summary = rest, "summary");
    } else {
        tracing::debug!(line, "stdout");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value as Json;

    fn sample() -> Record {
        Record::summary("capture")
            .set("biz", "hk4e_global")
            .set("files_captured", 8_u64)
            .set("bytes_downloaded", 96_041_377_i64)
            .set("complete", true)
            .set("exit_code", 0_i32)
    }

    #[test]
    fn the_identity_fields_come_first_and_always() {
        let line = sample().to_text_line();
        assert!(
            line.starts_with("SUMMARY event=capture tool=gircap version="),
            "{line}"
        );
    }

    #[test]
    fn text_renders_every_val_kind() {
        assert_eq!(
            sample().to_text_line(),
            "SUMMARY event=capture tool=gircap version=0.1.0 biz=hk4e_global \
             files_captured=8 bytes_downloaded=96041377 complete=true exit_code=0"
        );
    }

    #[test]
    fn json_keeps_numbers_and_booleans_unquoted() {
        let v: Json = serde_json::from_str(&sample().to_json_line()).expect("one object");
        assert_eq!(v["event"], "capture");
        assert_eq!(v["files_captured"], Json::from(8));
        assert_eq!(v["bytes_downloaded"], Json::from(96_041_377_i64));
        assert_eq!(v["complete"], Json::Bool(true));
        assert_eq!(v["biz"], "hk4e_global");
    }

    #[test]
    fn the_two_renderings_carry_the_same_fields_in_the_same_order() {
        let s = sample();
        let text_line = s.to_text_line();
        let text_keys: Vec<&str> = text_line
            .split_whitespace()
            .skip(1)
            .filter_map(|kv| kv.split_once('=').map(|(k, _)| k))
            .collect();
        assert_eq!(
            text_keys,
            json_keys_in_order(&s.to_json_line()),
            "text and json disagree on the field set or its order"
        );
    }

    /// Reads the top-level keys of a flat JSON object **in the order they appear
    /// in the bytes**.
    ///
    /// Not `serde_json::from_str` into a `Value`: that map is a `BTreeMap`, so it
    /// would report the keys alphabetically and the assertion would pass no
    /// matter what order the emitter wrote. This is the one property under test
    /// here, so it has to be read off the bytes.
    fn json_keys_in_order(line: &str) -> Vec<String> {
        let b: Vec<char> = line.chars().collect();
        let mut keys = Vec::new();
        let mut i = 1; // skip '{'
        loop {
            while i < b.len() && b[i].is_whitespace() {
                i += 1;
            }
            if i >= b.len() || b[i] == '}' {
                break;
            }
            assert_eq!(b[i], '"', "expected a key at {i} in {line}");
            i += 1;
            let mut key = String::new();
            while b[i] != '"' {
                if b[i] == '\\' {
                    i += 1;
                }
                key.push(b[i]);
                i += 1;
            }
            i += 1;
            keys.push(key);

            while i < b.len() && b[i] != ':' {
                i += 1;
            }
            i += 1;
            while i < b.len() && b[i].is_whitespace() {
                i += 1;
            }
            if b[i] == '"' {
                i += 1;
                while b[i] != '"' {
                    if b[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
            } else {
                while i < b.len() && b[i] != ',' && b[i] != '}' {
                    i += 1;
                }
            }
            while i < b.len() && b[i].is_whitespace() {
                i += 1;
            }
            if i < b.len() && b[i] == ',' {
                i += 1;
                continue;
            }
            break;
        }
        keys
    }

    /// A value carrying a quote is the only thing that forces the escaping path,
    /// and a hand-rolled emitter is exactly where that path gets forgotten.
    #[test]
    fn a_quote_or_backslash_in_a_value_survives_the_json_encoding() {
        let s = Record::summary("verify")
            .set("note", r#"a "quoted", back\slashed path"#)
            .set("out", r"C:\tmp\x");
        let line = s.to_json_line();
        let v: Json = serde_json::from_str(&line).expect("still one valid object");
        assert_eq!(v["note"], r#"a "quoted", back\slashed path"#);
        assert_eq!(v["out"], r"C:\tmp\x");
        assert_eq!(
            json_keys_in_order(&line),
            vec!["event", "tool", "version", "note", "out"]
        );
    }

    /// The text form encodes whitespace but does **not** escape quotes. Inventing an
    /// escaping scheme for `"` would be a second, different encoding on the same
    /// line; the one rule is "whitespace and `%` are encoded", and a quote is
    /// just a character that happens to look meaningful to a JSON reader.
    #[test]
    fn the_text_form_encodes_whitespace_but_not_quotes() {
        let s = Record::summary("x").set("note", r#"a "quoted" value"#);
        assert!(s.to_text_line().ends_with(r#"note=a%20"quoted"%20value"#));
    }

    #[test]
    fn json_is_one_line() {
        let line = sample().to_json_line();
        assert!(!line.contains('\n'), "{line}");
        assert!(!line.contains('\r'), "{line}");
    }

    /// A value containing `=` or a space must not shift the text parsing.
    #[test]
    fn a_path_field_survives_the_text_rendering_intact() {
        let s =
            Record::summary("verify").set("artifact", r"E:\artifacts\hk4e_global\7.0.0__7.1.0\id");
        let line = s.to_text_line();
        let tail = line.rsplit_once("artifact=").unwrap().1;
        assert_eq!(
            percent_decode(tail),
            r"E:\artifacts\hk4e_global\7.0.0__7.1.0\id"
        );
    }

    /// The whole point of the text form: split it on whitespace and every token
    /// after the first is a `k=v` pair. A value carrying a space used to produce
    /// bare words in the middle of the line, which is what the contract test
    /// found.
    #[test]
    fn every_text_token_is_a_key_value_pair() {
        let line = Record::summary("capture")
            .set("exit_code_meaning", "an assertion failed")
            .set("artifact", r"E:\a b\artifacts")
            .set("pct", "100%")
            .set("tab", "a\tb")
            .to_text_line();
        let mut decoded = Vec::new();
        for token in line.split_whitespace().skip(1) {
            let (k, v) = token
                .split_once('=')
                .unwrap_or_else(|| panic!("not a k=v pair: {token:?} in {line:?}"));
            decoded.push((k.to_string(), percent_decode(v)));
        }
        assert_eq!(
            decoded,
            vec![
                ("event".into(), "capture".to_string()),
                ("tool".into(), "gircap".to_string()),
                ("version".into(), crate::VERSION.to_string()),
                (
                    "exit_code_meaning".into(),
                    "an assertion failed".to_string()
                ),
                ("artifact".into(), r"E:\a b\artifacts".to_string()),
                ("pct".into(), "100%".to_string()),
                ("tab".into(), "a\tb".to_string()),
            ]
        );
    }

    /// A non-terminal record carries no `event` field, and still names itself in
    /// JSON — by its label, lowercased. That is what makes `{"event":"bundle"}`
    /// dispatchable without the record having to be told which command is
    /// running once per line.
    #[test]
    fn a_non_terminal_record_names_itself_by_its_label() {
        let rec = Record::labeled("BUNDLE")
            .set("path", "hk4e_global/7.0.0__7.1.0/20261015T111345Z")
            .set("blobs", 38_u64)
            .set("complete", true);
        assert_eq!(rec.label(), "BUNDLE");
        assert_eq!(
            rec.to_text_line(),
            "BUNDLE tool=gircap version=0.1.0 \
             path=hk4e_global/7.0.0__7.1.0/20261015T111345Z blobs=38 complete=true"
        );
        let line = rec.to_json_line();
        assert_eq!(
            line,
            r#"{"event":"bundle","tool":"gircap","version":"0.1.0","path":"hk4e_global/7.0.0__7.1.0/20261015T111345Z","blobs":38,"complete":true}"#,
            "`event` leads, and is the label lowercased"
        );
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["event"], "bundle");
        assert_eq!(v["blobs"], 38);
        assert_eq!(v["complete"], true);
    }

    /// `event` is the discriminator on both planes, and it must be spelled the
    /// same way in both — and **first**, so a consumer can dispatch without
    /// having parsed the rest of the line. The terminal record's `event` is the
    /// command, not the label; `list` emits `BUNDLE` records and one `list`
    /// summary, and a reader that saw `event: "list"` seven times would learn
    /// nothing the label does not already say.
    #[test]
    fn event_leads_on_both_planes_and_is_never_written_twice() {
        let summary = Record::summary("list").set("bundles", 2_u64);
        assert!(summary.to_text_line().starts_with("SUMMARY event=list "));
        assert_eq!(
            summary.to_json_line(),
            r#"{"event":"list","tool":"gircap","version":"0.1.0","bundles":2}"#
        );
        let bundle = Record::labeled("BUNDLE").set("path", "x");
        assert_eq!(
            bundle.to_json_line(),
            r#"{"event":"bundle","tool":"gircap","version":"0.1.0","path":"x"}"#
        );
        // No record may emit the key twice, whichever plane produced it.
        for line in [summary.to_json_line(), bundle.to_json_line()] {
            assert_eq!(
                line.matches("\"event\":").count(),
                1,
                "`event` appears more than once: {line}"
            );
        }
    }

    #[test]
    fn percent_encoding_round_trips_and_leaves_ordinary_values_alone() {
        for s in [
            "plain",
            "no spaces here",
            r"E:\artifacts\hk4e_global",
            "GenshinImpact_Data/Plugins/HoYoChannel.dll",
            "spaces and  tabs\tand\nnewlines",
            "100% done",
            "%41 already encoded",
            "unicode ✓ and ünïcödé",
        ] {
            let encoded = percent_encode(s);
            assert!(!encoded.contains(' '), "{s:?} -> {encoded:?}");
            assert_eq!(percent_decode(&encoded), s, "round trip failed for {s:?}");
        }
        // The no-op cases really are no-ops: a line that a human reads must not
        // come back full of percent signs.
        assert_eq!(percent_encode("hk4e_global"), "hk4e_global");
        assert_eq!(percent_decode("hk4e_global"), "hk4e_global");
    }

    /// A malformed escape is data, not a decode failure: `%` followed by
    /// non-hex is passed through rather than dropped, so a value can never
    /// silently lose characters on the way back.
    #[test]
    fn a_malformed_percent_sequence_passes_through() {
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%4"), "%4");
        assert_eq!(percent_decode("100%"), "100%");
    }

    /// A name with a space is the reader's problem, and the tool must not paper
    /// over it by inserting a quote that changes the bytes. Only *values* are
    /// encoded; keys are `&'static str`s this crate chose.
    #[test]
    fn names_with_spaces_are_the_documented_limitation() {
        let line = Record::summary("x").set("some key", 1_i32).to_text_line();
        assert!(line.ends_with("some key=1"), "{line}");
    }

    #[test]
    fn duplicate_names_are_kept_not_merged() {
        let s = Record::summary("x").set("n", 1_i32).set("n", 2_i32);
        assert_eq!(s.get("n"), Some(&Val::Int(1)));
        assert!(s.to_text_line().ends_with("n=1 n=2"));
    }

    #[test]
    fn output_format_parses_both_spellings_and_rejects_others() {
        assert_eq!("text".parse(), Ok(OutputFormat::Text));
        assert_eq!("json".parse(), Ok(OutputFormat::Json));
        assert!("xml".parse::<OutputFormat>().is_err());
        assert_eq!(OutputFormat::default(), OutputFormat::Text);
    }
}
