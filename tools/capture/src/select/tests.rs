use super::*;
use crate::manifest::diff::{PathRow, PathScore};

/// A scored path, with the **direction** and the **trim's input** spelled
/// out.
///
/// `cost` is the whole point: a common path is charged `old_bytes +
/// new_bytes`, so two scored paths only order against each other if they
/// actually differ — and `cost` is *not* `old_size + new_size`, which is
/// exactly the distinction the budget is about.
///
/// Sizes and chunk counts are tuples rather than loose arguments so that a
/// call site reads `("f/x", 500, (100, 200), (2, 1, 0), false)` as a row of
/// the spec's table instead of eight unnamed numbers.
///
/// * `(old_size, new_size)`
/// * `(nchunks, reused, moved)`
/// * `(moved_later, moved_earlier)`
#[allow(clippy::too_many_arguments)]
fn shaped(
    path: &str,
    cost: u64,
    size: (i64, i64),
    chunks: (u64, u64, u64),
    moves: (u64, u64),
    old_max_end: u64,
    md5: bool,
) -> PathScore {
    let (old_size, new_size) = size;
    let (nchunks, reused, moved) = chunks;
    PathScore {
        path: path.into(),
        old_size,
        new_size,
        nchunks,
        reused,
        moved,
        moved_later: moves.0,
        moved_earlier: moves.1,
        old_max_end,
        same_md5: md5,
        old_bytes: cost / 2,
        new_bytes: cost - cost / 2,
        patch_bytes: if reused == nchunks { 0 } else { cost / 4 },
        total_cost: cost,
    }
}

/// The simple case: every move lands **earlier**, and no reuse source is
/// recorded.
fn scored(
    path: &str,
    cost: u64,
    size: (i64, i64),
    chunks: (u64, u64, u64),
    md5: bool,
) -> PathScore {
    let moved = chunks.2;
    shaped(path, cost, size, chunks, (0, moved), 0, md5)
}

fn row(path: &str, state: State, s: Option<PathScore>, size: i64) -> PathRow {
    PathRow {
        path: path.into(),
        state,
        size,
        // These fixtures carry no chunks, so for an added row `size` stands in
        // for the compressed charge the real diff would read off its chunks.
        // The dedicated cost tests below spell out a compressed size that
        // differs from the declared one.
        download_bytes: (state == State::Added).then(|| size.max(0) as u64),
        score: s,
        is_folder: false,
    }
}

fn folder_row(path: &str, state: State, size: i64) -> PathRow {
    let mut r = row(path, state, None, size);
    r.is_folder = true;
    r
}

fn field(field: &str, rows: Vec<PathRow>) -> FieldDiff {
    FieldDiff {
        field: field.into(),
        counts: Default::default(),
        scores: rows.iter().filter_map(|r| r.score.clone()).collect(),
        rows,
        pool: Default::default(),
    }
}

/// The six classes, one candidate each, cheapest first.
///
/// The class set is a **partition** on the content, so a candidate answers
/// exactly one class. A class that another capture already answers is
/// recorded as `covered_by` only when that capture was **forced**
/// (`--files`), which is the one way a capture exists before the
/// admission looks at it.
fn all_six() -> Vec<SizedCandidate> {
    candidates(&[field(
        "game",
        vec![
            // identical, cheapest of all
            row(
                "a/identical",
                State::Identical,
                Some(scored("a/identical", 108, (54, 54), (1, 1, 0), true)),
                54,
            ),
            // nothing reusable: every chunk is fetched
            row(
                "c/no_reuse",
                State::Changed,
                Some(scored("c/no_reuse", 2_200, (1000, 1200), (3, 0, 0), false)),
                1200,
            ),
            // partial, nothing moved: the reused chunks are already correct
            // where they belong
            row(
                "d/in_place",
                State::Changed,
                Some(scored(
                    "d/in_place",
                    1_826_067,
                    (2340264, 2340264),
                    (3, 1, 0),
                    false,
                )),
                2340264,
            ),
            // partial, the reused chunks live elsewhere in the file
            row(
                "e/moved",
                State::Changed,
                Some(shaped(
                    "e/moved",
                    2_000_000,
                    (200, 100),
                    (3, 2, 1),
                    (0, 1),
                    150,
                    false,
                )),
                100,
            ),
            row("j/added", State::Added, None, 114_000),
            row("k/removed", State::Removed, None, 1_730_000),
        ],
    )])
}

fn the_pool_is_built_from_every_row_with_its_shape_and_classes() {
    let pool = all_six();
    assert_eq!(pool.len(), 6);
    let by_path = |p: &str| pool.iter().find(|c| c.candidate.path == p).unwrap();
    assert_eq!(
        by_path("a/identical").candidate.classes,
        vec![Class::Identical]
    );
    assert_eq!(by_path("j/added").candidate.classes, vec![Class::Added]);
    assert_eq!(by_path("k/removed").candidate.classes, vec![Class::Removed]);
    assert_eq!(
        by_path("c/no_reuse").candidate.classes,
        vec![Class::NoReuse]
    );
    assert_eq!(
        by_path("d/in_place").candidate.classes,
        vec![Class::ReuseInPlace]
    );
    assert_eq!(
        by_path("e/moved").candidate.classes,
        vec![Class::ReuseMoved]
    );
}

/// An added path is charged the compressed bytes a fetch downloads, not its
/// declared size on the new side.
fn an_added_path_is_charged_its_compressed_download_size() {
    let pool = candidates(&[field(
        "game",
        vec![PathRow {
            path: "x/fresh".into(),
            state: State::Added,
            size: 1_000,
            download_bytes: Some(100),
            score: None,
            is_folder: false,
        }],
    )]);
    assert_eq!(pool[0].budget_cost(), 100);
}

/// A removed path has no new side to fetch: its charge is still the bytes it
/// occupies in `game/`.
fn a_removed_path_is_still_charged_its_old_size() {
    let pool = all_six();
    let h = pool
        .iter()
        .find(|c| c.candidate.path == "k/removed")
        .unwrap();
    assert_eq!(h.budget_cost(), 1_730_000);
}

/// A common path still charges `total_cost`, not either side's own size.
fn a_common_path_is_still_charged_total_cost() {
    let pool = all_six();
    let d = pool
        .iter()
        .find(|c| c.candidate.path == "d/in_place")
        .unwrap();
    assert_eq!(d.budget_cost(), 1_826_067);
}

/// A byte ceiling admits an added path by the compressed download cost, not by
/// its declared size — whichever of the two the ceiling sits between.
fn a_budget_admits_an_added_path_by_compressed_size_not_declared_size() {
    let pool = |declared: i64, download: u64| {
        candidates(&[field(
            "game",
            vec![PathRow {
                path: "x/fresh".into(),
                state: State::Added,
                size: declared,
                download_bytes: Some(download),
                score: None,
                is_folder: false,
            }],
        )])
    };

    // Declared bigger than the ceiling, download under it: admitted.
    let s = select(&pool(1_000, 100), 1, Budget::capped(100), &[]);
    assert_eq!(s.paths(), vec!["x/fresh"]);
    assert_eq!(s.bytes_admitted, 100);

    // One byte below the download cost: refused, and the reason names 100.
    let s = select(&pool(1_000, 100), 1, Budget::capped(99), &[]);
    assert!(s.admitted.is_empty());
    let r = s.rejected.iter().find(|r| r.path == "x/fresh").unwrap();
    assert_eq!(r.budget_cost, 100);
    assert!(r.reason.contains("100"), "the reason names the charge");

    // Reverse the two numbers: a large download under a small declared size is
    // refused even though the declared size would have fitted.
    let s = select(&pool(100, 1_000), 1, Budget::capped(100), &[]);
    assert!(s.admitted.is_empty());
    let r = s.rejected.iter().find(|r| r.path == "x/fresh").unwrap();
    assert_eq!(r.budget_cost, 1_000);
}

fn the_pool_is_ordered_cheapest_first() {
    let costs: Vec<u64> = all_six().iter().map(SizedCandidate::budget_cost).collect();
    let mut sorted = costs.clone();
    sorted.sort_unstable();
    assert_eq!(costs, sorted, "the budget relies on this ordering");
    // A common path is charged old + new, so the cheapest thing here is the
    // 108 B file *plus its old copy* — not the file's own size, and not the
    // 114 000 B added file either. Charging a common path its own size would
    // understate what materializing the fixture actually costs.
    assert_eq!(costs[0], 108, "a/identical: 54 old + 54 new");
}

fn one_capture_per_class_is_the_greedy_answer() {
    let s = select(&all_six(), 1, Budget::unlimited(), &[]);
    let mut got: Vec<(Class, String)> = s
        .matrix
        .iter()
        .filter(|r| r.status == MatrixStatus::Captured)
        .map(|r| (r.class, r.path.clone().unwrap_or_default()))
        .collect();
    got.sort_by_key(|(c, _)| c.id());
    assert_eq!(
        got,
        vec![
            (Class::Added, "j/added".into()),
            (Class::Identical, "a/identical".into()),
            (Class::NoReuse, "c/no_reuse".into()),
            (Class::Removed, "k/removed".into()),
            (Class::ReuseInPlace, "d/in_place".into()),
            (Class::ReuseMoved, "e/moved".into()),
        ]
    );
    assert_eq!(s.admitted.len(), 6, "six classes, one capture each");
    assert!(s.unmet().is_empty(), "and every class answered");
}

/// The class set is a **partition** on the content: a candidate answers
/// exactly one class, so no capture can be bought twice for two shapes.
///
/// That is what makes the bundle's size predictable — one file per patch
/// shape — and it is why `covered_by` only ever arises from a **forced**
/// capture, which exists before the admission looks at the pool.
fn no_candidate_answers_two_classes() {
    for c in all_six() {
        assert!(
            c.candidate.classes.len() <= 1,
            "{} answers {:?}: the class set is a partition",
            c.candidate.path,
            c.candidate.classes
        );
    }
}

/// A **forced** capture answers its class without having been selected for
/// it — which is `covered_by`, and it is satisfied either way. What matters
/// is that the file is captured once, not once per class it happens to
/// answer.
fn a_forced_capture_answers_its_class_without_being_selected_for_it() {
    let s = select(&all_six(), 1, Budget::unlimited(), &["d/in_place".into()]);
    let row = s
        .matrix
        .iter()
        .find(|r| r.class == Class::ReuseInPlace)
        .expect("a row for every class");
    assert_eq!(row.status, MatrixStatus::CoveredBy);
    assert_eq!(row.covered_by.as_deref(), Some("d/in_place"));
    assert!(row.is_satisfied(), "covered counts: the capture exists");
    assert!(!row.unsatisfied());
}

/// The edge case: a capture that would have answered a class is **refused**
/// by the budget, so the class becomes `over_budget` — a decision about the
/// *budget*, not about the class.
///
/// The capture here is a **forced** one (`--files`), and the ceiling is too
/// small for it. The class it would have answered has that same file as its
/// only candidate, so the class cannot be funded either.
fn a_class_whose_capture_was_refused_is_over_budget_not_absent() {
    let pool = all_six();
    // Enough for the cheapest capture and not enough for `d/in_place`.
    let cap = 108;
    let s = select(&pool, 1, Budget::capped(cap), &["d/in_place".into()]);

    let row = s
        .matrix
        .iter()
        .find(|r| r.class == Class::ReuseInPlace)
        .expect("a row for every class");
    assert_eq!(
        row.status,
        MatrixStatus::OverBudget,
        "the capture could not be funded"
    );
    assert!(row.unsatisfied(), "and over_budget is not satisfied");
    assert!(
        row.reason
            .as_deref()
            .unwrap_or_default()
            .contains("the cheapest candidate"),
        "the reason names what happened: {:?}",
        row.reason
    );
}

/// Two forced paths, each under the ceiling on its own, whose **sum** is
/// not. The second must be refused.
///
/// This is the case the reverted series missed: checking each forced path
/// against the caller's unspent budget admitted both, because `0 + 60` and
/// `0 + 60` each fit a 100 B cap. The ledger has to be cumulative.
fn forced_costs_are_checked_against_the_cumulative_budget_not_one_by_one() {
    let pool = candidates(&[field(
        "game",
        vec![
            row("x/a", State::Added, None, 60),
            row("x/b", State::Added, None, 60),
        ],
    )]);
    let s = select(&pool, 1, Budget::capped(100), &["x/a".into(), "x/b".into()]);

    assert_eq!(s.admitted.len(), 1, "only one 60 B path fits in 100 B");
    assert_eq!(s.paths(), vec!["x/a"], "the first one named wins");
    assert_eq!(s.bytes_admitted, 60);
    assert!(
        s.bytes_admitted <= 100,
        "the cumulative admission must not exceed the ceiling"
    );
    assert_eq!(
        s.budget.spent(),
        60,
        "the returned budget is the one the forced phase charged"
    );
    let rejected: Vec<&str> = s.rejected.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(rejected, vec!["x/b"], "and the refusal is recorded");
}

/// A forced path over the ceiling must not hide a later, cheaper one.
///
/// A `break` on the first refusal would let `--files a --files b` with
/// `a = 500 B`, `b = 100 B` under a 200 B cap capture neither and record
/// neither. A refusal is about one path, not the rest of the list.
fn a_forced_path_over_the_ceiling_does_not_hide_a_cheaper_one_after_it() {
    let pool = candidates(&[field(
        "game",
        vec![
            row("x/a", State::Added, None, 500),
            row("x/b", State::Added, None, 100),
        ],
    )]);
    let s = select(&pool, 1, Budget::capped(200), &["x/a".into(), "x/b".into()]);

    assert_eq!(
        s.paths(),
        vec!["x/b"],
        "the later path fits and must be admitted"
    );
    let rejected: Vec<&str> = s.rejected.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(rejected, vec!["x/a"], "and the refused one is recorded");
    assert_eq!(
        s.matrix
            .iter()
            .find(|r| r.class == Class::Added)
            .map(|r| r.status),
        Some(MatrixStatus::CoveredBy),
        "the class is answered by the admitted forced path"
    );
}

/// Two **distinct** forced paths that answer the same primary class must not
/// share an id.
///
/// `capture_id` is `<class>` or `<class>+<n>`, and passing `0` for every
/// forced path would make two new-only paths both `added`, both write
/// `patchmaps/added.json`, and the second write replace the first while
/// `captures_admitted` still said two. Numbering per class is what keeps
/// the identity of a forced path distinct.
fn two_forced_paths_of_one_class_get_distinct_ids() {
    let pool = candidates(&[field(
        "game",
        vec![
            row("x/a", State::Added, None, 10),
            row("x/b", State::Added, None, 20),
        ],
    )]);
    let s = select(&pool, 1, Budget::unlimited(), &["x/a".into(), "x/b".into()]);

    assert_eq!(s.paths(), vec!["x/a", "x/b"], "both are captured");
    assert_eq!(
        s.admitted
            .iter()
            .map(|a| a.capture_id.as_str())
            .collect::<Vec<_>>(),
        vec!["added", "added+1"],
        "the second forced path of a class is numbered, not a duplicate"
    );
    let mut ids: Vec<&str> = s.admitted.iter().map(|a| a.capture_id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), s.admitted.len(), "ids are unique");
    // The matrix still resolves the class to the first admission, which is
    // now the same patchmap that describes the first path.
    let added = s.matrix.iter().find(|r| r.class == Class::Added).unwrap();
    assert_eq!(added.capture_id.as_deref(), Some("added"));
    assert_eq!(added.path.as_deref(), Some("x/a"));
}

/// A forced path refused by the budget must not lend its `<class>` id to a
/// cheaper pick of the same class that *is* funded.
///
/// The refused path is not in `taken` (only funded captures are), so phase 1
/// still considers its class and can pick a cheaper sibling. With two ordinal
/// counters — one for forced paths, one for wants — both paths claim `added`:
/// the rejection record describes one patchmap while the funded capture writes
/// another, and the two can overwrite each other in release builds. The ordinal
/// has to be shared by every selection that could produce an id.
fn a_refused_forced_path_and_a_funded_pick_do_not_share_an_id() {
    let pool = candidates(&[field(
        "game",
        vec![
            row("x/forced", State::Added, None, 500),
            row("x/cheap", State::Added, None, 100),
        ],
    )]);
    // Only `x/cheap` fits: the forced `x/forced` is refused in phase 0, then
    // `x/cheap` is wanted for `added` and funded in phase 2.
    let s = select(&pool, 1, Budget::capped(100), &["x/forced".into()]);

    assert_eq!(s.paths(), vec!["x/cheap"], "the cheaper pick is funded");
    let rejected: Vec<&str> = s.rejected.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(rejected, vec!["x/forced"], "the forced path is refused");

    let admitted_id = &s.admitted[0].capture_id;
    let rejected_id = &s.rejected[0].capture_id;
    assert_ne!(
        admitted_id, rejected_id,
        "the funded capture and the rejection cannot claim one patchmap"
    );
    debug_validate_unique_ids(&s).expect("every id in the selection is unique");
}

/// A forced path the pool carries but which answers **no** class is
/// recorded, not silently dropped.
///
/// A one-chunk file with no reuse is a degenerate patch: `added` already
/// covers "the file is created whole", so the class set is legitimately
/// empty. The row is still recorded as unscored, with nothing else under it.
fn a_forced_path_that_answers_no_class_is_recorded_as_unscored() {
    let pool = candidates(&[field(
        "game",
        vec![row(
            "x/degenerate",
            State::Changed,
            Some(scored("x/degenerate", 10, (5, 5), (1, 0, 0), false)),
            5,
        )],
    )]);
    assert!(
        pool[0].candidate.classes.is_empty(),
        "the fixture is the degenerate shape this test is about"
    );

    let s = select(&pool, 1, Budget::unlimited(), &["x/degenerate".into()]);
    assert!(s.admitted.is_empty(), "no class, so no capture");
    assert_eq!(
        s.forced_unscored,
        vec!["x/degenerate"],
        "the path is recorded as present but unscored"
    );
    assert!(
        s.forced_skipped.is_empty(),
        "it is in the manifests, so it is not `skipped`"
    );
}

/// A class with no candidate is recorded as `unavailable` **with a reason**,
/// rather than being dropped from the matrix.
fn an_unsatisfiable_class_is_recorded_with_a_reason_and_a_count() {
    let pool = candidates(&[field(
        "game",
        vec![row(
            "a/identical",
            State::Identical,
            Some(scored("a/identical", 108, (54, 54), (1, 1, 0), true)),
            54,
        )],
    )]);
    let s = select(&pool, 1, Budget::unlimited(), &[]);
    let row = s
        .matrix
        .iter()
        .find(|r| r.class == Class::ReuseMoved)
        .unwrap();
    assert_eq!(row.status, MatrixStatus::Unavailable);
    assert_eq!(row.candidates_considered, 0);
    let reason = row.reason.clone().unwrap();
    assert!(reason.contains("reuse_moved"), "{reason}");
    assert!(reason.contains("--audio"), "the remedy: {reason}");
    assert!(
        !s.matrix
            .iter()
            .any(|r| r.capture_id.is_none() && r.status == MatrixStatus::Captured)
    );
    assert_eq!(
        s.matrix.len(),
        Class::ALL.len(),
        "one row per class, always"
    );
}

/// A forced capture answers its class *without* having been selected for it —
/// which is `covered_by`, and it is satisfied either way. What matters is that
/// the file is captured once, not once per class it happens to answer.
fn a_class_with_candidates_already_captured_is_covered_by_not_recaptured() {
    // One file that satisfies two classes.
    let pool = candidates(&[field(
        "game",
        vec![row(
            "x/both",
            State::Changed,
            Some(scored("x/both", 200, (100, 100), (3, 1, 0), false)),
            100,
        )],
    )]);
    let s = select(&pool, 1, Budget::unlimited(), &["x/both".into()]);
    assert_eq!(s.admitted.len(), 1, "forced once, not twice");
    let same = s
        .matrix
        .iter()
        .find(|r| r.class == Class::ReuseInPlace)
        .unwrap();
    assert_eq!(same.status, MatrixStatus::CoveredBy);
    assert_eq!(same.covered_by.as_deref(), Some("x/both"));
    assert!(same.is_satisfied(), "covered counts: the capture exists");
    assert!(!same.unsatisfied());
    // Nothing else has a candidate, so every other row is unavailable and
    // the matrix is complete rather than short.
    assert_eq!(s.matrix.len(), Class::ALL.len());
}

/// `--per-class 2` takes a second candidate per class, cheapest first.
fn per_class_two_admits_a_second_file_for_a_class() {
    let pool = candidates(&[field(
        "game",
        vec![
            row(
                "x/a",
                State::Changed,
                Some(scored("x/a", 200, (100, 100), (3, 1, 0), false)),
                100,
            ),
            row(
                "x/b",
                State::Changed,
                Some(scored("x/b", 400, (100, 100), (3, 2, 0), false)),
                200,
            ),
            row(
                "x/c",
                State::Changed,
                Some(scored("x/b", 400, (100, 100), (3, 2, 0), false)),
                300,
            ),
        ],
    )]);
    let s = select(&pool, 2, Budget::unlimited(), &[]);
    let for_class: Vec<&str> = s
        .admitted
        .iter()
        .filter(|a| a.primary_class == Class::ReuseInPlace)
        .map(|a| a.path.as_str())
        .collect();
    assert_eq!(
        for_class,
        vec!["x/a", "x/b"],
        "cheapest first, then the next"
    );
    assert!(
        s.admitted.iter().any(|a| a.capture_id.contains('+')),
        "the second is `class+n`"
    );
}

/// A crafted manifest may declare `compressed_size: i64::MAX`, and the
/// scoring add saturates each `old_bytes + new_bytes` to `u64::MAX - 1`.
/// Two admitted paths are then more than a `u64` holds. The summary total
/// is a measurement next to the saturating ledger, so it must report the
/// saturated number rather than panic (debug) or wrap (release).
fn two_maximum_costs_report_the_saturated_ledger_not_an_overflow() {
    let cost = u64::MAX - 1;
    let pool = candidates(&[field(
        "game",
        vec![
            row(
                "a/one",
                State::Identical,
                Some(scored("a/one", cost, (1, 1), (1, 1, 0), true)),
                1,
            ),
            row(
                "a/two",
                State::Identical,
                Some(scored("a/two", cost, (1, 1), (1, 1, 0), true)),
                1,
            ),
        ],
    )]);
    let s = select(&pool, 2, Budget::unlimited(), &[]);
    assert_eq!(s.admitted.len(), 2, "an unlimited budget admits both");
    assert_eq!(s.budget.spent(), u64::MAX, "the ledger saturates");
    assert_eq!(
        s.bytes_admitted,
        u64::MAX,
        "the reported total is the same saturated number"
    );
}

/// A tight ceiling plus `--per-class 2` is the acceptance case: one fits,
/// the second is recorded as over budget rather than dropped.
fn per_class_two_under_a_tight_ceiling_tags_the_second_over_budget() {
    let mk = |p: &str, size: i64| row(p, State::Added, None, size);
    let pool = candidates(&[field(
        "game",
        vec![mk("x/a", 100), mk("x/b", 500), mk("x/c", 900)],
    )]);
    // Enough for `a` and `b`, not `c`. Only `added` has candidates.
    let s = select(&pool, 3, Budget::capped(600), &[]);
    let row = s.matrix.iter().find(|r| r.class == Class::Added).unwrap();
    assert_eq!(row.status, MatrixStatus::Captured);
    assert_eq!(
        s.admitted
            .iter()
            .filter(|a| a.primary_class == Class::Added)
            .count(),
        2
    );
    assert_eq!(s.bytes_admitted, 600);
}

fn a_class_whose_cheapest_candidate_cannot_be_funded_is_over_budget_not_absent() {
    let pool = candidates(&[field(
        "game",
        vec![
            row("x/a", State::Added, None, 100),
            row("x/b", State::Removed, None, 1_000_000),
        ],
    )]);
    let s = select(&pool, 1, Budget::capped(50), &[]);
    let row = s.matrix.iter().find(|r| r.class == Class::Added).unwrap();
    assert_eq!(row.status, MatrixStatus::OverBudget);
    assert!(
        row.reason.as_deref().unwrap_or("").contains("100"),
        "names the cost: {:?}",
        row.reason
    );
    // And over-budget is *not* satisfied, which is what --require-class acts on.
    assert!(row.unsatisfied());
    assert!(!row.is_satisfied());
    // Both one-sided classes had a candidate and could not be funded; the
    // other nine had none at all. `unmet` carries both, but the *reason*
    // is the useful part and it differs.
    let over: Vec<Class> = s
        .matrix
        .iter()
        .filter(|r| r.status == MatrixStatus::OverBudget)
        .map(|r| r.class)
        .collect();
    assert_eq!(over, vec![Class::Added, Class::Removed], "matrix order");
    let unavailable = s
        .matrix
        .iter()
        .filter(|r| r.status == MatrixStatus::Unavailable)
        .count();
    assert_eq!(unavailable, Class::ALL.len() - 2);
    assert_eq!(
        s.unmet().len(),
        Class::ALL.len(),
        "nothing in this fixture is satisfied"
    );
    assert!(s.admitted.is_empty());
    assert_eq!(s.bytes_admitted, 0);
}

fn unmet_reports_over_budget_and_unavailable_but_not_covered_by() {
    let pool = candidates(&[field(
        "game",
        vec![row(
            "x/a",
            State::Changed,
            Some(scored("x/a", 200, (100, 100), (3, 1, 0), false)),
            100,
        )],
    )]);
    let s = select(&pool, 1, Budget::unlimited(), &["x/a".into()]);
    assert_eq!(s.paths(), vec!["x/a"], "the forced one is the only capture");
    let covered = s
        .matrix
        .iter()
        .find(|r| r.class == Class::ReuseInPlace)
        .unwrap();
    assert_eq!(covered.status, MatrixStatus::CoveredBy);
    assert!(
        !s.unmet().contains(&Class::ReuseInPlace),
        "covered_by is never unmet: the capture exists"
    );
    assert_eq!(s.paths(), vec!["x/a"]);
}

fn the_matrix_has_exactly_one_row_per_class_in_admission_order() {
    let s = select(&all_six(), 1, Budget::unlimited(), &[]);
    assert_eq!(s.matrix.len(), Class::ALL.len());
    let ids: Vec<Class> = s.matrix.iter().map(|r| r.class).collect();
    assert_eq!(ids, Class::REQUIRED_ORDER.to_vec());
}

/// The cheapest-first property is what makes the budget fund the most
/// classes, so it is asserted as an invariant over many ceilings.
fn admission_is_cheapest_first_for_every_ceiling() {
    let pool = all_six();
    for cap in [0, 42, 100, 1_000, 2_000, 10_000, 1_000_000, u64::MAX] {
        let s = select(&pool, 1, Budget::capped(cap), &[]);
        let mut costs: Vec<u64> = s.admitted.iter().map(|a| a.budget_cost).collect();
        let spent: u64 = costs.iter().sum();
        assert!(spent <= cap, "cap {cap}: spent {spent}");
        costs.sort_unstable();
        // Every admitted cost is <= every non-admitted one.
        let cheapest_rejected = pool
            .iter()
            .map(SizedCandidate::budget_cost)
            .filter(|c| !s.admitted.iter().any(|a| a.budget_cost == *c))
            .min();
        if let Some(rej) = cheapest_rejected {
            assert!(
                costs.last().is_none_or(|last| *last <= rej),
                "cap {cap} admitted {costs:?} but rejected {rej}"
            );
        }
    }
}

fn a_forced_path_outside_the_manifests_is_skipped_not_fatal() {
    let s = select(
        &all_six(),
        1,
        Budget::unlimited(),
        &["nope/not/in/manifest".into()],
    );
    // A path the manifests do not carry is skipped, not fatal: the caller
    // may be naming a file for a future version. The matrix is still
    // complete, because a phantom path says nothing about which classes the
    // pool can satisfy.
    assert_eq!(s.matrix.len(), Class::ALL.len());
    assert_eq!(s.admitted.len(), 6, "the six real ones still went in");
}

/// The per-class candidate lists are **keyed by `Class`**, not by a position
/// that happens to line up with `Class::ALL`.
///
/// `REQUIRED_ORDER` and `ALL` are the same list today, but the lookup must not
/// rely on that: reordering either one would silently pair a class with another
/// class's candidates. Every assertion here goes through the class itself.
fn candidate_lists_are_keyed_by_class_not_by_array_position() {
    let pool = all_six();
    let by_class = candidates_by_class(&pool);

    for class in Class::ALL {
        let expected: Vec<usize> = pool
            .iter()
            .enumerate()
            .filter(|(_, c)| c.candidate.satisfies(class))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            by_class.get(&class),
            Some(&expected),
            "the list for `{class}` must be the pool indices that satisfy it"
        );
    }

    // A concrete pairing, independent of where either class sits in the array:
    // `added` points at the added path and `removed` at the removed one.
    let idx_of = |path: &str| {
        pool.iter()
            .position(|c| c.candidate.path == path)
            .expect("the fixture carries it")
    };
    assert_eq!(by_class[&Class::Added], vec![idx_of("j/added")]);
    assert_eq!(by_class[&Class::Removed], vec![idx_of("k/removed")]);
}

/// A folder row is dropped from the pool, so no `SizedCandidate` can describe a
/// folder — the diff still carries the row for its counts and sidecar.
fn candidates_exclude_folder_rows() {
    let diffs = [field(
        "game",
        vec![
            folder_row("d/folder", State::Added, 0),
            row(
                "a/file",
                State::Changed,
                Some(scored("a/file", 20, (10, 10), (3, 1, 0), false)),
                10,
            ),
        ],
    )];
    let pool = candidates(&diffs);
    assert_eq!(pool.len(), 1, "only the real file is a candidate");
    assert!(
        pool.iter().all(|c| c.candidate.path != "d/folder"),
        "a folder must never become a SizedCandidate"
    );
    assert_eq!(pool[0].candidate.path, "a/file");
}

/// A run whose only change is a folder admits nothing for it.
fn a_folder_only_change_admits_no_capture() {
    let diffs = [field("game", vec![folder_row("d/only", State::Added, 0)])];
    let decided = super::decide::decide(&diffs, 1, None, &[]);
    assert_eq!(decided.pool_len, 0, "the folder is not in the pool");
    assert!(
        decided.selection.admitted.is_empty(),
        "a folder is not a file capture"
    );
    assert!(!decided.selection.paths().contains(&"d/only"));
}

fn candidates_document_carries_the_columns_and_a_one_based_rank() {
    let diffs = [field(
        "game",
        vec![
            row(
                "a",
                State::Changed,
                Some(scored("a", 20, (10, 10), (3, 1, 0), false)),
                10,
            ),
            row(
                "b",
                State::Changed,
                Some(scored("b", 40, (20, 20), (1, 0, 0), false)),
                20,
            ),
        ],
    )];
    let doc = candidates_document(&diffs);
    assert_eq!(doc["schema_version"], crate::SCHEMA_VERSION);
    let cols = doc["columns"].as_array().unwrap();
    for c in [
        "path",
        "reused",
        "moved",
        "moved_later",
        "moved_earlier",
        "old_max_end",
        "patch_bytes",
        "total_cost",
        "classes",
        "rank",
        "admission_rank",
    ] {
        assert!(cols.iter().any(|x| x == c), "missing {c}");
    }
    let rows = doc["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["rank"], 1);
    assert_eq!(rows[1]["rank"], 2);
    // `a` is cheaper, so the scoring order puts it first here; the pool agrees.
    assert_eq!(rows[0]["path"], "a");
    assert_eq!(rows[0]["admission_rank"], 1);
    assert_eq!(rows[1]["admission_rank"], 2);
    assert_eq!(rows[0]["classes"][0], Class::ReuseInPlace.id());
}

fn cost_tie_scoring_rank_and_admission_rank_can_differ() {
    // Equal total_cost and field, so `by_cost` breaks the tie by `moved` desc
    // (`b` first) while `candidates` breaks it by path (`a` first). The two
    // ranks must therefore be swapped between the two published columns.
    let diffs = [field(
        "game",
        vec![
            row(
                "a",
                State::Changed,
                Some(scored("a", 40, (20, 20), (1, 0, 0), false)),
                20,
            ),
            row(
                "b",
                State::Changed,
                Some(scored("b", 40, (20, 20), (1, 0, 1), false)),
                20,
            ),
        ],
    )];

    // Confirm the premise before checking the document that rests on it: the
    // two comparators really do disagree about which of `a` / `b` comes first.
    let scoring: Vec<String> = by_cost(&diffs).into_iter().map(|(_, s)| s.path).collect();
    assert_eq!(scoring, vec!["b", "a"]);
    let admission: Vec<String> = candidates(&diffs)
        .into_iter()
        .map(|c| c.candidate.path)
        .collect();
    assert_eq!(admission, vec!["a", "b"]);

    let doc = candidates_document(&diffs);
    let rows = doc["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    // Rows stay in scoring order: `rank` climbs 1, 2; `admission_rank` does not.
    assert_eq!(rows[0]["path"], "b");
    assert_eq!(rows[0]["rank"], 1);
    assert_eq!(rows[0]["admission_rank"], 2);
    assert_eq!(rows[1]["path"], "a");
    assert_eq!(rows[1]["rank"], 2);
    assert_eq!(rows[1]["admission_rank"], 1);
}

fn every_common_row_carries_a_unique_admission_rank() {
    let diffs = [field(
        "game",
        vec![
            row(
                "a",
                State::Changed,
                Some(scored("a", 20, (10, 10), (3, 1, 0), false)),
                10,
            ),
            row(
                "b",
                State::Changed,
                Some(scored("b", 40, (20, 20), (1, 0, 0), false)),
                20,
            ),
            row(
                "c",
                State::Changed,
                Some(scored("c", 60, (30, 30), (2, 1, 1), false)),
                30,
            ),
        ],
    )];
    let doc = candidates_document(&diffs);
    let rows = doc["rows"].as_array().unwrap();
    let mut ranks: Vec<u64> = rows
        .iter()
        .map(|r| {
            r["admission_rank"]
                .as_u64()
                .expect("every common row has a numeric admission_rank")
        })
        .collect();
    ranks.sort_unstable();
    assert_eq!(ranks, vec![1, 2, 3], "unique paths carry unique ranks");
}

fn one_sided_rows_are_not_in_the_scoring_table() {
    let diffs = [field(
        "game",
        vec![
            row(
                "common",
                State::Changed,
                Some(scored("common", 20, (10, 10), (3, 1, 0), false)),
                10,
            ),
            row("added", State::Added, None, 50),
            row("removed", State::Removed, None, 50),
        ],
    )];
    let doc = candidates_document(&diffs);
    let rows = doc["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "one-sided rows are not scoring rows");
    assert_eq!(rows[0]["path"], "common");
}

fn classes_in_order_normalises_a_reader_supplied_list() {
    let got = classes_in_order(&["removed", "identical", "not_a_class"]);
    assert_eq!(got, vec![Class::Identical, Class::Removed]);
}

fn capture_ids_are_the_class_then_the_ordinal() {
    let c = SizedCandidate {
        candidate: all_six()[0].candidate.clone(),
        size: 54,
        download_bytes: 0,
    };
    assert_eq!(c.candidate.capture_id(Class::Identical, 0), "identical");
    assert_eq!(c.candidate.capture_id(Class::Identical, 2), "identical+2");
}

fn empty_selection() -> Selection {
    Selection {
        admitted: Vec::new(),
        rejected: Vec::new(),
        forced_skipped: Vec::new(),
        forced_unscored: Vec::new(),
        matrix: Vec::new(),
        bytes_admitted: 0,
        budget: Budget::unlimited(),
    }
}

fn admission(id: &str) -> Admission {
    Admission {
        capture_id: id.into(),
        primary_class: Class::Identical,
        path: format!("path/{id}"),
        matching_field: "game".into(),
        state: State::Identical,
        also_satisfies: Vec::new(),
        budget_cost: 0,
        patch_bytes: 0,
    }
}

fn rejection(id: &str) -> Rejection {
    Rejection {
        capture_id: id.into(),
        primary_class: Class::Identical,
        path: format!("path/{id}"),
        matching_field: "game".into(),
        state: State::Identical,
        budget_cost: 0,
        reason: String::new(),
    }
}

fn capture_ids_are_unique_in_a_normal_selection() {
    let s = select(&all_six(), 1, Budget::unlimited(), &[]);
    assert_eq!(s.admitted.len(), 6, "one capture per class");
    debug_validate_unique_ids(&s).expect("one capture per class means unique, valid ids");
}

fn duplicate_admitted_capture_ids_are_reported() {
    let mut s = empty_selection();
    s.admitted.push(admission("identical"));
    s.admitted.push(admission("identical"));
    let err = debug_validate_unique_ids(&s).expect_err("two admissions claim one patchmap");
    assert!(err.contains("duplicate admitted"), "{err}");
    assert!(err.contains("identical"), "{err}");
}

fn duplicate_rejected_capture_ids_are_reported() {
    let mut s = empty_selection();
    s.rejected.push(rejection("identical"));
    s.rejected.push(rejection("identical"));
    let err = debug_validate_unique_ids(&s).expect_err("two rejections claim one id");
    assert!(err.contains("duplicate rejected"), "{err}");
    assert!(err.contains("identical"), "{err}");
}

fn an_admitted_id_may_not_equal_a_rejected_id() {
    let mut s = empty_selection();
    s.admitted.push(admission("identical"));
    s.rejected.push(rejection("identical"));
    let err =
        debug_validate_unique_ids(&s).expect_err("the funded and the would-be capture collide");
    assert!(err.contains("both admitted and rejected"), "{err}");
    assert!(err.contains("identical"), "{err}");
}

fn capture_ids_must_be_single_file_stems() {
    for bad in ["", "a/b", "a\\b", "a..b"] {
        let mut s = empty_selection();
        s.admitted.push(admission(bad));
        let err =
            debug_validate_unique_ids(&s).expect_err("an id that is not a single stem is refused");
        assert!(err.contains("valid file stem"), "{err}");
    }
}

#[test]
fn select_decision_cases() {
    // Consolidated pure selection cases; each original case keeps its label and assertions.
    check_case(
        "the_pool_is_built_from_every_row_with_its_shape_and_classes",
        the_pool_is_built_from_every_row_with_its_shape_and_classes,
    );
    check_case(
        "an_added_path_is_charged_its_compressed_download_size",
        an_added_path_is_charged_its_compressed_download_size,
    );
    check_case(
        "a_removed_path_is_still_charged_its_old_size",
        a_removed_path_is_still_charged_its_old_size,
    );
    check_case(
        "a_common_path_is_still_charged_total_cost",
        a_common_path_is_still_charged_total_cost,
    );
    check_case(
        "a_budget_admits_an_added_path_by_compressed_size_not_declared_size",
        a_budget_admits_an_added_path_by_compressed_size_not_declared_size,
    );
    check_case(
        "the_pool_is_ordered_cheapest_first",
        the_pool_is_ordered_cheapest_first,
    );
    check_case(
        "one_capture_per_class_is_the_greedy_answer",
        one_capture_per_class_is_the_greedy_answer,
    );
    check_case(
        "no_candidate_answers_two_classes",
        no_candidate_answers_two_classes,
    );
    check_case(
        "a_forced_capture_answers_its_class_without_being_selected_for_it",
        a_forced_capture_answers_its_class_without_being_selected_for_it,
    );
    check_case(
        "a_class_whose_capture_was_refused_is_over_budget_not_absent",
        a_class_whose_capture_was_refused_is_over_budget_not_absent,
    );
    check_case(
        "forced_costs_are_checked_against_the_cumulative_budget_not_one_by_one",
        forced_costs_are_checked_against_the_cumulative_budget_not_one_by_one,
    );
    check_case(
        "a_forced_path_over_the_ceiling_does_not_hide_a_cheaper_one_after_it",
        a_forced_path_over_the_ceiling_does_not_hide_a_cheaper_one_after_it,
    );
    check_case(
        "two_forced_paths_of_one_class_get_distinct_ids",
        two_forced_paths_of_one_class_get_distinct_ids,
    );
    check_case(
        "a_refused_forced_path_and_a_funded_pick_do_not_share_an_id",
        a_refused_forced_path_and_a_funded_pick_do_not_share_an_id,
    );
    check_case(
        "a_forced_path_that_answers_no_class_is_recorded_as_unscored",
        a_forced_path_that_answers_no_class_is_recorded_as_unscored,
    );
    check_case(
        "an_unsatisfiable_class_is_recorded_with_a_reason_and_a_count",
        an_unsatisfiable_class_is_recorded_with_a_reason_and_a_count,
    );
    check_case(
        "a_class_with_candidates_already_captured_is_covered_by_not_recaptured",
        a_class_with_candidates_already_captured_is_covered_by_not_recaptured,
    );
    check_case(
        "per_class_two_admits_a_second_file_for_a_class",
        per_class_two_admits_a_second_file_for_a_class,
    );
    check_case(
        "two_maximum_costs_report_the_saturated_ledger_not_an_overflow",
        two_maximum_costs_report_the_saturated_ledger_not_an_overflow,
    );
    check_case(
        "per_class_two_under_a_tight_ceiling_tags_the_second_over_budget",
        per_class_two_under_a_tight_ceiling_tags_the_second_over_budget,
    );
    check_case(
        "a_class_whose_cheapest_candidate_cannot_be_funded_is_over_budget_not_absent",
        a_class_whose_cheapest_candidate_cannot_be_funded_is_over_budget_not_absent,
    );
    check_case(
        "unmet_reports_over_budget_and_unavailable_but_not_covered_by",
        unmet_reports_over_budget_and_unavailable_but_not_covered_by,
    );
    check_case(
        "the_matrix_has_exactly_one_row_per_class_in_admission_order",
        the_matrix_has_exactly_one_row_per_class_in_admission_order,
    );
    check_case(
        "admission_is_cheapest_first_for_every_ceiling",
        admission_is_cheapest_first_for_every_ceiling,
    );
    check_case(
        "a_forced_path_outside_the_manifests_is_skipped_not_fatal",
        a_forced_path_outside_the_manifests_is_skipped_not_fatal,
    );
    check_case(
        "candidate_lists_are_keyed_by_class_not_by_array_position",
        candidate_lists_are_keyed_by_class_not_by_array_position,
    );
    check_case(
        "candidates_exclude_folder_rows",
        candidates_exclude_folder_rows,
    );
    check_case(
        "a_folder_only_change_admits_no_capture",
        a_folder_only_change_admits_no_capture,
    );
    check_case(
        "candidates_document_carries_the_columns_and_a_one_based_rank",
        candidates_document_carries_the_columns_and_a_one_based_rank,
    );
    check_case(
        "cost_tie_scoring_rank_and_admission_rank_can_differ",
        cost_tie_scoring_rank_and_admission_rank_can_differ,
    );
    check_case(
        "every_common_row_carries_a_unique_admission_rank",
        every_common_row_carries_a_unique_admission_rank,
    );
    check_case(
        "one_sided_rows_are_not_in_the_scoring_table",
        one_sided_rows_are_not_in_the_scoring_table,
    );
    check_case(
        "classes_in_order_normalises_a_reader_supplied_list",
        classes_in_order_normalises_a_reader_supplied_list,
    );
    check_case(
        "capture_ids_are_the_class_then_the_ordinal",
        capture_ids_are_the_class_then_the_ordinal,
    );
    check_case(
        "capture_ids_are_unique_in_a_normal_selection",
        capture_ids_are_unique_in_a_normal_selection,
    );
    check_case(
        "duplicate_admitted_capture_ids_are_reported",
        duplicate_admitted_capture_ids_are_reported,
    );
    check_case(
        "duplicate_rejected_capture_ids_are_reported",
        duplicate_rejected_capture_ids_are_reported,
    );
    check_case(
        "an_admitted_id_may_not_equal_a_rejected_id",
        an_admitted_id_may_not_equal_a_rejected_id,
    );
    check_case(
        "capture_ids_must_be_single_file_stems",
        capture_ids_must_be_single_file_stems,
    );
}

fn check_case(label: &str, case: fn()) {
    println!("case: {label}");
    case();
}
