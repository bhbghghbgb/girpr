//! The ordered record, JSON and line-emission mechanics below the three report
//! planes.
//!
//! `girpr` writes an event-first object in declaration order, `girsync` writes
//! an alphabetically sorted object with `event` in its sorted place, and
//! `gircap` writes an event-first object whose `event` value is a real field.
//! The field orders, value vocabularies, escaping and boolean spellings are
//! deliberately different and stay in each tool. What is common is only the
//! *mechanic*: hold an ordered field list under a label, write it as an ordered
//! JSON object, mirror one rendered line to the log, and write that line plus
//! its newline.
//!
//! Nothing here names a field, a value, a flag or a program. Values arrive as
//! `Serialize` implementors owned by the caller, so this crate neither defines
//! nor normalises a value vocabulary: `serde` performs JSON escaping on the
//! caller's spelling, a tool that forbids booleans simply never passes one, and
//! a tool that renders `true` keeps rendering `true`. The text form's quoting or
//! percent-encoding is also the caller's — [`write_line`] writes an already
//! rendered string and never interprets it. The one value this crate does own
//! is [`OutputFormat`]: the two renderings' *name*, with one spelling and one
//! text rendering. Each plane keeps its own record fields, possible-value
//! descriptions and escaping.
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

/// How a report record is rendered on stdout. Selected by the global
/// `--output` flag.
///
/// The value set, its spelling and its text rendering are shared by the three
/// report planes; each plane keeps its own record layout, field order,
/// possible-value descriptions and escaping. The `ValueEnum` derive is opt-in
/// so a consumer that only renders records compiles no CLI framework.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "clap", value(rename_all = "kebab-case"))]
pub enum OutputFormat {
    /// One human-readable line per record. The default.
    #[default]
    Text,
    /// One JSON object per line (NDJSON), the same records in the same order.
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

/// One label and an ordered field list: the part of a report record that every
/// plane shares.
///
/// `V` is the plane's own value vocabulary. The core stores it and writes it
/// verbatim; it never inspects, folds or normalises a value, so girpr's `1`/`0`
/// flags, capture's native booleans and girsync's JSON values keep their own
/// spellings. The label is stored but never folded — each plane derives its
/// `event` value at its own call site — and the field order is exactly the
/// order values are pushed. A plane chooses its own [`Order`] when it writes
/// the object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record<V> {
    label: &'static str,
    fields: Vec<(&'static str, V)>,
}

impl<V> Record<V> {
    /// An empty record with `label`.
    pub fn new(label: &'static str) -> Self {
        Self {
            label,
            fields: Vec::new(),
        }
    }

    /// A record from a field list already in report order.
    pub fn with_fields(label: &'static str, fields: Vec<(&'static str, V)>) -> Self {
        Self { label, fields }
    }

    /// Append one field in report order.
    pub fn push(&mut self, key: &'static str, value: V) -> &mut Self {
        self.fields.push((key, value));
        self
    }

    /// Append one field in report order, returning the record for chaining.
    pub fn put(mut self, key: &'static str, value: V) -> Self {
        self.push(key, value);
        self
    }

    /// Append many fields at once, in the order given.
    pub fn extend<I>(&mut self, fields: I) -> &mut Self
    where
        I: IntoIterator<Item = (&'static str, V)>,
    {
        self.fields.extend(fields);
        self
    }

    /// The text label, e.g. `SUMMARY`. Never folded: that is a call-site
    /// decision, because the three planes derive `event` differently.
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// The fields in report order, without any `event` member.
    pub fn fields(&self) -> &[(&'static str, V)] {
        &self.fields
    }

    /// Write this record's field list as one JSON object through `serializer`.
    ///
    /// `event` supplies the named member's value; a field with the same key is
    /// skipped so that member is not written twice. Beyond that, values and
    /// their order are the caller's: they pass straight to [`write_json_object`],
    /// which applies `order` without changing any value.
    pub fn write_json<E, S>(
        &self,
        serializer: S,
        event_key: &str,
        event: &E,
        order: Order,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        E: Serialize,
        V: Serialize,
    {
        let fields: Vec<(&str, &V)> = self
            .fields
            .iter()
            .filter(|(key, _)| *key != event_key)
            .map(|(key, value)| (*key, value))
            .collect();
        write_json_object(serializer, event_key, event, &fields, order)
    }
}

/// Write one already-rendered line plus its newline.
///
/// The line is opaque: any quoting, percent-encoding or case is the caller's.
/// A closed or failing writer propagates its error so a caller can decide
/// whether it is fatal (the repair binary treats a broken pipe as the end of
/// its data plane, not a failed run; another tool may ignore it).
pub fn write_line(out: &mut impl Write, line: &str) -> std::io::Result<()> {
    // One preallocated buffer and one `write_all` per record: the emitted bytes
    // and the record's atomicity are unchanged (the writer was already locked
    // by the caller), but the writer is entered once instead of twice. Capacities
    // are exact so the single write cannot reallocate or split the record.
    let mut buf = Vec::with_capacity(line.len() + 1);
    buf.extend_from_slice(line.as_bytes());
    buf.push(b'\n');
    out.write_all(&buf)
}

/// Mirror a rendered line to the log, then write it to `out`.
///
/// This is the one "render → log → write one line" step shared by the three
/// report planes. The line arrives already rendered, so text/JSON shapes,
/// quoting and field order stay at the call site; `log_hook` runs *before* the
/// write so a consumer that closed stdout cannot cost the record its log copy.
///
/// The write error is returned rather than handled here. Stdout locking and
/// the broken-pipe policy are each program's published contract — one prints
/// through `println!` and keeps its panic, one downgrades a closed stdout to a
/// debug line, one writes under a lock and ignores the result — so the caller
/// passes its own writer and decides what an error means. Nothing here locks
/// stdout, derives an event name or interprets the line.
pub fn write_record_line<W: Write>(
    out: &mut W,
    line: &str,
    log_hook: impl FnOnce(&str),
) -> std::io::Result<()> {
    log_hook(line);
    write_line(out, line)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use serde::ser::{Serialize, Serializer};
    use serde_json::json;

    use super::{Order, OutputFormat, Record, write_json_object, write_line, write_record_line};

    /// A stand-in for a tool's own value vocabulary, with a quote that forces
    /// escaping and a boolean that would change bytes if this crate rendered
    /// either itself. The core must pass both straight to `serde`.
    #[derive(Clone)]
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

    /// The four report-core contracts, folded into one process so nextest spawns
    /// one test instead of four. Every original assertion and message is kept;
    /// a labelled subcase names the contract that failed.
    #[test]
    fn write_json_object_and_write_line_contracts() {
        let subcases: &[(&str, &dyn Fn())] = &[
            (
                "event_first_keeps_declaration_order_and_the_callers_values",
                &|| {
                    let event = "summary".to_string();
                    let line = serde_json::to_string(&EventFirstProbe(&event, fields()))
                        .expect("one object");
                    assert_eq!(
                        line,
                        r#"{"event":"summary","zulu":true,"alpha":"a \"quoted\" \\ path","middle":-1}"#,
                        "event_first_keeps_declaration_order_and_the_callers_values"
                    );
                },
            ),
            (
                "alphabetical_sorts_fields_and_places_event_among_them",
                &|| {
                    let event = "summary".to_string();
                    let line = serde_json::to_string(&AlphabeticalProbe(&event, fields()))
                        .expect("one object");
                    assert_eq!(
                        line,
                        r#"{"alpha":"a \"quoted\" \\ path","event":"summary","middle":-1,"zulu":true}"#,
                        "alphabetical_sorts_fields_and_places_event_among_them"
                    );
                    let parsed: serde_json::Value =
                        serde_json::from_str(&line).expect("valid object");
                    assert_eq!(
                        parsed,
                        json!({"event": "summary", "zulu": true,
                                             "alpha": "a \"quoted\" \\ path", "middle": -1}),
                        "alphabetical_sorts_fields_and_places_event_among_them"
                    );
                },
            ),
            (
                "a_real_event_field_value_is_used_instead_of_a_derived_label",
                &|| {
                    // The event value is supplied by the caller in its own type;
                    // here a boolean, to show the core never assumes it is a
                    // lowercased label.
                    let line = serde_json::to_string(&BoolEventProbe).expect("one object");
                    assert_eq!(
                        line, r#"{"event":true,"only":1}"#,
                        "a_real_event_field_value_is_used_instead_of_a_derived_label"
                    );
                },
            ),
            (
                "write_line_appends_exactly_one_newline_and_propagates_errors",
                &|| {
                    let mut out = Vec::new();
                    write_line(&mut out, "SUMMARY total=1").expect("write");
                    assert_eq!(
                        out, b"SUMMARY total=1\n",
                        "write_line appends exactly one newline"
                    );

                    assert_eq!(
                        write_line(&mut Fail, "x").expect_err("propagated").kind(),
                        std::io::ErrorKind::BrokenPipe,
                        "write_line propagates the writer error"
                    );
                },
            ),
        ];
        for (label, run) in subcases {
            println!("subcase: {label}");
            run();
        }
    }

    /// The one value this crate owns: two spellings, one text rendering, and
    /// a hand parse whose wording is the same one the capture tool published.
    #[test]
    fn output_format_value_set_and_renderings() {
        assert_eq!(OutputFormat::default(), OutputFormat::Text);
        assert_eq!(OutputFormat::Text.as_str(), "text");
        assert_eq!(OutputFormat::Json.as_str(), "json");
        assert_eq!(OutputFormat::Text.to_string(), "text");
        assert_eq!(OutputFormat::Json.to_string(), "json");
        assert_eq!("text".parse::<OutputFormat>(), Ok(OutputFormat::Text));
        assert_eq!("json".parse::<OutputFormat>(), Ok(OutputFormat::Json));
        assert_eq!(
            "xml".parse::<OutputFormat>().expect_err("unknown format"),
            "unknown output format \"xml\"; expected text or json"
        );
    }

    /// The field list is held once, in push order, and written as one object in
    /// whichever order the plane asked for. A field named like the event member
    /// is skipped so it cannot be written twice — capture relies on this for
    /// its real `event` field, while girpr/girsync never have one.
    #[test]
    fn record_holds_push_order_and_writes_one_object() {
        let subcases: &[(&str, &dyn Fn())] = &[
            ("push_order_and_accessors", &|| {
                let mut rec = Record::new("SUMMARY");
                rec.push("renamed", Probe::Num(2))
                    .push("delta", Probe::Num(-1));
                rec.extend([("total", Probe::Num(1))]);
                assert_eq!(rec.label(), "SUMMARY");
                assert_eq!(
                    rec.fields().iter().map(|(k, _)| *k).collect::<Vec<_>>(),
                    vec!["renamed", "delta", "total"]
                );
                assert_eq!(rec.clone().put("last", Probe::Flag(true)).fields().len(), 4);
            }),
            ("event_first_keeps_the_callers_order", &|| {
                let rec = Record::new("SUMMARY")
                    .put("zulu", Probe::Flag(true))
                    .put("alpha", Probe::Str("a \"quoted\" path"));
                let line = serde_json::to_string(&RecordProbe(&rec, "summary", Order::EventFirst))
                    .expect("one object");
                assert_eq!(
                    line,
                    r#"{"event":"summary","zulu":true,"alpha":"a \"quoted\" path"}"#
                );
            }),
            ("alphabetical_places_event_among_the_fields", &|| {
                let rec = Record::new("SUMMARY")
                    .put("zulu", Probe::Flag(true))
                    .put("alpha", Probe::Str("plain"));
                let line =
                    serde_json::to_string(&RecordProbe(&rec, "summary", Order::Alphabetical))
                        .expect("one object");
                assert_eq!(line, r#"{"alpha":"plain","event":"summary","zulu":true}"#);
            }),
            ("a_real_event_field_value_replaces_the_derived_one", &|| {
                let rec = Record::with_fields(
                    "SUMMARY",
                    vec![
                        ("event", Probe::Str("capture")),
                        ("tool", Probe::Str("gircap")),
                    ],
                );
                let line = serde_json::to_string(&RecordProbe(&rec, "capture", Order::EventFirst))
                    .expect("one object");
                assert_eq!(line, r#"{"event":"capture","tool":"gircap"}"#);
            }),
        ];
        for (label, run) in subcases {
            println!("subcase: {label}");
            run();
        }
    }

    /// The sink logs before it writes, writes exactly one newline, and leaves
    /// the broken-pipe decision to the caller by returning the error.
    #[test]
    fn write_record_line_logs_then_writes_and_propagates_errors() {
        let events = std::cell::RefCell::new(Vec::new());
        let mut out = Vec::new();
        write_record_line(&mut out, "SUMMARY total=1", |line| {
            events.borrow_mut().push(line.to_string());
        })
        .expect("write");
        assert_eq!(out, b"SUMMARY total=1\n");
        assert_eq!(*events.borrow(), vec!["SUMMARY total=1".to_string()]);

        let err = write_record_line(&mut Fail, "x", |_| {}).expect_err("propagated");
        assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
    }

    /// A caller-supplied `event` value and order, as a `Serialize` adapter for
    /// a core record — exactly how a report plane renders one.
    struct RecordProbe<'a, V>(&'a Record<V>, &'a str, Order);

    impl<V: Serialize> serde::Serialize for RecordProbe<'_, V> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            self.0.write_json(serializer, "event", &self.1, self.2)
        }
    }

    /// A writer whose reader has gone away, for the broken-pipe assertion.
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
