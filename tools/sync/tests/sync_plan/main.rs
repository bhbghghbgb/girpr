//! The `sync` plan, asserted as *what happens to each path*.
//!
//! ## Why there is no golden here
//!
//! A whole-run golden is a transcript: one hardcoded vector for everything the run
//! reported. It works only while a reference implementation exists to compare
//! against, and the only way to update one once that is gone is to paste the
//! actual output — which requires no understanding of whether that output is right.
//! From that point it is a change detector wearing the costume of a specification,
//! and every future behaviour lands as a diff against a wall of JSON rather than as
//! a statement about intent.
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
//! It does not compare a dry run against a real run — that belongs in [`dry_run`],
//! which shares this fixture.
//!
//! ## Layout
//!
//! This file holds the table itself and the vocabulary every case reads — the
//! `Shape`/`Case` types, the fixture, and the paths a run is driven through. The
//! cases are grouped by the part of the two-segment report they pin:
//!
//! | module | what it covers |
//! | --- | --- |
//! | [`table`] | the derived assertions, plus the table asserted per path and per mode |
//! | [`dry_run`] | that a dry run reports the same document and writes nothing |
//! | [`plan_print`] | the first segment: the diff, printed before anything happens |
//! | [`as_done`] | the second segment: the actions, as they complete |

// This target lives at `tests/sync_plan/main.rs`, so the shared fixtures one
// level up need an explicit path rather than the sibling `mod common;` every
// other test target gets.
#[path = "../common/mod.rs"]
mod common;

mod as_done;
mod dry_run;
mod plan_print;
mod table;

use std::collections::BTreeSet;
use std::path::Path;

use common::{TempDir, parse_ndjson, summary_of, sync_mtime, temp_root, wfile};
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
fn build(t: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let src = t.subdir("src");
    let dst = t.subdir("dst");
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
