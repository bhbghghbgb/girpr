//! As-done action printing: the actions, as they complete.
//!
//! The second segment. `COPY` records are emitted inside the copy worker, so under a
//! parallel rw limit their order is a function of I/O timing — what must hold is
//! the *set* and the count, never the order.

use super::*;

/// **`COPY` records are emitted as each copy completes, not after the pool joins.**
///
/// The only honest observable is that the *set* is unchanged and every file is still
/// copied: completion **order** is a function of I/O timing when more than one rw
/// operation is permitted, so asserting an order here would be asserting the
/// machine's mood. What must hold is that moving the emit into the worker changed
/// nothing about *what* was reported.
#[test]
fn actions_are_reported_as_they_complete_under_a_parallel_rw_limit() {
    let t = temp_root("asdone");
    let (src, dst) = build(&t);
    let recs = run(&src, &dst, &["--rw-threads", "4"]);

    // Everything still copied, each exactly once, with the same reasons.
    let copies: BTreeSet<(&str, &str)> = recs
        .iter()
        .filter(|r| r["event"] == "copy")
        .map(|r| (r["path"].as_str().unwrap(), r["because"].as_str().unwrap()))
        .collect();
    let expected: BTreeSet<(&str, &str)> = [
        ("Case.txt", "changed"),
        ("conflict.txt", "type-conflict"),
        ("differs.txt", "changed"),
        ("fixdir/inner.txt", "missing"),
        ("newdir/nested.txt", "missing"),
        ("onlysrc.txt", "missing"),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        copies, expected,
        "--rw-threads 4 must report the same copies"
    );
    assert_eq!(
        recs.iter().filter(|r| r["event"] == "copy").count(),
        copies.len(),
        "and no copy is reported twice"
    );
    assert_eq!(
        summary_of(&recs)["copied"].as_u64().unwrap(),
        copies.len() as u64,
        "the summary agrees, so nothing was silently dropped from the count"
    );
}

/// **`--rw-threads 1` and `--rw-threads 4` report the same set of actions.** The
/// only thing the rw limit may affect is which thread finishes first. If it
/// changed *what* was reported, the limit would be a correctness flag.
///
/// This is the general form of the crate's rule — the limiter changes scheduling and
/// nothing else — on the surface that reports actions. `rw_threads/` pins the same
/// property on the resolver, against effective maps and `ScanStats` rather than
/// printed records; this one is here because the action stream is what a caller
/// actually reads.
#[test]
fn the_rw_limit_does_not_change_what_is_reported() {
    let t1 = temp_root("asdone_1");
    let (src1, dst1) = build(&t1);
    let one = run(&src1, &dst1, &["--rw-threads", "1"]);

    let t4 = temp_root("asdone_4");
    let (src4, dst4) = build(&t4);
    let four = run(&src4, &dst4, &["--rw-threads", "4"]);

    let acts = |recs: &[Value]| -> BTreeSet<String> {
        recs.iter()
            .filter(|r| is_action(r["event"].as_str().unwrap_or("")))
            .map(|r| r.to_string())
            .collect()
    };
    assert_eq!(
        acts(&one),
        acts(&four),
        "the rw limit changes who finishes first, not what is reported"
    );
    assert_eq!(
        one.iter().filter(|r| r["event"] == "copy").count(),
        four.iter().filter(|r| r["event"] == "copy").count(),
        "and the action counts must agree exactly"
    );
}

/// **Every planned copy really happened**, under a parallel rw limit. The `COPY`
/// record is emitted from inside the copy worker, which is exactly the kind of
/// placement that can leave a file copied but unreported — so the filesystem is
/// asserted, not only the stream.
#[test]
fn parallel_copies_actually_land_on_disk() {
    let t = temp_root("asdone_disk");
    let (src, dst) = build(&t);
    // A real run this time: the point is that the files arrive.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .arg("sync")
        .arg("--src")
        .arg(&src)
        .arg("--dst")
        .arg(&dst)
        .arg("--rw-threads")
        .arg("4")
        .arg("--output")
        .arg("json")
        .env("GIRSYNC_TEST_CACHE", "memory")
        .output()
        .expect("spawn girsync");
    assert!(out.status.success(), "sync failed");
    for rel in [
        "onlysrc.txt",
        "differs.txt",
        "Case.txt",
        "conflict.txt",
        "fixdir/inner.txt",
        "newdir/nested.txt",
    ] {
        assert!(
            dst.join(rel).is_file(),
            "{rel} was planned and copied, so it must be on disk"
        );
    }
    assert_eq!(
        std::fs::read(dst.join("onlysrc.txt")).unwrap(),
        b"only here",
        "and it holds src's bytes, not an empty placeholder"
    );
}

/// **The parity test never asserts an action order** — and this is the case that
/// proves it.
///
/// A single-file fixture finishes too fast to shuffle, so an order assertion would
/// pass against any implementation and give false comfort. Twenty-four files
/// of increasing size under `--rw-threads 4` do shuffle, and it is this one — the
/// multiset comparison — that has to hold. Asserted as a *property of the fixture*
/// rather than of a particular run: the order may still come out sorted on a lucky
/// machine, and a test that depended on it would be a flaky test.
#[test]
fn a_multi_file_parallel_copy_reports_the_same_set_whatever_the_order() {
    let t = temp_root("asdone_many");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    // Increasing sizes (1 KB up to ~920 KB, ~11 MB total), so the copies take
    // measurably different times and completion order is free to diverge from plan
    // order.
    for i in 0..24usize {
        wfile(
            &src,
            &format!("pad{i:02}.bin"),
            &vec![b'x'; i * 40_000 + 1024],
        );
    }
    let recs = run(&src, &dst, &["--rw-threads", "4"]);
    let copies: Vec<&str> = recs
        .iter()
        .filter(|r| r["event"] == "copy")
        .filter_map(|r| r["path"].as_str())
        .collect();
    assert_eq!(
        copies.len(),
        24,
        "every planned file is reported exactly once"
    );
    let unique: BTreeSet<&&str> = copies.iter().collect();
    assert_eq!(unique.len(), 24, "and none twice");
    for p in &unique {
        assert!(
            dst.join(*p).is_file(),
            "{p} was reported as copied, so it must exist"
        );
    }
}

/// **`--show-identical` reaches sync's plan print too.** It is a `compare`-shaped
/// flag, and the plan print *is* a compare-shaped report — so accepting it on one and
/// not the other would be an inconsistency a user could trip over.
#[test]
fn sync_plan_honours_show_identical() {
    let t = temp_root("plan_show_identical");
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
