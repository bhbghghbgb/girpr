use super::*;
use crate::encoding::percent_encode;
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
    let s = Record::summary("verify").set("artifact", r"E:\artifacts\hk4e_global\7.0.0__7.1.0\id");
    let line = s.to_text_line();
    let tail = line.rsplit_once("artifact=").unwrap().1;
    assert_eq!(
        percent_decode(tail),
        r"E:\artifacts\hk4e_global\7.0.0__7.1.0\id"
    );
}

/// The whole point of the text form: split it on whitespace and every token
/// after the first is a `k=v` pair — so a value carrying a space must be
/// quoted or escaped, never emitted as a bare word.
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

/// The scan is over bytes, so the two bytes after a `%` are read as text even
/// when they are the middle of a multi-byte character. There is no char
/// boundary there, and a slice of the string would panic.
#[test]
fn a_percent_before_a_multibyte_character_passes_through() {
    assert_eq!(percent_decode("%中"), "%中");
    assert_eq!(percent_decode("%zz中"), "%zz中");
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

/// A count too large for `i64` clamps, and does not wrap negative.
///
/// Counts and byte totals are `u64` while a field value is `i64`. An `as i64`
/// cast would turn the largest count into a negative one, so a summary would
/// report a run that downloaded fewer than zero bytes; the clamp is the whole
/// point of the conversion, so it is pinned rather than left to the comment
/// above it.
#[test]
fn a_count_too_large_for_i64_clamps_instead_of_wrapping_negative() {
    assert_eq!(Val::from(u64::MAX), Val::Int(i64::MAX));
    // On a 32-bit target `usize::MAX` fits in `i64` and must not be clamped.
    #[cfg(target_pointer_width = "64")]
    assert_eq!(Val::from(usize::MAX), Val::Int(i64::MAX));
    // The clamp must not touch a value that already fits.
    assert_eq!(Val::from(7_u64), Val::Int(7));
    assert_eq!(Val::from(7_usize), Val::Int(7));
}

#[test]
fn output_format_parses_both_spellings_and_rejects_others() {
    assert_eq!("text".parse(), Ok(OutputFormat::Text));
    assert_eq!("json".parse(), Ok(OutputFormat::Json));
    assert!("xml".parse::<OutputFormat>().is_err());
    assert_eq!(OutputFormat::default(), OutputFormat::Text);
}

/// A `CaptureSummary` with every number zero and every string empty.
///
/// The values do not matter here - this exists to make the *shape*
/// observable, so the key test below reads the same regardless of what a
/// run measured.
fn zeroed() -> CaptureSummary<'static> {
    CaptureSummary {
        result: "",
        biz: "",
        old_tag: "",
        new_tag: "",
        package_id: "",
        old_build_id: "",
        new_build_id: "",
        artifact: std::path::Path::new(""),
        api_documents: 0,
        selection_pool: "",
        selection: SelectionStats {
            candidates_scored: 0,
            captures_admitted: 0,
            captures_over_budget: 0,
            forced_files_requested: 0,
            forced_files_skipped: 0,
            forced_files_unscored: 0,
            capture_bytes_admitted: 0,
            budget_unlimited: false,
            budget_limit: 0,
            classes_satisfied: 0,
            classes_unmet: 0,
        },
        chunks: ChunkStats {
            wanted: 0,
            stored: 0,
            bytes_projected: 0,
            bytes_fetched: 0,
            fetches_failed: 0,
            a6_failing: 0,
            patch_fetch_chunks: 0,
        },
        assembly: AssemblyStats {
            patchmaps_written: 0,
            selfcheck_rows: 0,
            selfcheck_failures: 0,
            patch_reuse_chunks: 0,
            patch_fetch_chunks: 0,
            patch_moved_chunks: 0,
        },
        manifests: ManifestStats {
            captured: 0,
            bytes: 0,
            entries: 0,
            chunks: 0,
            max_chunk_compressed_bytes: 0,
        },
        diff: DiffStats {
            common: 0,
            identical: 0,
            changed: 0,
            added: 0,
            removed: 0,
            patch_bytes: 0,
        },
        reuse: ReuseStats {
            md5_present_in_old: 0,
            cross_file_only_chunks: 0,
            cross_file_bytes_forgone: 0,
        },
        http: HttpStats {
            documents_written: 0,
            requests: 0,
            bytes_downloaded: 0,
        },
        probe: ProbeStats {
            probed: 0,
            served: 0,
            served_outside_diff_tags: 0,
        },
        legacy_endpoint_majors_behind: 0,
        exit_code: 0,
    }
}

#[test]
fn the_capture_summary_key_list_is_the_contract_and_does_not_drift() {
    // `girpr` and anything else reading this line parse these names in this
    // order. Moving the builder out of `capture()` is only safe if the line
    // it produces matches the contract, so the key list is pinned here rather
    // than left to the builder's own doc comment.
    //
    // The first three are [`Record::summary`]'s own preamble, not this
    // function's - they are listed so that a change to *them* also fails
    // here, since a reader sees one line and not two sources.
    let got: Vec<&str> = capture_summary(&zeroed()).keys().collect();
    let expected: Vec<&str> = vec![
        "event",
        "tool",
        "version",
        "result",
        "stage",
        "biz",
        "old_tag",
        "new_tag",
        "package_id",
        "old_build_id",
        "new_build_id",
        "artifact",
        "api_documents",
        "selection_pool",
        "candidates_scored",
        "captures_admitted",
        "captures_over_budget",
        "forced_files_requested",
        "forced_files_skipped",
        "forced_files_unscored",
        "chunk_blobs_wanted",
        "chunk_blobs_stored",
        "chunk_bytes_projected",
        "chunk_bytes_fetched",
        "chunk_fetches_failed",
        "chunk_a6_failing",
        "patch_fetch_chunks",
        "patchmaps_written",
        "assembly_selfcheck_rows",
        "assembly_selfcheck_failures",
        "patch_reuse_chunks",
        "patch_fetch_chunks_actually",
        "patch_moved_chunks",
        "capture_bytes_admitted",
        "budget_unlimited",
        "budget_limit",
        "classes_satisfied",
        "classes_unmet",
        "manifests_captured",
        "manifest_bytes",
        "manifest_entries",
        "manifest_chunks",
        "max_chunk_compressed_bytes",
        "diff_common",
        "diff_identical",
        "diff_changed",
        "diff_added",
        "diff_removed",
        "diff_patch_bytes",
        "reuse_md5_present_in_old",
        "reuse_cross_file_only_chunks",
        "reuse_cross_file_bytes_forgone",
        "documents_written",
        "requests",
        "bytes_downloaded",
        "tags_probed",
        "tags_served",
        "tags_served_outside_diff_tags",
        "legacy_endpoint_majors_behind",
        "exit_code",
    ];
    if got != expected {
        // Report the first divergence rather than two 56-element dumps.
        let first = got
            .iter()
            .zip(expected.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(got.len().min(expected.len()));
        panic!(
            "capture summary key list changed at index {first}:\n  \
                 now:  {:?}\n  was:  {:?}\n  ({} keys now, {} before)",
            got.get(first),
            expected.get(first),
            got.len(),
            expected.len()
        );
    }
}

#[test]
fn the_stage_names_the_stage_not_a_milestone() {
    // `m8-artifact` told a reader about the order the crate was built in and
    // nothing about the run in front of them, so a milestone label must not
    // creep back in. It is pinned here because it is on the data plane: a
    // caller matching on `stage=artifact` has to break loudly if it changes.
    let r = capture_summary(&zeroed());
    assert_eq!(
        r.get("stage"),
        Some(&Val::from(STAGE_CAPTURE)),
        "stage must name the stage"
    );
    assert_eq!(STAGE_CAPTURE, "artifact");
    assert!(
        !STAGE_CAPTURE.starts_with('m') || !STAGE_CAPTURE.contains(|c: char| c.is_ascii_digit()),
        "a milestone label has crept back into the data plane: {STAGE_CAPTURE:?}"
    );
}

#[test]
fn each_summary_group_reports_under_its_own_name_and_not_its_struct_name() {
    // The trap in grouping the input: `assembly.selfcheck_rows` reads
    // naturally as `assembly_selfcheck_rows`, and it does - but
    // `chunks.patch_fetch_chunks` and `assembly.patch_fetch_chunks` are two
    // *different* numbers under two different names, because they were when
    // they were written by hand. Renaming either to make the grouping tidier
    // would silently repoint a caller at the other one.
    let r = capture_summary(&zeroed());
    assert_eq!(r.get("patch_fetch_chunks"), Some(&Val::Int(0)));
    assert_eq!(r.get("patch_fetch_chunks_actually"), Some(&Val::Int(0)));
    assert_eq!(r.get("assembly_selfcheck_rows"), Some(&Val::Int(0)));
    assert_eq!(r.get("assembly_selfcheck_failures"), Some(&Val::Int(0)));
    // And neither is the bare struct-field spelling.
    assert_eq!(r.get("patch_fetch"), None);
    assert_eq!(r.get("selfcheck_rows"), None);
}

#[test]
fn set_all_appends_in_order_rather_than_sorting() {
    // `set_all` takes an ordered iterable on purpose. Had it taken a map, a
    // `BTreeMap` would have silently rewritten the reader-visible order -
    // which is the one thing `set_all` exists to preserve.
    let r =
        Record::summary("x").set_all([("z", Val::Int(1)), ("a", Val::Int(2)), ("m", Val::Int(3))]);
    // `summary`'s own preamble comes first; the point is that `z, a, m`
    // survive in that order rather than being sorted to `a, m, z`.
    assert_eq!(
        r.keys().collect::<Vec<_>>(),
        vec!["event", "tool", "version", "z", "a", "m"]
    );
    assert!(r.to_text_line().ends_with("z=1 a=2 m=3"));
}

#[test]
fn set_all_appends_rather_than_replacing_like_set_does_not() {
    // Duplicate names are kept, exactly as `set` keeps them. If `set_all`
    // were implemented as a map insert, a summary would silently lose a
    // field instead of showing two facts under one name.
    let r = Record::summary("x").set_all([("n", Val::Int(1)), ("n", Val::Int(2))]);
    assert_eq!(
        r.keys().collect::<Vec<_>>(),
        vec!["event", "tool", "version", "n", "n"]
    );
    assert!(r.to_text_line().ends_with("n=1 n=2"));
}
