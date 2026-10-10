//! The `sync` plan, asserted as *what happens to each path*.
//!
//! ## Why there is no golden here
//!
//! A whole-run golden is a transcript: one hardcoded vector for everything the run
//! reported. It works while an eager implementation exists to compare against, and
//! stops working the moment it does not, because then the only way to "update" it
//! is to paste the actual output — which requires no understanding of whether the
//! new output is right. From that point it is a change detector wearing the costume
//! of a specification, and every future behaviour lands as a diff against a wall of
//! JSON rather than as a statement about intent.
//!
//! So the expectation here is a **table keyed by path**. Each row says what a path
//! is on each side and what the plan must therefore do about it. That is a claim
//! about intent rather than a transcript of a run: it stays true however the planner
//! is implemented, it fails with a pointed message naming the path, and adding a
//! path is adding a row rather than editing a vector.
//!
//! Three consequences worth stating, because they are what make the table load-bearing
//! rather than decorative:
//!
//! - **The summary is derived, not transcribed.** Its counts are tallied from the
//!   plan records, so it can never disagree with the plan, and adding a row cannot
//!   desynchronise a number.
//! - **The `--case-sensitive` expectation is derived too**, by a rule applied to the
//!   same table, because that mode is the same fixture with the rename pass absent.
//!   One table, two expectations, and the difference between them *is* the rename pass.
//! - **Order is asserted as a property**, not as a transcript: event kinds appear in
//!   apply order and each kind's records are contiguous. Adding a record kind makes
//!   that check fail until it says where the kind applies — which is the question a
//!   golden cannot ask.
//!
//! ## What this file does not do
//!
//! It does not compare a dry run against a real run. That is a differential between
//! two runs of the same code differing only in the write gate, which is meaningful
//! without an eager reference — it catches the dry run taking a different branch,
//! which is the whole `--dry-run` contract. It lives here too, at the end, sharing
//! this fixture.

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use common::{TempRoot, parse_ndjson, summary_of, sync_mtime, wfile};
use serde_json::Value;

/// Run `girsync sync --output json` and return the records it reported.
///
/// The real binary rather than the library, so the `rename` record is covered: it
/// comes from the rename pass, not the plan builder, and the rename pass is the part
/// of `sync` most exposed to the planner running before it.
fn run(src: &Path, dst: &Path, extra: &[&str]) -> Vec<Value> {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .arg("sync")
        .arg("--src")
        .arg(src)
        .arg("--dst")
        .arg(dst)
        .arg("--output")
        .arg("json")
        .args(extra)
        .output()
        .expect("spawn girsync");
    assert!(
        out.status.success(),
        "sync failed: {}\n{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    parse_ndjson(&out.stdout)
}

/// What a path looks like on one side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// A file with these bytes.
    File(&'static [u8]),
    /// An empty directory.
    Dir,
    /// Not present on this side.
    Absent,
}

/// One path of the fixture, and what the plan must do about it.
///
/// `expect` is the whole claim: records the plan must report, as `(event, subject)`.
/// The subject is the record's `path`, or its `from` for a `rename` — the `to` is
/// implied by the fixture, since it is always src's spelling.
///
/// **The plan segment and the action segment are both here.** `sync` prints its diff
/// up-front and its actions after, and a row states what the tool must say about the
/// path across *both*: `same.txt` says nothing at all, `onlysrc.txt` is a `MISSING`
/// in the plan and a `COPY` among the actions, `notondst.txt` is an `EXTRA` and a
/// `DELETE`. That doubling is the point of the plan print — it is what makes the
/// plan independently parseable — so a row that listed only the action would be
/// asserting half of what the tool says about the path.
struct Case {
    /// The rel path on src, and the destination spelling for a case-only pair.
    src_rel: &'static str,
    /// The rel path on dst, where it is spelled differently.
    dst_rel: Option<&'static str>,
    src: Shape,
    dst: Shape,
    /// What the **apply phases** must do, as `(event, subject)`.
    expect: &'static [(&'static str, &'static str)],
}

impl Case {
    /// What the **plan print** must say about this path: the diff records.
    ///
    /// **Derived from the fixture's `Shape`s rather than written beside `expect`.**
    /// The two would otherwise be two hand-maintained claims about the same path, and
    /// a row where they disagree would be a row whose two halves no one checks
    /// against each other. Deriving it means the plan print is pinned by the *same*
    /// statement that pins the actions — which is the point, since the two segments
    /// are supposed to agree.
    fn plan(&self) -> Vec<(&'static str, String)> {
        let rel = self.subject();
        match (&self.src, &self.dst) {
            (Shape::Absent, Shape::Absent) => vec![],
            (Shape::Absent, _) => vec![("extra", rel)],
            (_, Shape::Absent) => vec![("missing", rel)],
            // Kinds differ: a file where the other side has a directory.
            (Shape::Dir, Shape::File(_)) | (Shape::File(_), Shape::Dir) => {
                vec![("type-conflict", rel)]
            }
            // Both files: equal content is not a difference, which is why
            // `same.txt` and the equal-content case-only row contribute nothing.
            (Shape::File(a), Shape::File(b)) if a == b => vec![],
            (Shape::File(_), Shape::File(_)) => vec![("changed", rel)],
            // Two directories: compared by presence, and both sides have one.
            (Shape::Dir, Shape::Dir) => vec![],
        }
    }

    /// The path a record about this row names.
    fn subject(&self) -> String {
        match self.src {
            Shape::Absent => self.dst_rel.unwrap_or(self.src_rel).to_string(),
            _ => self.src_rel.to_string(),
        }
    }
}

impl Case {
    /// True when both sides hold a file under different spellings — the case the
    /// rename pass exists to settle, and the only case where `--case-sensitive`
    /// changes the plan.
    fn is_case_only(&self) -> bool {
        self.dst_rel.is_some()
            && matches!(self.src, Shape::File(_))
            && matches!(self.dst, Shape::File(_))
    }
}

/// The order the apply phases run in, which is the order a dry run reports in:
/// renames first (they are decided before the diff that consumes them), then the
/// plan categories, and the summary last because it counts them.
///
/// This is a *property* of the plan, not a transcript of it. Asserting that event
/// kinds appear in this order and that each kind's records are contiguous is one
/// check, and it is the check that asks a new record kind to declare its place.
/// The order the apply phases run in, which is the order a dry run reports in.
///
/// The apply phases run in that order. `rename` is **not** among them: it is a
/// separate pass that must finish before the diff can be taken at all (the diff
/// compares exact keys), so it reports before the plan rather than within it.
const APPLY_ORDER: &[&str] = &["mkdir", "fix-dir", "delete", "copy", "rmdir"];

/// Which segment a record belongs to.
///
/// The stream is three segments plus a summary, in this order:
///
/// 1. **rename** — dst's casing aligned to src's. This happens *before* the diff and
///    cannot happen after it: the diff is taken on exact keys, which only exist once
///    the two spellings have been collapsed. It is a precondition of the plan rather
///    than a decision about it, which is why it is its own segment and not part of
///    either;
/// 2. **plan** — the diff, as `compare` prints it;
/// 3. **actions** — what the apply phases do, in apply order.
///
/// Kept apart because they answer different questions, and a check that ignored the
/// distinction would pass for a run that printed its plan and never acted, or acted
/// without ever saying what it was doing.
fn is_rename(event: &str) -> bool {
    event == "rename"
}

/// The apply phases, in the order they run. See [`is_rename`] for why `rename` is
/// not among them.
fn is_action(event: &str) -> bool {
    matches!(event, "mkdir" | "fix-dir" | "delete" | "copy" | "rmdir")
}

/// One of `sync`'s diff events.
fn is_plan(event: &str) -> bool {
    matches!(
        event,
        "missing" | "extra" | "changed" | "type-conflict" | "case-mismatch" | "identical"
    )
}

/// Every `(event, subject)` the run reported in one segment, sorted.
fn segment_pairs(recs: &[Value], in_segment: fn(&str) -> bool) -> BTreeSet<(String, String)> {
    recs.iter()
        .filter_map(|r| {
            let event = r["event"].as_str()?.to_string();
            if event == "summary" || !in_segment(&event) {
                return None;
            }
            // A rename names two paths; it is keyed on the one it renames *from*,
            // because that is the path dst actually holds.
            let subject = match event.as_str() {
                "rename" => r["from"].as_str()?,
                _ => r["path"].as_str()?,
            };
            Some((event, subject.to_string()))
        })
        .collect()
}

/// Every `(event, subject)` the run reported in its **action** segment.
fn reported_pairs(recs: &[Value]) -> BTreeSet<(String, String)> {
    segment_pairs(recs, is_action)
}

/// Every `(event, subject)` the table requires for the **apply phases**, sorted.
///
/// The plan segment is required separately by [`assert_plan_matches`]; the two are
/// separate checks because they are separate claims, and a set union would hide a
/// row whose plan and actions disagreed.
fn required_pairs(cases: &[Case]) -> BTreeSet<(String, String)> {
    cases
        .iter()
        .flat_map(|c| c.expect.iter())
        .map(|(e, s)| (e.to_string(), s.to_string()))
        .collect()
}

/// Every `(event, subject)` the table requires from the **rename pass**, sorted.
///
/// Its own set so that a `RENAME` appearing where the plan should be — or a plan
/// record appearing where a rename should be — is a mismatch rather than a union
/// that happens to contain both.
fn required_renames(cases: &[Case]) -> BTreeSet<(String, String)> {
    cases
        .iter()
        .flat_map(|c| c.expect.iter())
        .filter(|(e, _)| is_rename(e))
        .map(|(e, s)| (e.to_string(), s.to_string()))
        .collect()
}

/// Every `(event, subject)` the table requires for the **plan print**, sorted.
fn required_plan_pairs(cases: &[Case]) -> BTreeSet<(String, String)> {
    cases
        .iter()
        .flat_map(|c| c.plan())
        .map(|(e, s)| (e.to_string(), s))
        .collect()
}

/// Every `(event, subject)` actually reported in the **plan segment**, sorted.
///
/// Read by name rather than by position: the segment is defined by what a plan
/// record *is*, and locating it by "everything before the first action" would make
/// this check depend on the ordering `assert_apply_order` is there to verify.
fn reported_plan_pairs(recs: &[Value]) -> BTreeSet<(String, String)> {
    segment_pairs(recs, is_plan)
}

/// The expectation for `--case-sensitive`, derived from the same rows.
///
/// There is no rename pass in that mode, so a case-only pair is no longer the same
/// path: it becomes two unrelated paths — src's is `MISSING` and copied, dst's is
/// `EXTRA` and deleted. Everything else about the plan is unchanged, and every other
/// row is taken as written. Stating this as a rule rather than as a second table is
/// the point — the difference between the two modes *is* the rename pass, so it
/// should read as one rule and not as a parallel transcript that has to be kept in
/// sync.
fn sensitive_expect(case: &Case) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = case
        .expect
        .iter()
        .filter(|(e, _)| *e != "rename")
        .map(|(e, s)| (*e, s.to_string()))
        .collect();
    if case.is_case_only() {
        out.push(("copy", case.src_rel.to_string()));
        out.push(("delete", case.dst_rel.unwrap().to_string()));
    }
    out
}

/// The plan records for `--case-sensitive`, derived by the same rule.
///
/// The only rows that differ are the case-only ones, and they differ in both
/// segments: without the rename pass the two spellings are unrelated paths, so the
/// pair is no longer "compared and found equal" but "one missing, one extra". Every
/// other row's plan is unchanged.
fn sensitive_plan(case: &Case) -> Vec<(&'static str, String)> {
    if !case.is_case_only() {
        return case.plan();
    }
    vec![
        ("missing", case.src_rel.to_string()),
        ("extra", case.dst_rel.unwrap().to_string()),
    ]
}

/// The fixture: one row per path, each stating what the plan must do about it.
///
/// Every event kind is represented, so a record kind cannot quietly stop being
/// exercised — asserted as well as arranged, by
/// `the_fixture_covers_every_apply_phase`.
fn cases() -> Vec<Case> {
    use Shape::*;
    vec![
        // Equal on both sides: the plan must say nothing about it.
        Case {
            src_rel: "same.txt",
            dst_rel: None,
            src: File(b"identical"),
            dst: File(b"identical"),
            expect: &[],
        },
        // Different sizes, so stat settles it and no digest is needed. The pair a
        // lazy scan must stop reading — and the plan must not notice.
        Case {
            src_rel: "differs.txt",
            dst_rel: None,
            src: File(b"src-content-that-is-longer"),
            dst: File(b"dst"),
            expect: &[("copy", "differs.txt")],
        },
        // Casing only, same bytes: settled by renaming, with nothing to copy.
        Case {
            src_rel: "Data.txt",
            dst_rel: Some("data.txt"),
            src: File(b"payload"),
            dst: File(b"payload"),
            expect: &[("rename", "data.txt")],
        },
        // Casing *and* content, at the same size so stat cannot settle it.
        //
        // The row that guards the planner's case flag, and the reason this file
        // exists rather than a golden. `plan_pairs` runs before the rename pass,
        // so it has to pair by lowercase here; pairing by exact key makes this pair
        // look one-sided, skips the digest it needs, and the rename then collapses
        // the keys with `hashes_differ` silent on dst's absent digest — a bare
        // rename, nothing copied, dst left holding the wrong bytes. Every other
        // case-only row in the suite uses identical bytes, which is exactly why
        // that mistake passes all of them.
        Case {
            src_rel: "Case.txt",
            dst_rel: Some("case.txt"),
            src: File(b"payload"),
            dst: File(b"PAYLOAD"),
            expect: &[("rename", "case.txt"), ("copy", "Case.txt")],
        },
        // One-sided, both directions.
        Case {
            src_rel: "onlysrc.txt",
            dst_rel: None,
            src: File(b"only here"),
            dst: Absent,
            expect: &[("copy", "onlysrc.txt")],
        },
        Case {
            src_rel: "notondst.txt",
            dst_rel: Some("onlydst.txt"),
            src: Absent,
            dst: File(b"only there"),
            expect: &[("delete", "onlydst.txt")],
        },
        // A dst-only file under a dst-only directory tree: the file is deleted and
        // then the directories it emptied are removed, deepest first.
        Case {
            src_rel: "extradir/nested/deep.bin",
            dst_rel: None,
            src: Absent,
            dst: File(b"x"),
            expect: &[("delete", "extradir/nested/deep.bin")],
        },
        Case {
            src_rel: "extradir/nested",
            dst_rel: None,
            src: Absent,
            dst: Dir,
            expect: &[("rmdir", "extradir/nested")],
        },
        Case {
            src_rel: "extradir",
            dst_rel: None,
            src: Absent,
            dst: Dir,
            expect: &[("rmdir", "extradir")],
        },
        // A directory on src standing where dst has a file: resolved toward src, so
        // the file is replaced by a directory.
        Case {
            src_rel: "fixdir",
            dst_rel: None,
            src: Dir,
            dst: File(b"a blocking file"),
            expect: &[("fix-dir", "fixdir")],
        },
        Case {
            src_rel: "fixdir/inner.txt",
            dst_rel: None,
            src: File(b"f"),
            dst: Absent,
            expect: &[("copy", "fixdir/inner.txt")],
        },
        // The same conflict resolved the other way: src has the file, so dst's
        // directory is simply overwritten by the copy.
        Case {
            src_rel: "conflict.txt",
            dst_rel: None,
            src: File(b"a file"),
            dst: Dir,
            expect: &[("copy", "conflict.txt")],
        },
        // A src-only directory, and a file inside it.
        Case {
            src_rel: "newdir",
            dst_rel: None,
            src: Dir,
            dst: Absent,
            expect: &[("mkdir", "newdir")],
        },
        Case {
            src_rel: "newdir/nested.txt",
            dst_rel: None,
            src: File(b"nested"),
            dst: Absent,
            expect: &[("copy", "newdir/nested.txt")],
        },
    ]
}

/// Materialise the table as two trees.
///
/// Mtimes are pinned wherever both sides hold a file, so every stat difference in
/// the fixture is a difference of *size* and nothing else. Without that, two files
/// written microseconds apart can land on one timestamp in one tree and different
/// ones in the other, and a pair becomes undecided for reasons that have nothing to
/// do with the code under test.
fn build(t: &TempRoot) -> (std::path::PathBuf, std::path::PathBuf) {
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    for c in cases() {
        for (dir, rel, shape) in [
            (&src, Some(c.src_rel), c.src),
            (&dst, c.dst_rel.or(Some(c.src_rel)), c.dst),
        ] {
            let Some(rel) = rel else { continue };
            match shape {
                Shape::Absent => continue,
                Shape::Dir => std::fs::create_dir_all(dir.join(rel)).unwrap(),
                Shape::File(bytes) => wfile(dir, rel, bytes),
            }
        }
        if matches!(c.src, Shape::File(_)) && matches!(c.dst, Shape::File(_)) {
            let d = c.dst_rel.unwrap_or(c.src_rel);
            sync_mtime(&src.join(c.src_rel), &dst.join(d));
        }
    }
    (src, dst)
}

/// Assert the plan reports what the table requires for every row, and nothing else.
///
/// Two directions, because they fail differently: a missing record means the plan
/// under-reports, an extra one means it over-reports. Both name the path.
///
/// Checked twice — once for the plan print, once for the actions — because they are
/// two claims. Doing it as one set would pass for a run that printed the plan and no
/// actions, or the actions and no plan, which is precisely the split step 4 is
/// about.
fn assert_plan_matches(cases: &[Case], recs: &[Value], mode: &str) {
    for (label, reported, required) in [
        (
            "rename",
            segment_pairs(recs, is_rename),
            required_renames(cases),
        ),
        (
            "plan",
            reported_plan_pairs(recs),
            required_plan_pairs(cases),
        ),
        (
            "actions",
            reported_pairs(recs),
            required_pairs(cases)
                .into_iter()
                .filter(|(e, _)| !is_rename(e))
                .collect(),
        ),
    ] {
        for (event, subject) in &required {
            assert!(
                reported.contains(&(event.clone(), subject.clone())),
                "{mode}: the {label} segment expected a `{event}` record for {subject}\n  \
                 reported: {}",
                shown(&reported)
            );
        }
        let extra: Vec<_> = reported.difference(&required).collect();
        assert!(
            extra.is_empty(),
            "{mode}: the {label} segment reported but required by no row: {extra:?}\n  \
             required: {:?}",
            required
        );
    }
}

fn shown(pairs: &BTreeSet<(String, String)>) -> String {
    pairs
        .iter()
        .map(|(e, s)| format!("{e} {s}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Assert the summary counts follow from the plan, so it cannot disagree with it.
///
/// The counts are tallied from the records rather than written out, which is what
/// lets a new row be added without a new number — and what makes a summary that
/// lies about the plan a failure here rather than something a reader has to notice.
/// The flags are literals because they are inputs echoed back, not deductions.
fn assert_summary_agrees_with_the_plan(recs: &[Value], mode: &str) {
    let s = summary_of(recs);
    let count = |event: &str| recs.iter().filter(|r| r["event"] == event).count();
    for (field, event) in [
        ("renamed", "rename"),
        ("mkdir", "mkdir"),
        ("copied", "copy"),
        ("deleted", "delete"),
        ("rmdir", "rmdir"),
    ] {
        let n = count(event);
        assert_eq!(
            s[field], n,
            "{mode}: summary {field} disagrees with the {n} `{event}` records"
        );
    }
    assert_eq!(s["missing_only"], false, "{mode}");
    assert_eq!(s["keep_extra"], false, "{mode}");
}

/// Assert the record stream is **plan, then actions in apply order, then the
/// summary**.
///
/// One check covering three properties, all of which are about *position* rather than
/// content:
///
/// - the plan segment precedes every action — the plan is printed up-front, and an
///   action before a plan record would mean the tool acted before it had said what
///   it was doing;
/// - action kinds appear in apply order and contiguously (indices never decrease);
/// - the summary is last, because a run emits exactly one and a consumer reads
///   "the last line is the summary".
///
/// This is what a golden cannot do — it cannot ask a new record kind where it
/// applies, it can only notice afterwards that the transcript changed.
fn assert_apply_order(recs: &[Value], mode: &str) {
    // Each record's segment rank, as one ordered sequence. Monotonicity of that
    // sequence is the whole property: it says the renames come first, then the
    // plan, then the actions in apply order — and, because indices never decrease,
    // that no segment is interleaved with another.
    let rank = |r: &Value| -> usize {
        let e = r["event"].as_str().unwrap_or("");
        if is_rename(e) {
            0
        } else if is_plan(e) {
            1
        } else {
            // The apply phases, in the order `APPLY_ORDER` states (which no longer
            // holds `rename`).
            APPLY_ORDER
                .iter()
                .position(|k| *k == e)
                .unwrap_or_else(|| panic!("{mode}: `{e}` is in no known segment"))
                + 2
        }
    };
    assert_eq!(
        recs.last().map(|r| r["event"].as_str().unwrap_or("")),
        Some("summary"),
        "{mode}: the summary is last"
    );
    let ranks: Vec<usize> = recs[..recs.len() - 1].iter().map(rank).collect();
    assert!(
        ranks.windows(2).all(|w| w[0] <= w[1]),
        "{mode}: records are out of order — renames, then the plan, then the actions \
         in apply order {APPLY_ORDER:?}\n  got: {}",
        recs[..recs.len() - 1]
            .iter()
            .map(|r| r["event"].as_str().unwrap_or("?"))
            .collect::<Vec<_>>()
            .join(", ")
    );
}

#[test]
fn the_dry_run_does_exactly_what_the_table_says() {
    let t = TempRoot::new("plan");
    let (src, dst) = build(&t);
    let recs = run(&src, &dst, &["--dry-run"]);

    assert_plan_matches(&cases(), &recs, "insensitive");
    assert_summary_agrees_with_the_plan(&recs, "insensitive");
    assert_apply_order(&recs, "insensitive");
    assert_eq!(
        summary_of(&recs)["dry_run"],
        true,
        "the dry run marks itself"
    );
}

/// The same table under `--case-sensitive`, with the case-only rows resolved by
/// [`sensitive_expect`] rather than by a second hand-written plan. Two rows differ
/// between the modes and both are asserted, so the rename pass cannot quietly
/// become unconditional.
#[test]
fn case_sensitive_is_the_same_table_without_the_rename_pass() {
    let t = TempRoot::new("plan_cs");
    let (src, dst) = build(&t);
    let recs = run(&src, &dst, &["--dry-run", "--case-sensitive"]);

    let cases = cases();
    for (label, reported, required) in [
        (
            "plan",
            reported_plan_pairs(&recs),
            cases
                .iter()
                .flat_map(sensitive_plan)
                .map(|(e, s)| (e.to_string(), s))
                .collect::<BTreeSet<_>>(),
        ),
        (
            "actions",
            reported_pairs(&recs),
            cases
                .iter()
                .flat_map(sensitive_expect)
                .map(|(e, s)| (e.to_string(), s))
                .collect::<BTreeSet<_>>(),
        ),
    ] {
        assert_eq!(
            reported, required,
            "case-sensitive {label} differs from the derived expectation"
        );
    }
    assert_summary_agrees_with_the_plan(&recs, "case-sensitive");
    assert_apply_order(&recs, "case-sensitive");
    assert_eq!(summary_of(&recs)["dry_run"], true);
    assert!(
        !recs.iter().any(|r| r["event"] == "rename"),
        "there is no rename pass in this mode"
    );
}

/// The fixture must keep exercising every apply phase, or the checks above stop
/// meaning anything — a table that quietly stopped covering `rmdir` would still
/// pass. Asserted rather than assumed, so losing a row fails loudly.
#[test]
fn the_fixture_covers_every_apply_phase() {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for c in cases() {
        for (e, _) in c.expect {
            seen.insert(*e);
        }
        for (e, _) in sensitive_expect(&c) {
            seen.insert(e);
        }
    }
    // And the rename pass must stay covered, even though it is no longer an apply
    // phase: it is what makes the diff possible at all.
    assert!(
        seen.contains("rename"),
        "no row expects a `rename`, so the casing pass is no longer covered"
    );
    for kind in APPLY_ORDER {
        assert!(
            seen.contains(kind),
            "no row expects a `{kind}` record, so the fixture no longer covers that phase"
        );
    }
    // And the row that carries the planner's case flag, since it is the only thing
    // in this file that depends on `plan_pairs` pairing by lowercase.
    assert!(
        cases()
            .iter()
            .any(|c| c.is_case_only() && c.expect.contains(&("copy", c.src_rel))),
        "the case-only-different-content row is gone; the planner's case flag is unguarded"
    );
}

/// A dry run must report a plan and change nothing — not the filesystem, not
/// either cache. This is the promise laziness is most able to break, because it
/// stops both sides from writing the backfill a real run would.
#[test]
fn dry_run_changes_nothing() {
    let t = TempRoot::new("plan_untouched");
    let (src, dst) = build(&t);
    // Warm both caches so there is something a stray write could damage.
    for dir in [&src, &dst] {
        girsync::cmd_update(
            girsync::UpdateOpts {
                dir: dir.clone(),
                common: common::opts(),
            },
            &common::log(),
        )
        .unwrap();
    }
    let before = snapshot(&src);
    let dst_before = snapshot(&dst);

    let _ = run(&src, &dst, &["--dry-run"]);

    assert_eq!(snapshot(&src), before, "src is byte-identical");
    assert_eq!(
        snapshot(&dst),
        dst_before,
        "dst, cache included, is byte-identical"
    );
    assert!(
        !snapshot(&src).is_empty(),
        "the snapshot is not vacuously empty"
    );
}

/// Every byte under `dir`, cache included.
///
/// The cache is deliberately *not* filtered out. A dry run's real promise is that
/// neither cache changes, and a correct plan says nothing about that — a stray
/// prune, a stat-only row or a `meta` rewrite would leave every record untouched.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = walk(dir)
        .into_iter()
        .map(|(rel, p)| (rel, std::fs::read(p).unwrap_or_default()))
        .collect();
    out.sort();
    out
}

fn walk(dir: &Path) -> Vec<(String, std::path::PathBuf)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((
                    p.strip_prefix(dir).unwrap().to_string_lossy().into_owned(),
                    p,
                ));
            }
        }
    }
    out
}

/// A dry run and a real run must report the same thing.
///
/// This is the differential that a golden cannot replace, and it is worth keeping
/// precisely because it is *not* against a transcript: both sides are the same code
/// path with only the write gate differing, so any divergence means the dry run took
/// a different branch — which is exactly the `--dry-run` contract. Two runs over two
/// pristine fixtures, because a dry run writes nothing and so cannot disturb the
/// state a real run reads.
///
/// ## Compared as two segments, not as one document
///
/// The plan is decided before anything is written, so it is **ordered** and compared
/// field for field. The actions are *as-done*, and completion order is a function of
/// I/O timing rather than of the plan — so from step 6 they are compared as a
/// **multiset**. That is a real weakening, and it is not something to engineer around:
/// the alternative is to keep the pool joining before emitting, which is exactly the
/// behaviour step 6 removes.
///
/// What the weakening does not touch is the count. A dropped or duplicated `COPY`
/// changes the multiset, so a dry run that skipped a file, or copied one twice, still
/// fails here.
#[test]
fn a_dry_run_reports_what_a_real_run_reports() {
    let dry_root = TempRoot::new("parity_dry");
    let (dry_src, dry_dst) = build(&dry_root);
    let dry = run(&dry_src, &dry_dst, &["--dry-run"]);
    let real_root = TempRoot::new("parity_real");
    let (real_src, real_dst) = build(&real_root);
    let real = run(&real_src, &real_dst, &[]);

    let shown = |recs: &[Value]| {
        recs.iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    };

    // Partition at the summary: everything before it, and the summary itself.
    let split = |recs: &[Value]| -> (Vec<Value>, Value) {
        let at = recs
            .iter()
            .position(|r| r["event"] == "summary")
            .unwrap_or_else(|| panic!("no summary in:\n{}", shown(recs)));
        (recs[..at].to_vec(), recs[at].clone())
    };
    let (dry_body, dry_sum) = split(&dry);
    let (real_body, real_sum) = split(&real);

    // The decided part: ordered, field for field. Renames and the plan print come
    // first and are deterministic.
    let decided = |recs: &[Value]| -> Vec<Value> {
        recs.iter()
            .filter(|r| {
                is_rename(r["event"].as_str().unwrap_or(""))
                    || is_plan(r["event"].as_str().unwrap_or(""))
            })
            .cloned()
            .collect()
    };
    assert_eq!(
        decided(&dry_body),
        decided(&real_body),
        "a dry run decides the same plan a real run does\n--- dry ---\n{}\n--- real ---\n{}",
        shown(&dry_body),
        shown(&real_body)
    );

    // The done part: as a multiset, because completion order is a function of I/O
    // timing. Counts are still exact — only the order is not asserted.
    let acted = |recs: &[Value]| -> BTreeSet<String> {
        recs.iter()
            .filter(|r| is_action(r["event"].as_str().unwrap_or("")))
            .map(|r| r.to_string())
            .collect()
    };
    let dry_actions = acted(&dry_body);
    let real_actions = acted(&real_body);
    assert_eq!(
        dry_actions,
        real_actions,
        "a dry run reports the same actions a real run does, as a set\n--- dry ---\n{}\n\
         --- real ---\n{}",
        shown(&dry_body),
        shown(&real_body)
    );
    // The multiset claim is only as strong as the count it implies, so state it: a
    // duplicate would collapse in a `BTreeSet`, and a dropped action is the failure
    // this whole test exists to catch.
    assert_eq!(
        dry_body
            .iter()
            .filter(|r| is_action(r["event"].as_str().unwrap_or("")))
            .count(),
        real_body
            .iter()
            .filter(|r| is_action(r["event"].as_str().unwrap_or("")))
            .count(),
        "and the same number of them, so a duplicate cannot hide in the set"
    );

    // The summary: same fields, same numbers, `dry_run` the only difference.
    let mut dry_sum_n = dry_sum.clone();
    let mut real_sum_n = real_sum.clone();
    for s in [&mut dry_sum_n, &mut real_sum_n] {
        if let Some(o) = s.as_object_mut() {
            o.remove("dry_run");
        }
    }
    assert_eq!(
        dry_sum_n, real_sum_n,
        "and the same summary, with only the dry_run marker differing"
    );
    assert_eq!(
        summary_of(&dry)["dry_run"],
        true,
        "the dry run marks itself"
    );
    assert_eq!(summary_of(&real)["dry_run"], false);
}

// -- the plan print -------------------------------------------------------------

/// **The sync report is two segments: the plan, then the actions.** A consumer that
/// wants to know what the tool *decided* reads the first; one that wants to know what
/// it *did* reads the second. They are one stream because a plan you cannot correlate
/// with the actions is not a plan.
#[test]
fn sync_reports_the_diff_as_its_plan_before_it_acts() {
    let t = TempRoot::new("plan_print");
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

/// **`sync` still exits 0 with a non-empty plan.** The single most damaging thing this
/// feature could do by accident: printing a plan makes `sync` *look* like `compare`,
/// and `compare` exits 4 on a difference. Returning 4 here would break every script
/// that syncs a partly-different tree, and it is not what was asked for.
///
/// Asserted against a plan that is genuinely non-empty, so it cannot pass vacuously.
#[test]
fn sync_still_exits_zero_with_a_non_empty_plan() {
    let t = TempRoot::new("plan_exit");
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
    let t = TempRoot::new("plan_missing");
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
/// action says what it is doing, and those are different questions. Step 5 is why
/// `COPY` never carries a `why=`.
#[test]
fn the_plan_carries_reasons_when_why_is_passed() {
    let t = TempRoot::new("plan_why");
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

/// **`--show-identical` reaches sync's plan print too.** It is a `compare`-shaped
/// flag, and the plan print *is* a compare-shaped report — so accepting it on one and
/// not the other would be an inconsistency a user could trip over.
#[test]
fn sync_plan_honours_show_identical() {
    let t = TempRoot::new("plan_show_identical");
    let (src, dst) = build(&t);
    let flagged = run(&src, &dst, &["--dry-run", "--show-identical"]);
    assert!(
        flagged.iter().any(|r| r["event"] == "identical"),
        "`--show-identical` is accepted on sync's plan print: {flagged:?}"
    );
    let unflagged = run(&src, &dst, &["--dry-run"]);
    assert!(
        !unflagged.iter().any(|r| r["event"] == "identical"),
        "and it is still default off: {unflagged:?}"
    );
}
