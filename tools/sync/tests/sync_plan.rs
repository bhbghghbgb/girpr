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
struct Case {
    /// The rel path on src, and the destination spelling for a case-only pair.
    src_rel: &'static str,
    /// The rel path on dst, where it is spelled differently.
    dst_rel: Option<&'static str>,
    src: Shape,
    dst: Shape,
    expect: &'static [(&'static str, &'static str)],
}

impl Case {
    /// What to call this path in a failure message.
    fn name(&self) -> &'static str {
        match self.src {
            Shape::Absent => self.dst_rel.unwrap_or(self.src_rel),
            _ => self.src_rel,
        }
    }

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
const APPLY_ORDER: &[&str] = &["rename", "mkdir", "fix-dir", "delete", "copy", "rmdir"];

/// Every `(event, subject)` the plan reported, sorted, excluding the summary.
fn reported_pairs(recs: &[Value]) -> BTreeSet<(String, String)> {
    recs.iter()
        .filter_map(|r| {
            let event = r["event"].as_str()?.to_string();
            if event == "summary" {
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

/// Every `(event, subject)` the table requires, sorted.
fn required_pairs(cases: &[Case]) -> BTreeSet<(String, String)> {
    cases
        .iter()
        .flat_map(|c| c.expect.iter())
        .map(|(e, s)| (e.to_string(), s.to_string()))
        .collect()
}

/// The expectation for `--case-sensitive`, derived from the same rows.
///
/// There is no rename pass in that mode, so a case-only pair is no longer the same
/// path: it becomes two unrelated paths, so src's is copied and dst's is deleted.
/// Everything else about the plan is unchanged, and every other row is taken as
/// written. Stating this as a rule rather than as a second table is the point —
/// the difference between the two modes *is* the rename pass, so it should read as
/// one line of code and not as a parallel transcript that has to be kept in sync.
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
fn assert_plan_matches(cases: &[Case], recs: &[Value], mode: &str) {
    let reported = reported_pairs(recs);
    for c in cases {
        for (event, subject) in c.expect {
            assert!(
                reported.contains(&(event.to_string(), subject.to_string())),
                "{mode}: {} expected a `{event}` record for {subject}\n  reported: {}",
                c.name(),
                shown(&reported)
            );
        }
    }
    let required = required_pairs(cases);
    let extra: Vec<_> = reported.difference(&required).collect();
    assert!(
        extra.is_empty(),
        "{mode}: reported but required by no row: {extra:?}\n  required: {:?}",
        required
    );
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

/// Assert event kinds appear in apply order, contiguously, with the summary last.
///
/// One check covering two properties: indices that never decrease mean each kind's
/// records are contiguous, because a kind cannot reappear once another has started.
/// This is what a golden cannot do — it cannot ask a new record kind where it
/// applies, it can only notice afterwards that the transcript changed.
fn assert_apply_order(recs: &[Value], mode: &str) {
    let rank = |r: &Value| -> usize {
        let e = r["event"].as_str().unwrap_or("");
        APPLY_ORDER.iter().position(|k| *k == e).unwrap_or_else(|| {
            panic!("{mode}: `{e}` is not a known apply phase in {APPLY_ORDER:?}")
        })
    };
    assert_eq!(
        recs.last().map(|r| r["event"].as_str().unwrap_or("")),
        Some("summary"),
        "{mode}: the summary is last"
    );
    let ranks: Vec<usize> = recs[..recs.len() - 1].iter().map(rank).collect();
    assert!(
        ranks.windows(2).all(|w| w[0] <= w[1]),
        "{mode}: records are not in apply order {APPLY_ORDER:?}\n  got: {}",
        shown(&reported_pairs(recs))
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
    let required: BTreeSet<(String, String)> = cases
        .iter()
        .flat_map(sensitive_expect)
        .map(|(e, s)| (e.to_string(), s))
        .collect();
    let reported = reported_pairs(&recs);
    assert_eq!(
        reported, required,
        "case-sensitive plan differs from the derived expectation"
    );
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

/// A dry run and a real run must report the same thing, field for field.
///
/// This is the differential that a golden cannot replace, and it is worth keeping as
/// a whole-document comparison precisely because it is *not* against a transcript:
/// both sides are the same code path with only the write gate differing, so any
/// divergence means the dry run took a different branch — which is exactly the
/// `--dry-run` contract. Two runs over two pristine fixtures, because a dry run
/// writes nothing and so cannot disturb the state a real run reads.
#[test]
fn a_dry_run_reports_what_a_real_run_reports() {
    let dry_root = TempRoot::new("parity_dry");
    let (dry_src, dry_dst) = build(&dry_root);
    let dry = run(&dry_src, &dry_dst, &["--dry-run"]);
    let real_root = TempRoot::new("parity_real");
    let (real_src, real_dst) = build(&real_root);
    let real = run(&real_src, &real_dst, &[]);

    let normalised = |recs: &[Value]| -> Vec<Value> {
        recs.iter()
            .map(|r| {
                let mut r = r.clone();
                if let Some(o) = r.as_object_mut() {
                    o.remove("dry_run");
                }
                r
            })
            .collect()
    };
    let shown = |recs: &[Value]| {
        recs.iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        normalised(&dry),
        normalised(&real),
        "a dry run reports what a real run reports\n--- dry ---\n{}\n--- real ---\n{}",
        shown(&dry),
        shown(&real)
    );
    assert_eq!(
        summary_of(&dry)["dry_run"],
        true,
        "the dry run marks itself"
    );
    assert_eq!(summary_of(&real)["dry_run"], false);
}
