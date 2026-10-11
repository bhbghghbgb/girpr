//! `--dry-run`: the same document, and no writes.
//!
//! A dry run has to answer the same question as a real run, so the comparison here
//! is between two runs of the same code differing only in the write gate. That is
//! the one whole-document comparison the suite keeps, and it earns it: a divergence
//! means the dry run took a different branch. It shares this fixture with the rest
//! of the target, so a change to the tree the plan runs over fails every segment at
//! once rather than quietly making one of them vacuous.

use super::*;

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
/// I/O timing rather than of the plan — so they are compared as a
/// **multiset**. That is a real weakening, and it is not something to engineer around:
/// the alternative is to keep the pool joining before emitting, which would serialise
/// the reporting behind the work it reports on.
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
