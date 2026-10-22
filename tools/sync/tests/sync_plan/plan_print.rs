//! The plan print: the diff, reported before anything happens.
//!
//! The first of the two segments a consumer reads. It exists so the plan is
//! independently parseable, which is why a `MISSING` path appears here *and* as a
//! `COPY` among the actions.

use super::*;

/// **The sync report is two segments: the plan, then the actions.** A consumer that
/// wants to know what the tool *decided* reads the first; one that wants to know what
/// it *did* reads the second. They are one stream because a plan you cannot correlate
/// with the actions is not a plan.
#[test]
fn sync_reports_the_diff_as_its_plan_before_it_acts() {
    let t = temp_root("plan_print");
    let (src, dst) = build(&t);
    let recs = run(&src, &dst, &["--dry-run"]);

    let events: Vec<&str> = recs.iter().map(|r| r["event"].as_str().unwrap()).collect();
    for expected in ["missing", "extra", "changed"] {
        assert!(
            events.contains(&expected),
            "the plan print must include `{expected}` records, as `compare` does: {events:?}"
        );
    }
    // The plan segment comes before the actions: every plan record precedes every
    // apply action, so a `MISSING` precedes the `COPY` it causes.
    let first_action = events
        .iter()
        .position(|e| is_action(e))
        .expect("the fixture has actions");
    for (i, e) in events.iter().enumerate() {
        if is_action(e) || *e == "summary" {
            continue;
        }
        assert!(
            is_plan(e) || is_rename(e),
            "record {i} is `{e}`, which is in no known segment"
        );
        if !is_rename(e) {
            assert!(
                i < first_action,
                "record {i} is `{e}`, a plan record, but follows an action"
            );
        }
    }
    assert_eq!(
        recs.last().map(|r| r["event"].as_str().unwrap()),
        Some("summary"),
        "and the summary is still last"
    );
}

/// **`sync` still exits 0 with a non-empty plan.** The single most damaging thing
/// this command could do by accident: printing a plan makes `sync` *look* like
/// `compare`, and `compare` exits 4 on a difference. Returning 4 here would break
/// every script that syncs a partly-different tree, and it is not what is asked
/// for.
///
/// Asserted against a plan that is genuinely non-empty, so it cannot pass vacuously.
#[test]
fn sync_still_exits_zero_with_a_non_empty_plan() {
    let t = temp_root("plan_exit");
    let (src, dst) = build(&t);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .arg("sync")
        .arg("--src")
        .arg(&src)
        .arg("--dst")
        .arg(&dst)
        .arg("--dry-run")
        .arg("--output")
        .arg("json")
        .output()
        .expect("spawn girsync");
    assert_eq!(
        out.status.code(),
        Some(0),
        "a non-empty plan must not change sync's exit code: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let recs = parse_ndjson(&out.stdout);
    let s = summary_of(&recs);
    assert!(
        s["total_diff"].as_u64().unwrap_or(0) > 0 || s["renamed"].as_u64().unwrap_or(0) > 0,
        "the fixture must differ, or this proves nothing: {recs:?}"
    );
}

/// **A `MISSING` file produces both a plan line and a `COPY` line.** Intentional, and
/// the reason the plan is independently parseable: a consumer reading only the plan
/// segment can enumerate what *will* happen without joining it against the actions.
#[test]
fn a_missing_file_appears_in_both_the_plan_and_the_actions() {
    let t = temp_root("plan_missing");
    let (src, dst) = build(&t);
    let recs = run(&src, &dst, &["--dry-run"]);
    // Only the missing **files**: a missing directory is `MISSING` in the plan and
    // `MKDIR` among the actions, which is the same doubling under a different verb.
    // Asserting the file case specifically is what keeps this about `COPY`.
    let missing_files: Vec<&str> = recs
        .iter()
        .filter(|r| r["event"] == "missing")
        .filter(|r| {
            !recs
                .iter()
                .any(|o| o["event"] == "mkdir" && o["path"] == r["path"])
        })
        .filter_map(|r| r["path"].as_str())
        .collect();
    assert!(
        !missing_files.is_empty(),
        "the fixture must have a missing file: {recs:?}"
    );
    for path in missing_files {
        assert!(
            recs.iter()
                .any(|r| r["event"] == "copy" && r["path"] == path),
            "{path} is planned as MISSING and must also be acted on: {recs:?}"
        );
    }
    // And the directory case, so the exclusion above is a stated rule rather than a
    // gap: a missing directory doubles as `MKDIR` rather than `COPY`.
    assert!(
        recs.iter()
            .any(|r| r["event"] == "missing" && r["path"] == "newdir")
            && recs
                .iter()
                .any(|r| r["event"] == "mkdir" && r["path"] == "newdir"),
        "a missing directory is planned as MISSING and acted on as MKDIR: {recs:?}"
    );
}

/// **`sync`'s plan carries reasons when asked, and only then.** The reasons appear on
/// the *plan* line, not the action line: the plan says why the tool decided, the
/// action says what it is doing, and those are different questions. That is why
/// `COPY` never carries a `why=`.
#[test]
fn the_plan_carries_reasons_when_why_is_passed() {
    let t = temp_root("plan_why");
    let (src, dst) = build(&t);

    let flagged = run(&src, &dst, &["--dry-run", "--why"]);
    let changed: Vec<&Value> = flagged.iter().filter(|r| r["event"] == "changed").collect();
    assert!(
        !changed.is_empty(),
        "the fixture must have a changed file: {flagged:?}"
    );
    for r in &changed {
        assert!(
            r["why"].is_string(),
            "a CHANGED plan record names why it changed: {r}"
        );
    }
    assert!(
        !flagged
            .iter()
            .any(|r| r["event"] == "copy" && r.get("why").is_some()),
        "and the COPY line never repeats it — the action line answers 'what', the \
         plan line answers 'why': {flagged:?}"
    );

    let unflagged = run(&src, &dst, &["--dry-run"]);
    assert!(
        !unflagged.iter().any(|r| r.get("why").is_some()),
        "and `--why` is still default off: {unflagged:?}"
    );
}

/// **`COPY` says which bucket caused it, and never repeats why that bucket was
/// reached.** The action line answers *"what is this tool doing"*; the plan line
/// answers *"why did it decide that"*. A `COPY ... why=` would put the same fact in
/// both places and force a consumer reading only the actions to parse a tag whose
/// vocabulary is the diff's.
///
/// `because` is coarser than `why` on purpose: `because=changed` is the whole truth
/// about the action, and the reason for the change belongs to the `CHANGED` plan line
/// for the same path.
#[test]
fn copy_names_the_bucket_that_caused_it_and_never_a_why() {
    let t = temp_root("because");
    let (src, dst) = build(&t);
    let recs = run(&src, &dst, &["--dry-run", "--why"]);

    let copies: Vec<&Value> = recs.iter().filter(|r| r["event"] == "copy").collect();
    assert!(
        !copies.is_empty(),
        "the fixture must copy something: {recs:?}"
    );
    for r in &copies {
        assert!(
            r.get("why").is_none(),
            "an action line must not repeat a reason: {r}"
        );
        assert!(
            r["because"].is_string(),
            "and must name the bucket that caused it: {r}"
        );
        assert!(
            matches!(
                r["because"].as_str().unwrap(),
                "missing" | "changed" | "type-conflict"
            ),
            "and `because` is one of the three buckets: {r}"
        );
    }
}

/// **Every `because` value is true of that path's plan record.** The one-to-one join:
/// a `COPY ... because=missing` must have a `MISSING` plan line for the same path,
/// and the bucket must be the one that actually applied. Asserted as a table
/// keyed by path rather than as a set, so a copy whose cause is merely *a* valid
/// bucket — but the wrong one — fails.
#[test]
fn each_copy_because_matches_its_own_plan_record() {
    let t = temp_root("because_join");
    let (src, dst) = build(&t);
    let recs = run(&src, &dst, &["--dry-run"]);

    // The bucket each `COPY` must name, derived from the path's plan record rather
    // than written out — so a copy and a plan record cannot disagree here.
    let bucket_of = |path: &str| -> String {
        let event = recs
            .iter()
            .find(|r| {
                matches!(
                    r["event"].as_str(),
                    Some("missing" | "changed" | "type-conflict")
                ) && r["path"] == path
            })
            .map(|r| r["event"].as_str().unwrap())
            .unwrap_or_else(|| panic!("{path} is copied but has no plan record: {recs:?}"));
        event.to_string()
    };
    for r in recs.iter().filter(|r| r["event"] == "copy") {
        let path = r["path"].as_str().unwrap();
        assert_eq!(
            r["because"].as_str().unwrap(),
            bucket_of(path),
            "{path}: the action must name the bucket its own plan record is in"
        );
    }
}

/// **A type conflict names `type-conflict`, not `missing` or `changed`.** The three
/// buckets are the ones `build_plan` merges into one flat copy list, and they are
/// exactly the three ways a path can be both present and wrong. The third is the one
/// a table derived from "present on one side" would get wrong.
#[test]
fn a_copy_over_a_blocking_directory_names_the_type_conflict() {
    let t = temp_root("because_conflict");
    let (src, dst) = build(&t);
    let recs = run(&src, &dst, &["--dry-run"]);
    let conflict_copy = recs
        .iter()
        .find(|r| r["event"] == "copy" && r["path"] == "conflict.txt")
        .expect("conflict.txt is a file over a dst directory, so it is copied");
    assert_eq!(
        conflict_copy["because"], "type-conflict",
        "src has the file and dst has the directory: {recs:?}"
    );
    assert!(
        recs.iter()
            .any(|r| r["event"] == "type-conflict" && r["path"] == "conflict.txt"),
        "and the plan says the same: {recs:?}"
    );
}

/// **`--missing-only` suppresses changed copies, and the surviving ones keep their
/// bucket.** The flag removes paths from the copy list; it must not relabel the ones
/// that remain, or `because` would describe a plan the run is not following.
#[test]
fn missing_only_keeps_the_bucket_of_the_copies_it_leaves() {
    let t = temp_root("because_missing_only");
    let (src, dst) = build(&t);
    let recs = run(&src, &dst, &["--dry-run", "--missing-only"]);

    for r in recs.iter().filter(|r| r["event"] == "copy") {
        assert_eq!(
            r["because"], "missing",
            "--missing-only copies only what is absent: {recs:?}"
        );
        assert!(
            !recs
                .iter()
                .any(|p| p["event"] == "copy" && p["path"] == "differs.txt"),
            "so a changed path is not copied at all: {recs:?}"
        );
    }
}

/// **One `COPY` per path, ever.** This is the invariant that lets `build_plan`
/// deduplicate on the path alone: `missing`, `changed` and `type_conflict` are
/// disjoint buckets, so a path can never reach the copy list twice carrying two
/// different causes.
///
/// Asserted rather than assumed, because it is the thing that makes the dedup key
/// safe — and a dedup keyed on the whole item instead of the path would behave
/// identically *today* and start keeping duplicates the day two buckets could
/// overlap. Nothing else in the suite would notice: the count would still be right
/// until it was not, and by then a file would be copied twice.
#[test]
fn a_path_is_never_planned_for_copying_twice() {
    let t = temp_root("copy_once");
    let (src, dst) = build(&t);
    // `run` already passes `--output json`, so these are the extra flags only.
    for extra in [vec!["--dry-run"], vec!["--dry-run", "--case-sensitive"]] {
        let recs = run(&src, &dst, &extra);
        let copies: Vec<&str> = recs
            .iter()
            .filter(|r| r["event"] == "copy")
            .filter_map(|r| r["path"].as_str())
            .collect();
        let unique: BTreeSet<&&str> = copies.iter().collect();
        assert_eq!(
            copies.len(),
            unique.len(),
            "{extra:?}: the same path is copied more than once: {copies:?}"
        );
        assert!(
            !copies.is_empty(),
            "{extra:?}: the fixture must copy something"
        );
    }
}
