//! The ordered-JSON and line-writing mechanics below the three report planes.
//!
//! `girpr` writes an event-first object in declaration order, `girsync` writes
//! an alphabetically sorted object with `event` in its sorted place, and
//! `gircap` writes an event-first object whose `event` value is a real field.
//! The field orders, value vocabularies, escaping and boolean spellings are
//! deliberately different and stay in each tool. What is common is only the
//! *mechanic*: write an ordered JSON object from a list of `(key, value)`
//! pairs supplied by the caller, and write one rendered line plus its newline.
//!
//! Nothing here names a field, a value, a flag or a program. Values arrive as
//! `Serialize` implementors owned by the caller, so this crate neither defines
//! nor normalises a value vocabulary: `serde` performs JSON escaping on the
//! caller's spelling, a tool that forbids booleans simply never passes one, and
//! a tool that renders `true` keeps rendering `true`. The text form's quoting or
//! percent-encoding is also the caller's — [`write_line`] writes an already
//! rendered string and never interprets it.
//!
//! No `serde_json` `preserve_order` feature is enabled or needed: keys are
//! emitted in the order this crate chooses through `serialize_entry`, which is
//! independent of how the workspace unifies that feature for `serde_json`'s
//! `Map`/`Value`.

use std::io::Write;

use serde::ser::{Serialize, SerializeMap, Serializer};

/// The emission order of a JSON object's entries.
///
/// Both variants keep the caller's field values verbatim; only the order in
/// which entries are handed to the serializer changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// `event` is written first, then the fields in the order supplied. This is
    /// the `girpr` and `gircap` contract.
    EventFirst,
    /// Every entry — fields and `event` alike — is sorted by key, so `event`
    /// lands in its alphabetical position. This is the `girsync` contract.
    Alphabetical,
}

/// Write one JSON object: an `event` entry plus the keyed `fields`.
///
/// `fields` is the caller's field list in report order. [`Order::EventFirst`]
/// emits it unchanged; [`Order::Alphabetical`] sorts a copy of it. Both pass
/// each value straight through the serializer, so escaping, number widths and
/// boolean rendering remain the caller's (`Serialize`) decision, and no field
/// order or value is normalised here.
///
/// The caller owns the `event` value too. A tool that derives it from a label
/// passes a lowercased string; one that carries a real `event` field passes
/// that field's value and filters it out of `fields` so it is not written
/// twice. The `event_key` parameter names that slot for the alphabetical
/// placement rule.
///
/// Serialization is infallible for the caller's primitive values; the result
/// is kept as `Result` so a caller's `Serialize` impl can still fail.
pub fn write_json_object<E, V, S>(
    serializer: S,
    event_key: &str,
    event_value: &E,
    fields: &[(&str, V)],
    order: Order,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    E: Serialize,
    V: Serialize,
{
    match order {
        Order::EventFirst => {
            let mut map = serializer.serialize_map(Some(1 + fields.len()))?;
            map.serialize_entry(event_key, event_value)?;
            for (key, value) in fields {
                map.serialize_entry(key, value)?;
            }
            map.end()
        }
        Order::Alphabetical => {
            let mut entries: Vec<(&str, &V)> = fields.iter().map(|(k, v)| (*k, v)).collect();
            entries.sort_unstable_by_key(|(k, _)| *k);
            let event_at = entries.partition_point(|(k, _)| *k < event_key);
            let mut map = serializer.serialize_map(Some(1 + entries.len()))?;
            for (key, value) in &entries[..event_at] {
                map.serialize_entry(key, value)?;
            }
            map.serialize_entry(event_key, event_value)?;
            for (key, value) in &entries[event_at..] {
                map.serialize_entry(key, value)?;
            }
            map.end()
        }
    }
}

/// Write one already-rendered line plus its newline.
///
/// The line is opaque: any quoting, percent-encoding or case is the caller's.
/// A closed or failing writer propagates its error so a caller can decide
/// whether it is fatal (the repair binary treats a broken pipe as the end of
/// its data plane, not a failed run; another tool may ignore it).
pub fn write_line(out: &mut impl Write, line: &str) -> std::io::Result<()> {
    out.write_all(line.as_bytes())?;
    out.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use serde::ser::{Serialize, Serializer};
    use serde_json::json;

    use super::{Order, write_json_object, write_line};

    /// A stand-in for a tool's own value vocabulary, with a quote that forces
    /// escaping and a boolean that would change bytes if this crate rendered
    /// either itself. The core must pass both straight to `serde`.
    enum Probe {
        Str(&'static str),
        Num(i64),
        Flag(bool),
    }

    impl Serialize for Probe {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            match self {
                Probe::Str(s) => serializer.serialize_str(s),
                Probe::Num(n) => serializer.serialize_i64(*n),
                Probe::Flag(b) => serializer.serialize_bool(*b),
            }
        }
    }

    fn fields() -> Vec<(&'static str, Probe)> {
        vec![
            ("zulu", Probe::Flag(true)),
            ("alpha", Probe::Str("a \"quoted\" \\ path")),
            ("middle", Probe::Num(-1)),
        ]
    }

    #[test]
    fn event_first_keeps_declaration_order_and_the_callers_values() {
        let event = "summary".to_string();
        let line = serde_json::to_string(&EventFirstProbe(&event, fields())).expect("one object");
        assert_eq!(
            line,
            r#"{"event":"summary","zulu":true,"alpha":"a \"quoted\" \\ path","middle":-1}"#
        );
    }

    #[test]
    fn alphabetical_sorts_fields_and_places_event_among_them() {
        let event = "summary".to_string();
        let line = serde_json::to_string(&AlphabeticalProbe(&event, fields())).expect("one object");
        assert_eq!(
            line,
            r#"{"alpha":"a \"quoted\" \\ path","event":"summary","middle":-1,"zulu":true}"#
        );
        let parsed: serde_json::Value = serde_json::from_str(&line).expect("valid object");
        assert_eq!(
            parsed,
            json!({"event": "summary", "zulu": true,
                                 "alpha": "a \"quoted\" \\ path", "middle": -1})
        );
    }

    #[test]
    fn a_real_event_field_value_is_used_instead_of_a_derived_label() {
        // The event value is supplied by the caller in its own type; here a
        // boolean, to show the core never assumes it is a lowercased label.
        let line = serde_json::to_string(&BoolEventProbe).expect("one object");
        assert_eq!(line, r#"{"event":true,"only":1}"#);
    }

    #[test]
    fn write_line_appends_exactly_one_newline_and_propagates_errors() {
        let mut out = Vec::new();
        write_line(&mut out, "SUMMARY total=1").expect("write");
        assert_eq!(out, b"SUMMARY total=1\n");

        struct Fail;
        impl Write for Fail {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "closed",
                ))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert_eq!(
            write_line(&mut Fail, "x").expect_err("propagated").kind(),
            std::io::ErrorKind::BrokenPipe
        );
    }

    struct EventFirstProbe<'a>(&'a str, Vec<(&'static str, Probe)>);

    impl serde::Serialize for EventFirstProbe<'_> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            write_json_object(serializer, "event", &self.0, &self.1, Order::EventFirst)
        }
    }

    struct AlphabeticalProbe<'a>(&'a str, Vec<(&'static str, Probe)>);

    impl serde::Serialize for AlphabeticalProbe<'_> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            write_json_object(serializer, "event", &self.0, &self.1, Order::Alphabetical)
        }
    }

    struct BoolEventProbe;

    impl serde::Serialize for BoolEventProbe {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let fields = [("only", Probe::Num(1))];
            write_json_object(serializer, "event", &true, &fields, Order::EventFirst)
        }
    }
}
