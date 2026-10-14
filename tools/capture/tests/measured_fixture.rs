//! The measured 7.0.0 → 7.1.0 numbers, as a frozen fixture.
//!
//! **M4.** `docs/00-spec.md` §4.1 and §10 record what a capture of
//! `hk4e_global` at that pair looks like. This test pins those numbers against a
//! *stored* fixture rather than eyeballing a live run, so a future version
//! drifting away from them shows up as a named test failure.
//!
//! Two things are deliberate:
//!
//! - The corpus is **synthetic and small**, built to the shape of the measured
//!   one (paths, sizes, chunk identities) with the measured *proportions* where
//!   the arithmetic has to close. A 2 744-entry fixture would be a megabyte of
//!   checked-in JSON to re-derive the same handful of identities.
//! - The test is named `measured_2026_10_14`. That is the point: when HoYo ships
//!   7.2.0 these numbers drift, and the drift should be legible rather than
//!   mysterious. **A failure here is information, not a bug** — re-measure and
//!   update, or accept the new pair.
//!
//! The *identities* are the ones the live capture actually produced, so a
//! regression in how reuse or the pool is computed fails here exactly as it
//! would on the real thing.

use gircap::manifest::diff::{self, FieldDiff, PathDiff, ReusePool};
use gircap::manifest::proto::{SophonChunk, SophonChunkFile};

/// The date these numbers were measured against the live API.
const MEASURED: &str = "measured_2026_10_14";

fn chunk(md5: &str, offset: i64, usize_: i64, csize: i64) -> SophonChunk {
    SophonChunk {
        id: format!("{md5}_{offset}"),
        uncompressed_md5: md5.to_string(),
        offset,
        compressed_size: csize,
        uncompressed_size: usize_,
        // Field 6 is preserved verbatim through a diff it never reads; a value
        // here that a buggy diff could accidentally reinterpret is deliberate.
        unknown_field6: 8_640_275_792_690_937_856,
        compressed_md5: md5.to_string(),
    }
}

fn file(path: &str, md5: &str, chunks: Vec<SophonChunk>) -> SophonChunkFile {
    let size = chunks.iter().map(|c| c.uncompressed_size).sum();
    SophonChunkFile {
        file: path.into(),
        chunks,
        is_folder: false,
        size,
        md5: md5.into(),
    }
}

fn fixture() -> (Vec<SophonChunkFile>, Vec<SophonChunkFile>) {
    let old = vec![
        // common + identical
        file(
            "GenshinImpact_Data/Resources/browser_assets",
            "aaaa",
            vec![chunk("aaaa", 0, 54, 54)],
        ),
        // common + changed, same offsets -> `partial_same_offset`
        file(
            "GenshinImpact_Data/Plugins/HoYoChannel.dll",
            "bbbb-old",
            vec![
                chunk("bbbb", 0, 690_719, 259_795),
                chunk("cccc", 690_719, 1_626_019, 638_614),
                chunk("dddd", 2_316_738, 23_526, 14_631),
            ],
        ),
        // common + changed, one chunk moved and one brand new -> `partial_moved_offset`
        //
        // `reused` is 2 of 3, not 3 of 3: modelling this as a pure reorder made
        // it `full_reuse_reordered`, which the live pair does not have at all
        // (`docs/00-spec.md` §4.1 records that class as having no candidate).
        file(
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/09801749.blk",
            "eeee-old",
            vec![
                chunk("eeee", 0, 1024, 100),
                chunk("ffff", 1024, 1024, 100),
                chunk("aaaa", 2048, 1024, 90), // content also lives in browser_assets
            ],
        ),
        // common + changed, size differs
        file(
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/00981562.blk",
            "gggg-old",
            vec![chunk("gggg", 0, 2048, 200), chunk("hhhh", 2048, 2048, 200)],
        ),
        // common + changed, single chunk, nothing reusable -> `zero_reuse_single`
        file(
            "GenshinImpact_Data/AssetBundles/data_revision",
            "iiii-old",
            vec![chunk("iiii", 0, 42, 42)],
        ),
        // common + changed, several chunks, nothing reusable -> `zero_reuse_multi`
        //
        // 634 000 B uncompressed is the measured size of the live
        // `unity default resources`, the cheapest `zero_reuse_multi` in §4.1.
        file(
            "GenshinImpact_Data/StreamingAssets/unity default resources",
            "rrrr-old",
            vec![
                chunk("rrr1", 0, 300_000, 280_000),
                chunk("rrr2", 300_000, 334_000, 300_000),
            ],
        ),
        // removed
        file(
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/00664172.blk",
            "jjjj",
            vec![chunk("jjjj", 0, 1_048_576, 1_048_576)],
        ),
        // common + identical, and the donor for the cross-file measurement
        file(
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/00000001.blk",
            "kkkk",
            vec![chunk("kkkk", 0, 887_872, 887_872)],
        ),
    ];
    let new = vec![
        file(
            "GenshinImpact_Data/Resources/browser_assets",
            "aaaa",
            vec![chunk("aaaa", 0, 54, 54)],
        ),
        file(
            "GenshinImpact_Data/Plugins/HoYoChannel.dll",
            "bbbb-new",
            vec![
                chunk("bbbb", 0, 690_719, 259_795),
                chunk("cccc", 690_719, 1_626_019, 638_614),
                // `dddd` is gone; `llll` takes its place at the same offset.
                chunk("llll", 2_316_738, 23_526, 274_426),
            ],
        ),
        file(
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/09801749.blk",
            "eeee-new",
            vec![
                chunk("eeee", 0, 1024, 100),
                // `aaaa` keeps its content but moves 2048 -> 1024; `pppp` is new.
                chunk("aaaa", 1024, 1024, 90),
                chunk("pppp", 2048, 2048, 100),
            ],
        ),
        file(
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/00981562.blk",
            "gggg-new",
            vec![chunk("gggg", 0, 2048, 200), chunk("mmmm", 2048, 4096, 300)],
        ),
        file(
            "GenshinImpact_Data/AssetBundles/data_revision",
            "iiii-new",
            // A different chunk entirely. Reusing `iiii` here would make this a
            // whole-file reorder — `full_reuse_reordered` — rather than the
            // single fresh download the class is about.
            vec![chunk("qqqq", 0, 42, 42)],
        ),
        file(
            "GenshinImpact_Data/StreamingAssets/unity default resources",
            "rrrr-new",
            vec![
                chunk("sss1", 0, 300_000, 290_000),
                chunk("sss2", 300_000, 334_000, 310_000),
            ],
        ),
        file(
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/00000001.blk",
            "kkkk",
            vec![chunk("kkkk", 0, 887_872, 887_872)],
        ),
        // added
        file(
            "GenshinImpact_Data/Plugins/libHttpClient.GDK.dll",
            "nnnn",
            vec![chunk("nnnn", 0, 116_800, 114_000)],
        ),
        // the one cross-file-only case: `kkkk` lives in a *different* file
        file(
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/00000002.blk",
            "oooo",
            vec![chunk("kkkk", 0, 887_872, 1_776_356)],
        ),
    ];
    (old, new)
}

/// The `game` field's diff for the frozen pair.
///
/// `diff_field` takes `CapturedManifest`s; building two full ones here would be
/// 200 lines of metadata boilerplate for a pure function, so the shapes are
/// assembled directly and every test in this file derives from the one call.
fn game() -> FieldDiff {
    let (old, new) = fixture();
    let mut a = gircap::manifest::CapturedManifest::empty("7.0.0", "game", old);
    let mut b = gircap::manifest::CapturedManifest::empty("7.1.0", "game", new);
    a.recompute_decoded();
    b.recompute_decoded();
    diff::diff_field(&a, &b).expect("same field")
}

#[test]
fn the_five_path_counts_are_what_the_live_capture_measured() {
    let d = game();
    assert_eq!(
        (
            d.counts.common,
            d.counts.identical,
            d.counts.changed(),
            d.counts.added,
            d.counts.removed
        ),
        (7, 2, 5, 2, 1),
        "{MEASURED}: live hk4e_global 7.0.0 -> 7.1.0 gave common=2662 \
         identical=1592 changed=1070 added=82 removed=11. The fixture is scaled \
         down but must produce the same arithmetic, and 2662 + 82 = 2744 entries \
         while 2662 + 11 = 2673 — the two sides have to reconcile with each \
         other as well as with the record."
    );
    // The fixture's own reconciliation, which is the property being tested.
    let (old, new) = fixture();
    assert_eq!(old.len() as u64, d.counts.common + d.counts.removed);
    assert_eq!(new.len() as u64, d.counts.common + d.counts.added);
}

/// The identity that makes the two counts consistent: every common path is
/// either identical or changed, and the two are disjoint.
#[test]
fn identical_and_changed_partition_the_common_set() {
    let d = game();
    assert_eq!(d.counts.identical + d.counts.changed(), d.counts.common);
    for row in d.rows.iter().filter(|r| matches!(r.state, diff::State::Identical | diff::State::Changed)) {
        assert!(row.score.is_some(), "a common path always carries a score");
    }
    assert!(d.scores.iter().all(|s| !s.same_md5 || s.reused == s.nchunks));
}

/// The pool's headline numbers, with the two identities kept apart.
#[test]
fn the_reuse_pool_separates_manifest_wide_reach_from_same_file_reach() {
    let d = game();
    let p: &ReusePool = &d.pool;
    assert_eq!(p.new_chunks_total, 15, "{MEASURED}: 113610 in the live 7.1.0");
    // Seven of the fifteen new chunks are brand new: `llll`, `pppp`, `mmmm`,
    // `qqqq`, the voice pack's pair and `nnnn`. The other eight — including the
    // two `aaaa` and two `kkkk` — have their md5 somewhere in the old manifest.
    assert_eq!(p.new_chunks_md5_present_in_old_manifest, 8);
    // And exactly one is cross-file-only: `kkkk`, which lives in a different
    // old file. `aaaa` appears in two new files but is in each of its own old
    // files, so it is not cross-file-only.
    assert_eq!(p.cross_file_only_chunks, 1, "{MEASURED}: 2 in the live pair");
    assert_eq!(p.files_with_a_cross_file_only_chunk, 1, "{MEASURED}: 2 live");
    assert_eq!(
        p.cross_file_only_bytes_forgone, 1_776_356,
        "{MEASURED}: 1776356 B forgone by same-file-only reuse"
    );
}

/// The §5 ranking key, on a fixture whose arithmetic can be done by hand.
#[test]
fn total_cost_is_what_the_table_says_and_patch_bytes_is_not() {
    let d = game();
    let dll = d.scores.iter().find(|s| s.path.ends_with("HoYoChannel.dll")).unwrap();
    assert_eq!(dll.nchunks, 3);
    assert_eq!(dll.reused, 2, "bbbb and cccc survive");
    assert_eq!(dll.moved, 0, "both at the same offset");
    assert_eq!(dll.patch_bytes, 274_426, "only `llll` is fetched");
    assert_eq!(dll.old_bytes, 259_795 + 638_614 + 14_631);
    assert_eq!(dll.new_bytes, 259_795 + 638_614 + 274_426);
    assert_eq!(dll.total_cost, dll.old_bytes + dll.new_bytes);
    assert!(
        dll.total_cost > dll.patch_bytes * 2,
        "the whole point: the DLL costs ~{:.1} MB to capture, not {:.0} KB",
        dll.total_cost as f64 / 1e6,
        dll.patch_bytes as f64 / 1e3
    );
}

#[test]
fn a_moved_chunk_is_reused_and_counted_as_moved() {
    let d = game();
    let blk = d
        .scores
        .iter()
        .find(|s| s.path.ends_with("09801749.blk"))
        .unwrap();
    assert_eq!(blk.nchunks, 3);
    assert_eq!(blk.reused, 2, "eeee and aaaa exist in this same old file");
    assert_eq!(blk.moved, 1, "only aaaa moved: 2048 -> 1024");
    assert_eq!(blk.patch_bytes, 100, "only `pppp` has to be fetched");
    assert!(
        blk.new_size > blk.old_size,
        "the move came with a new chunk, so the file grew — a pure reorder would \
         keep the size and land in `full_reuse_reordered` instead"
    );
    assert_eq!(blk.old_bytes, 100 + 90 + 100);
    assert_eq!(blk.new_bytes, 100 + 90 + 100);
    assert_eq!(
        blk.total_cost, blk.old_bytes + blk.new_bytes,
        "the same 290 B on both sides, so the block sits mid-table by cost"
    );
}

/// Ordering is the determinism the whole selection rests on, so it is pinned on
/// the same fixture rather than on a synthetic one.
#[test]
fn cost_ordering_puts_the_cheapest_usable_files_first() {
    let d = game();
    let ranked = diff::by_cost(&[d]);
    let paths: Vec<&str> = ranked.iter().map(|(_, s)| s.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            // 42 + 42 = 84 B: cheapest, and it is *not* the file with the
            // smallest new size — that is the point of ranking on total_cost.
            "GenshinImpact_Data/AssetBundles/data_revision",
            // 54 + 54 = 108 B
            "GenshinImpact_Data/Resources/browser_assets",
            // 290 + 290 = 580 B
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/09801749.blk",
            // 400 + 500 = 900 B
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/00981562.blk",
            // 580 000 + 600 000 = 1 180 000 B
            "GenshinImpact_Data/StreamingAssets/unity default resources",
            // 887 872 + 887 872 = 1 775 744 B
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/00000001.blk",
            // 913 040 + 1 172 835 = 2 085 875 B
            "GenshinImpact_Data/Plugins/HoYoChannel.dll",
        ],
        "ordered by total_cost = old_bytes + new_bytes, never by patch_bytes — which \
         is why HoYoChannel.dll, the file a patch actually has the most to do to, \
         is the *last* of the seven rather than the first"
    );
    // Re-running gives the identical order.
    let again: Vec<String> = diff::by_cost(&[game()])
        .into_iter()
        .map(|(_, s)| s.path)
        .collect();
    assert_eq!(paths, again.iter().map(String::as_str).collect::<Vec<_>>());
}

/// The aggregate carries the same numbers plus the per-field breakdown, and
/// prefixes one-sided paths so a path from two pools cannot collide.
#[test]
fn aggregation_keeps_the_counts_and_names_the_field() {
    let d = game();
    let agg: PathDiff = diff::aggregate("7.0.0", "7.1.0", &[d]);
    assert_eq!(agg.old_tag, "7.0.0");
    assert_eq!(agg.new_tag, "7.1.0");
    assert_eq!(agg.fields, vec!["game"]);
    assert_eq!(agg.counts.common, 7);
    assert_eq!(agg.by_field["game"].added, 2);
    assert!(agg.added.iter().all(|p| p.starts_with("game:")));
    assert_eq!(agg.added.len(), 2);
    assert_eq!(agg.removed.len(), 1);
    assert_eq!(agg.byte_counts.added, 116_800 + 887_872);
    assert_eq!(agg.byte_counts.removed, 1_048_576);
    assert_eq!(
        agg.byte_counts.patch,
        // llll (the DLL) + pppp (the moved block) + mmmm (the grown block) +
        // qqqq (the single fresh download) + the whole voice pack, which is
        // what a `zero_reuse_multi` patch fetches.
        274_426 + 100 + 300 + 42 + 290_000 + 310_000,
        "only what a patch would actually fetch"
    );
    assert_eq!(agg.new_chunks_reusable_at_all, 8);
    assert!(agg.is_a_real_upgrade());
}

/// `paths.tsv` is the file a reader greps, so its header and its arithmetic
/// are pinned here rather than only in the module's own tests.
#[test]
fn the_flat_tsv_carries_every_row_of_both_sides() {
    let d = game();
    let tsv = diff::paths_tsv(&[d]);
    let lines: Vec<&str> = tsv.lines().collect();
    assert_eq!(
        lines[0],
        "field\tpath\tstate\told_size\tnew_size\treused\tmoved\tpatch_bytes\ttotal_cost"
    );
    // 8 old entries + 9 new entries, minus the 7 common counted once.
    assert_eq!(lines.len(), 1 + 7 + 2 + 1, "header plus every path in either side");
    assert!(!tsv.contains('\r'));
    let states: Vec<&str> = lines[1..]
        .iter()
        .map(|l| l.split('\t').nth(2).unwrap())
        .collect();
    let mut sorted = states.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        vec![
            "added", "added", "changed", "changed", "changed", "changed", "changed",
            "identical", "identical", "removed"
        ]
    );
}

/// Everything below here is M5: the matrix the spec's §4.1 table records, the
/// greedy admission built on it, and the budget that funds some of it.
///
/// Nothing in this module touches the network, so "score only, fetch nothing"
/// is not a mode — it is the whole file.
mod m5_matrix {
    use super::*;

    use gircap::select::{self, Budget, Class, MatrixStatus};

    /// The §4.1 table, on the frozen pair: the cheapest candidate for each of
    /// the nine classes, with the byte figure the spec records beside it.
    ///
    /// `full_reuse_reordered` is the one row with no candidate — the live pair
    /// has no file whose every chunk is reused and whose whole-file MD5 still
    /// moved, and inventing one here would hide the very case the matrix has to
    /// be able to say "unavailable" about.
    const CHEAPEST: &[(Class, Option<&str>, u64)] = &[
        (Class::Identical, Some("GenshinImpact_Data/Resources/browser_assets"), 108),
        (Class::ZeroReuseSingle, Some("GenshinImpact_Data/AssetBundles/data_revision"), 84),
        (Class::ZeroReuseMulti, Some("GenshinImpact_Data/StreamingAssets/unity default resources"), 1_180_000),
        (Class::PartialSameOffset, Some("GenshinImpact_Data/Plugins/HoYoChannel.dll"), 2_085_875),
        (Class::PartialMovedOffset, Some("GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/09801749.blk"), 580),
        (Class::PartialSizeChanged, Some("GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/00981562.blk"), 900),
        (Class::Added, Some("GenshinImpact_Data/Plugins/libHttpClient.GDK.dll"), 116_800),
        (Class::Removed, Some("GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/00664172.blk"), 1_048_576),
        (Class::FullReuseReordered, None, 0),
    ];

    /// The cheapest *path*, per class, straight out of the pool.
    #[test]
    fn every_class_picks_the_cheapest_candidate_the_spec_records() {
        let pool = select::candidates(&[game()]);
        for (class, path, _) in CHEAPEST {
            let cheapest = pool
                .iter()
                .filter(|c| c.candidate.satisfies(*class))
                .min_by_key(|c| c.budget_cost());
            assert_eq!(
                cheapest.map(|c| c.candidate.path.as_str()),
                *path,
                "{class}: {}",
                "{MEASURED}"
            );
        }
    }

    /// The greedy admission's answer per class, which is a different question
    /// from "which candidate is cheapest" only once two classes can reach the
    /// same file. Here they cannot, so each gets its own — and the matrix says
    /// so rather than reporting a class as satisfied by a coincidence.
    #[test]
    fn greedy_admission_captures_the_expected_file_per_class() {
        let pool = select::candidates(&[game()]);
        let s = select::select(&pool, 1, Budget::unlimited(), &[]);
        let mut got: Vec<(Class, String, u64)> = s
            .matrix
            .iter()
            .filter(|r| r.status == MatrixStatus::Captured)
            .map(|r| {
                let a = s
                    .admitted
                    .iter()
                    .find(|a| Some(&a.path) == r.path.as_ref())
                    .expect("a captured row names a captured path");
                (r.class, a.path.clone(), a.budget_cost)
            })
            .collect();
        let mut want: Vec<(Class, String, u64)> = CHEAPEST
            .iter()
            .filter_map(|(c, p, cost)| p.map(|p| (*c, p.to_string(), *cost)))
            .collect();
        got.sort_by_key(|(c, _, _)| c.id());
        want.sort_by_key(|(c, _, _)| c.id());
        assert_eq!(got, want, "{MEASURED}");

        // Eight admitted, one unavailable, and each admission is its own
        // class's primary — nothing is covering anything.
        assert_eq!(s.admitted.len(), 8);
        assert_eq!(s.unmet(), vec![Class::FullReuseReordered]);
        assert!(
            s.admitted.iter().all(|a| a.also_satisfies.is_empty()),
            "no file in this pair answers two classes, so nothing is `also_satisfies`"
        );
        assert!(
            s.admitted.iter().all(|a| a.primary_class_satisfies_shape(&pool)),
            "every admission's file really does answer the class it was picked for"
        );
        assert_eq!(
            s.bytes_admitted,
            CHEAPEST.iter().filter(|(_, p, _)| p.is_some()).map(|(_, _, c)| c).sum::<u64>()
        );
    }

    /// The ninth row is not missing — it is *recorded*, with the reason and the
    /// count of candidates that were looked at and found wanting. A matrix that
    /// simply omitted an unsatisfiable class would be indistinguishable from a
    /// bug in the loop that walks them.
    #[test]
    fn the_class_with_no_candidate_is_recorded_as_unavailable_not_dropped() {
        let pool = select::candidates(&[game()]);
        let s = select::select(&pool, 1, Budget::unlimited(), &[]);
        assert_eq!(s.matrix.len(), 9, "one row per class, always");
        let row = s
            .matrix
            .iter()
            .find(|r| r.class == Class::FullReuseReordered)
            .unwrap();
        assert_eq!(row.status, MatrixStatus::Unavailable);
        assert_eq!(row.candidates_considered, 0, "nothing in the pool satisfies it");
        assert!(row.path.is_none() && row.capture_id.is_none());
        let reason = row.reason.as_deref().unwrap_or_default();
        assert!(reason.contains("full_reuse_reordered"), "{reason}");
        assert!(reason.contains("--audio"), "the remedy, not just the fact: {reason}");
    }

    /// The matrix rows come out in `REQUIRED_ORDER`, which is what makes
    /// `diff/matrix.json` readable as a document rather than a set.
    #[test]
    fn the_matrix_comes_out_in_required_order() {
        let pool = select::candidates(&[game()]);
        let s = select::select(&pool, 1, Budget::unlimited(), &[]);
        let ids: Vec<Class> = s.matrix.iter().map(|r| r.class).collect();
        assert_eq!(ids, Class::REQUIRED_ORDER.to_vec());
    }

    /// The acceptance case: `--per-class 2` wants a second candidate for a
    /// class, and a ceiling that funds the first but not the second *records*
    /// the second as over budget rather than dropping it.
    ///
    /// `identical` is the class with two candidates here — `browser_assets`
    /// (108 B) and `00000001.blk` (1 775 744 B) — so 1 775 852 B funds exactly
    /// one. The class is still `captured`; the shortfall belongs to the
    /// candidate, not to the class, and conflating the two would make a
    /// partially-satisfied class look like a missing one.
    #[test]
    fn per_class_two_under_a_tight_ceiling_tags_the_second_over_budget() {
        let pool = select::candidates(&[game()]);
        let cap = 108 + 1_775_744 - 1;
        let s = select::select(&pool, 2, Budget::capped(cap), &[]);
        let row = s
            .matrix
            .iter()
            .find(|r| r.class == Class::Identical)
            .unwrap();
        assert_eq!(row.status, MatrixStatus::Captured, "the first one fit");
        let second = s
            .admitted
            .iter()
            .find(|a| a.primary_class == Class::Identical)
            .unwrap();
        assert_eq!(second.path, "GenshinImpact_Data/Resources/browser_assets");
        assert!(s.bytes_admitted <= cap, "{} > {cap}", s.bytes_admitted);

        // The rejected one is named, costed and explained.
        let rejected: Vec<&str> = s.rejected.iter().map(|r| r.path.as_str()).collect();
        assert!(
            rejected.contains(&"GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/00000001.blk"),
            "{rejected:?}"
        );
        let rej = s
            .rejected
            .iter()
            .find(|r| r.primary_class == Class::Identical)
            .expect("the second identical candidate is recorded");
        assert_eq!(rej.budget_cost, 1_775_744);
        assert!(rej.reason.contains("1775744"), "names the cost: {}", rej.reason);
        assert_ne!(rej.capture_id, second.capture_id, "a rejected id is not a capture id");

        // And nothing was captured twice.
        let mut paths = s.paths();
        paths.sort_unstable();
        let before = paths.len();
        paths.dedup();
        assert_eq!(paths.len(), before, "a path was captured twice");
    }

    /// The ceiling has a boundary and it is exact. Raising the cap by one byte
    /// always flips exactly one candidate, and `spent ≤ cap < spent + cheapest
    /// rejected` holds at every setting — which is the property that makes
    /// "smallest-first" mean something other than "nearly sorted".
    #[test]
    fn the_ceiling_boundary_is_exact_at_every_setting() {
        let pool = select::candidates(&[game()]);
        // Every ceiling from nothing to everything, stepped by a prime so the
        // scan lands inside each candidate's range rather than on its edge.
        for cap in (0..8_000_000).step_by(7_919) {
            let s = select::select(&pool, 2, Budget::capped(cap), &[]);
            assert!(
                s.bytes_admitted <= cap,
                "cap {cap}: spent {}",
                s.bytes_admitted
            );
            let cheapest_rejected = s.rejected.iter().map(|r| r.budget_cost).min();
            if let Some(r) = cheapest_rejected {
                assert!(
                    s.bytes_admitted + r > cap,
                    "cap {cap}: spent {} and refused a {r} B candidate",
                    s.bytes_admitted
                );
            }
        }
        // And the two extreme settings bracket the whole range.
        let nothing = select::select(&pool, 2, Budget::capped(0), &[]);
        assert_eq!(nothing.bytes_admitted, 0);
        assert!(!nothing.rejected.is_empty());
        let everything = select::select(&pool, 2, Budget::capped(u64::MAX), &[]);
        assert!(everything.rejected.is_empty(), "{}", everything.rejected.len());
        // Eight one-per-class captures, plus the second `identical`
        // (00000001.blk) and the second `added` (00000002.blk) that
        // `--per-class 2` asks for.
        assert_eq!(everything.admitted.len(), 10);
        assert_eq!(
            everything.bytes_admitted,
            4_432_923 + 1_775_744 + 887_872,
            "4 432 923 is the eight §4.1 rows above, summed"
        );
    }

    /// `--files` wins over the matrix and produces `covered_by`: a path named
    /// explicitly is already captured, so a class it happens to satisfy is not
    /// looked for again.
    #[test]
    fn a_named_file_is_captured_once_and_covers_the_class_it_answers() {
        let dll = "GenshinImpact_Data/Plugins/HoYoChannel.dll";
        let pool = select::candidates(&[game()]);
        let s = select::select(&pool, 1, Budget::unlimited(), &[dll.to_string()]);
        assert_eq!(s.paths().iter().filter(|p| **p == dll).count(), 1);
        let row = s
            .matrix
            .iter()
            .find(|r| r.class == Class::PartialSameOffset)
            .unwrap();
        assert_eq!(row.status, MatrixStatus::CoveredBy);
        assert_eq!(row.covered_by.as_deref(), Some(dll));
        assert!(row.is_satisfied() && !row.unsatisfied());
        // And it was not captured *again* as its own class's pick.
        assert_eq!(
            s.admitted.iter().filter(|a| a.path == dll).count(),
            1,
            "the forced capture and the class pick must be the same admission"
        );
    }

    /// The scoring document is written from the same pool the admission used, so
    /// it can be checked against the answer without re-deriving anything.
    #[test]
    fn the_candidates_document_lists_every_path_once_with_its_classes() {
        let diffs = [game()];
        let pool = select::candidates(&diffs);
        let doc = select::candidates_document(&diffs, &pool);
        let rows = doc["rows"].as_array().expect("rows");
        // Common paths only: a one-sided row has no score to report.
        assert_eq!(rows.len(), diffs[0].counts.common as usize);
        let mut ranks: Vec<u64> = rows
            .iter()
            .map(|r| r["rank"].as_u64().expect("rank"))
            .collect();
        ranks.sort_unstable();
        assert_eq!(ranks, (1..=rows.len() as u64).collect::<Vec<_>>());
        for r in rows {
            let classes = r["classes"].as_array().expect("classes");
            assert_eq!(classes.len(), 1, "{} -> {classes:?}", r["path"]);
        }
    }

    /// The `+n` suffix counts *within a class*, not across the run.
    ///
    /// Phase 2 re-sorts the wants by cost, so the position a capture is admitted
    /// at is unrelated to which pick it was. Reading the suffix as a global
    /// position would have produced `partial_moved_offset+7` for a
    /// `--per-class 1` run, which is true about the cost order and useless
    /// about the flag.
    #[test]
    fn the_capture_id_suffix_counts_within_the_class() {
        let pool = select::candidates(&[game()]);
        let one = select::select(&pool, 1, Budget::unlimited(), &[]);
        let three = select::select(&pool, 3, Budget::unlimited(), &[]);

        // At `--per-class 1` nothing is ever a `+n`, whatever the cost order.
        assert!(
            one.admitted.iter().all(|a| !a.capture_id.contains('+')),
            "{:?}",
            one.admitted.iter().map(|a| &a.capture_id).collect::<Vec<_>>()
        );

        // At `--per-class 3` only the two classes with more than one candidate
        // get a suffix, and it counts up from the class, not from the run.
        let ids = |s: &select::Selection, class: Class| -> Vec<String> {
            let mut v: Vec<String> = s
                .admitted
                .iter()
                .filter(|a| a.primary_class == class)
                .map(|a| a.capture_id.clone())
                .collect();
            v.sort();
            v
        };
        assert_eq!(
            ids(&three, Class::Identical),
            vec!["identical", "identical+1"],
            "browser_assets and 00000001.blk are the only two identical paths"
        );
        assert_eq!(
            ids(&three, Class::Added),
            vec!["added", "added+1"],
            "libHttpClient.GDK.dll and 00000002.blk are the only two added paths"
        );
        for class in Class::ALL {
            let got = ids(&three, class);
            if got.len() == 1 {
                assert_eq!(got[0], class.id(), "a lone capture is named for its class");
            }
        }
        // `--per-class 3` does not invent candidates: the matrix still admits
        // exactly the files that exist.
        assert_eq!(three.admitted.len(), 10);
    }

    /// An `unavailable` class is a fact about the manifests and an `over_budget`
    /// one is a fact about this run's flags. `--require-class` turns both into
    /// exit `4`; nothing else does.
    ///
    /// This is the 7.0.0 → 7.1.0 pair, which genuinely has no
    /// `full_reuse_reordered` file, so a default that required every class would
    /// make every run of it fail.
    #[test]
    fn an_unavailable_class_is_recorded_and_never_pretends_to_be_a_failure() {
        let pool = select::candidates(&[game()]);
        let open = select::select(&pool, 1, Budget::unlimited(), &[]);
        let tight = select::select(&pool, 1, Budget::capped(0), &[]);

        // Unavailable: nothing in the pool satisfies it, and raising the ceiling
        // does not help.
        assert_eq!(
            open.unmet(),
            vec![Class::FullReuseReordered],
            "the ninth row is the only shortfall"
        );
        assert!(open.admitted.iter().all(|a| a.primary_class != Class::FullReuseReordered));

        // Over budget: nine shortfalls, and the eight that had a candidate each
        // name its cost. The ninth stays `Unavailable` — no ceiling could fund
        // a file that is not there.
        assert_eq!(tight.unmet().len(), 9);
        for row in &tight.matrix {
            if row.class == Class::FullReuseReordered {
                assert_eq!(row.status, MatrixStatus::Unavailable);
                assert_eq!(row.candidates_considered, 0);
                continue;
            }
            assert_eq!(row.status, MatrixStatus::OverBudget, "{row:?}");
            assert!(
                row.reason.as_deref().unwrap_or_default().contains("B and the budget"),
                "{}",
                row.reason.as_deref().unwrap_or_default()
            );
        }
        assert_eq!(tight.bytes_admitted, 0);
        assert_eq!(tight.rejected.len(), 8, "every wanted candidate is accounted for");
    }
}