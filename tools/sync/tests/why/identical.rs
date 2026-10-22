//! `--show-identical`: the second bucket, and how it is reported.
//!
//! The harness and the fixture builders are in the parent module.

use super::*;

/// A diff whose pairs all came out equal, with the `identical` bucket filled and
/// **no** difference bucket touched.
fn equal_only(count: usize) -> Diff {
    let mut d = Diff::default();
    for i in 0..count {
        let rel = format!("same{i}.txt");
        d.identical.push(rel.clone());
        d.verdicts.insert(
            rel,
            Verdict::Matches {
                algos: AlgoSet::one(HashAlg::Md5),
            },
        );
    }
    d
}

/// **The gate, stated first because it is the most damaging bug this feature could
/// have.** `Diff::total()` feeds the exit code. A clean tree whose pairs were all
/// reported equal must still exit `0`; counting the equal set would turn "these trees
/// are identical" into exit `4` — the exact inverse of the truth.
#[test]
fn equal_pairs_never_count_as_differences() {
    let d = equal_only(5);
    assert_eq!(
        d.total(),
        0,
        "five equal pairs are not five differences: {} identical entries",
        d.identical.len()
    );
    assert!(
        d.is_empty(),
        "and a diff that found nothing different is empty"
    );
    // And one real difference alongside them still counts exactly once.
    let mut mixed = equal_only(5);
    mixed.changed.push("b.txt".into());
    mixed.verdicts.insert(
        "b.txt".into(),
        Verdict::Stat {
            field: StatField::Size,
            both: false,
        },
    );
    assert_eq!(
        mixed.total(),
        1,
        "the equal pairs must not inflate the count a consumer reads"
    );
    assert!(!mixed.is_empty());
}

/// **`is_empty()` must not treat an equal pair as a difference.** Separate from
/// `total()` because it is a different predicate — it also consults `case_mismatch`,
/// which `total()` deliberately ignores — so one can be correct while the other is
/// not.
#[test]
fn an_all_equal_diff_is_empty() {
    assert!(
        equal_only(1).is_empty(),
        "one equal pair is still no difference"
    );
    // A case mismatch *is* a difference for this predicate even though `total()`
    // does not count it, so the two must not be conflated.
    let mut cased = equal_only(1);
    cased.case_mismatch.push(("a.txt".into(), "A.txt".into()));
    assert!(
        !cased.is_empty(),
        "a casing difference is still a difference"
    );
    assert_eq!(
        cased.total(),
        0,
        "and `total()` still excludes it, exactly as before"
    );
}

/// **The bucket is gated, not merely unprinted.** `diff_maps` is asked for it or not;
/// an always-populated field would make `total()` and `is_empty()` wrong by default
/// and turn the gate into a convention rather than a property.
#[test]
fn the_identical_bucket_is_empty_unless_it_was_asked_for() {
    let t = temp_root("identical_gate");
    let (src, dst) = pinned(&t, "a.txt", b"same", b"same");
    std::fs::create_dir_all(src.join("d")).unwrap();
    std::fs::create_dir_all(dst.join("d")).unwrap();
    let common = md5();
    let (sm, dm, plans) = resolved("gate", &src, &dst, &common);

    let off = diff_maps(
        &sm,
        &dm,
        &plans.required,
        plans.stat,
        CasePairing::from(common.case_sensitive),
        false,
    );
    assert!(
        off.identical.is_empty(),
        "unflagged: {} entries, and none may be reported",
        off.identical.len()
    );
    assert_eq!(off.total(), 0, "so the exit code is unaffected");
    assert!(off.is_empty());

    let on = diff_maps(
        &sm,
        &dm,
        &plans.required,
        plans.stat,
        CasePairing::from(common.case_sensitive),
        true,
    );
    assert_eq!(
        on.identical,
        vec!["a.txt".to_string(), "d".to_string()],
        "flagged: both equal pairs, and **the directory too** — leaving it out would \
         make the flag's output look arbitrarily partial"
    );
    assert_eq!(
        on.total(),
        0,
        "and still not a difference: the gate must not reach the exit code"
    );
    assert!(on.is_empty());
}

/// **A `CHANGED` path is never also `IDENTICAL`.** The two buckets are read off one
/// classification, so a path in both would be copied *and* reported equal — the two
/// claims cannot both be true.
#[test]
fn a_pair_is_never_both_changed_and_identical() {
    let t = temp_root("identical_exclusive");
    let (src, dst) = pinned(&t, "same.txt", b"same", b"same");
    // Both sides written *before* the pin — pinning a path dst does not hold yet
    // opens nothing and fails. Size is the only difference, so this pair is settled
    // by stat and lands in `CHANGED`.
    wfile(&src, "differing.txt", b"a much longer body on this side");
    wfile(&dst, "differing.txt", b"short");
    sync_mtime(&src.join("differing.txt"), &dst.join("differing.txt"));
    let common = md5();
    let (sm, dm, plans) = resolved("exclusive", &src, &dst, &common);

    let d = diff_maps(
        &sm,
        &dm,
        &plans.required,
        plans.stat,
        CasePairing::from(common.case_sensitive),
        true,
    );
    assert_eq!(d.changed, vec!["differing.txt".to_string()]);
    assert_eq!(d.identical, vec!["same.txt".to_string()]);
    let both: Vec<_> = d
        .changed
        .iter()
        .filter(|p| d.identical.contains(p))
        .collect();
    assert!(
        both.is_empty(),
        "a path cannot be in both buckets: {both:?}"
    );
    assert_eq!(d.total(), 1, "one difference, one equal pair");
}

// -- reporting the second bucket -----------------------------------------------

/// Every record a diff reports, rendered as text.
///
/// Both reporting flags at once, because they are orthogonal and a case that only
/// ever passed one of them would not show that: a `CHANGED` with no reason, and an
/// `IDENTICAL` with one.
fn report_all(diff: &Diff, why: bool, show_identical: bool) -> Vec<String> {
    girsync::verdict(diff, why, show_identical)
        .iter()
        .map(|r| r.text())
        .collect()
}

/// **`IDENTICAL` records come after the differences and before the `SUMMARY`**, each
/// carrying its own reason — the flag is a second bucket, not a second vocabulary.
///
/// The order matters: the summary counts them, so a consumer reading "the last line
/// is the summary" must not find an `IDENTICAL` after it.
#[test]
fn identical_records_are_reported_with_their_own_reasons() {
    let mut diff = Diff {
        missing: vec!["m.txt".into()],
        changed: vec!["c.txt".into()],
        identical: vec!["same.txt".into(), "other.txt".into()],
        ..Diff::default()
    };
    diff.verdicts.insert(
        "c.txt".into(),
        Verdict::Differs {
            algos: AlgoSet::one(HashAlg::Md5),
        },
    );
    diff.verdicts.insert(
        "same.txt".into(),
        Verdict::Matches {
            algos: AlgoSet::one(HashAlg::Md5),
        },
    );
    diff.verdicts
        .insert("other.txt".into(), Verdict::DirPresent);

    let rendered = report_all(&diff, true, true);
    assert_eq!(
        &rendered[..rendered.len() - 1],
        [
            "MISSING m.txt",
            "CHANGED c.txt why=digest-differs:md5",
            "IDENTICAL same.txt why=digest-matches:md5",
            "IDENTICAL other.txt why=dir-present",
        ],
        "flagged: the equal pairs report after the differences, each with its own reason"
    );
    assert!(
        rendered.last().unwrap().starts_with("SUMMARY "),
        "and the summary is still last: {:?}",
        rendered.last()
    );
    assert_eq!(
        rendered.last().unwrap(),
        "SUMMARY missing=1 extra=0 changed=1 type_conflict=0 case_mismatch=0 identical=2 total_diff=2",
        "the summary counts them, and total_diff still excludes them"
    );
}

/// **`SUMMARY` gains `identical=N` only when the flag is on**, so an unflagged run
/// stays byte-identical to a run that never passes it.
///
/// A conditional field rather than an unconditional one: a `SUMMARY` is a
/// machine-read record, and adding a field to every consumer's parse on every run
/// would be a cost paid by the runs that did not ask for it.
#[test]
fn the_summary_gains_identical_only_when_the_flag_is_on() {
    let mut diff = Diff {
        changed: vec!["c.txt".into()],
        identical: vec!["same.txt".into()],
        ..Diff::default()
    };
    diff.verdicts.insert("same.txt".into(), Verdict::StatMatch);
    diff.verdicts.insert(
        "c.txt".into(),
        Verdict::Stat {
            field: StatField::Size,
            both: false,
        },
    );

    assert_eq!(
        report_all(&diff, false, false),
        vec![
            "CHANGED c.txt",
            "SUMMARY missing=0 extra=0 changed=1 type_conflict=0 case_mismatch=0 \
             total_diff=1"
        ],
        "unflagged: no identical records, and no identical field either"
    );
    assert_eq!(
        report_all(&diff, false, true),
        vec![
            "CHANGED c.txt",
            // No `why=`: the two flags are independent, and a tag is `--why`'s to
            // give. An `IDENTICAL` without a reason is not a worse record than a
            // `CHANGED` without one — both say what happened, and `--why` says why.
            "IDENTICAL same.txt",
            "SUMMARY missing=0 extra=0 changed=1 type_conflict=0 case_mismatch=0 identical=1 total_diff=1",
        ],
        "flagged: the records appear and the summary counts them, and total_diff \
         still counts only the one real difference"
    );
    assert_eq!(
        report_all(&diff, true, true),
        vec![
            "CHANGED c.txt why=stat-size",
            "IDENTICAL same.txt why=stat-match",
            "SUMMARY missing=0 extra=0 changed=1 type_conflict=0 case_mismatch=0 identical=1 total_diff=1",
        ],
        "and with both flags, each record names why it landed in its bucket"
    );
}
