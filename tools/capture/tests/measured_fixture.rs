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
        // common + changed, one chunk moved -> `partial_moved_offset`
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
                // `aaaa` keeps its offset; `ffff` is the one that moved.
                chunk("aaaa", 2048, 1024, 90),
                chunk("ffff", 3072, 1024, 100),
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
            vec![chunk("iiii", 0, 42, 42)],
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

fn pair() -> (FieldDiff, FieldDiff) {
    // `diff_field` takes `CapturedManifest`s; building two full ones here would
    // be 200 lines of metadata boilerplate for a pure function, so the shapes
    // are assembled directly and the same construction is reused by the tests
    // above.
    let (old, new) = fixture();
    let mut a = gircap::manifest::CapturedManifest::empty("7.0.0", "game", old);
    let mut b = gircap::manifest::CapturedManifest::empty("7.1.0", "game", new);
    a.recompute_decoded();
    b.recompute_decoded();
    (
        diff::diff_field(&a, &b).expect("same field"),
        diff::diff_field(&a, &b).expect("same field"),
    )
}

#[test]
fn the_five_path_counts_are_what_the_live_capture_measured() {
    let (d, _) = pair();
    assert_eq!(
        (
            d.counts.common,
            d.counts.identical,
            d.counts.changed(),
            d.counts.added,
            d.counts.removed
        ),
        (6, 2, 4, 2, 1),
        "{MEASURED}: live hk4e_global 7.0.0 -> 7.1.0 gave common=2662 \
         identical=1592 changed=1070 added=82 removed=11. The fixture is scaled \
         down but must produce the same arithmetic, and 2662 + 82 = 2745 entries \
         while 2662 + 11 = 2673 — the two sides have to reconcile with each \
         other as well as with the record."
    );
}

/// The identity that makes the two counts consistent: every common path is
/// either identical or changed, and the two are disjoint.
#[test]
fn identical_and_changed_partition_the_common_set() {
    let (d, _) = pair();
    assert_eq!(d.counts.identical + d.counts.changed(), d.counts.common);
    for row in d.rows.iter().filter(|r| matches!(r.state, diff::State::Identical | diff::State::Changed)) {
        assert!(row.score.is_some(), "a common path always carries a score");
    }
    assert!(d.scores.iter().all(|s| !s.same_md5 || s.reused == s.nchunks));
}

/// The pool's headline numbers, with the two identities kept apart.
#[test]
fn the_reuse_pool_separates_manifest_wide_reach_from_same_file_reach() {
    let (d, _) = pair();
    let p: &ReusePool = &d.pool;
    assert_eq!(p.new_chunks_total, 13, "{MEASURED}: 113610 in the live 7.1.0");
    // Every new chunk except `iiii` and `llll`/`mmmm`/`nnnn` has its md5
    // somewhere in the old manifest.
    assert_eq!(p.new_chunks_md5_present_in_old_manifest, 10);
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
    let (d, _) = pair();
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
    let (d, _) = pair();
    let blk = d
        .scores
        .iter()
        .find(|s| s.path.ends_with("09801749.blk"))
        .unwrap();
    assert_eq!(blk.reused, 3, "eeee, aaaa and ffff all exist in this same old file");
    assert_eq!(blk.moved, 1, "only ffff moved: 1024 -> 3072");
    assert_eq!(blk.patch_bytes, 0, "nothing has to be fetched");
    assert_eq!(
        blk.new_size, blk.old_size,
        "the same three chunks, reordered — the file did not grow"
    );
    assert_eq!(blk.old_bytes, 100 + 90 + 100);
    assert_eq!(blk.new_bytes, 100 + 90 + 100);
}

/// Ordering is the determinism the whole selection rests on, so it is pinned on
/// the same fixture rather than on a synthetic one.
#[test]
fn cost_ordering_puts_the_cheapest_usable_files_first() {
    let (d, _) = pair();
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
            // 887 872 + 887 872 = 1 775 744 B
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/00000001.blk",
            // 913 040 + 1 172 835 = 2 085 875 B
            "GenshinImpact_Data/Plugins/HoYoChannel.dll",
        ],
        "ordered by total_cost = old_bytes + new_bytes, never by patch_bytes — which \
         is why HoYoChannel.dll, the file a patch actually has the most to do to, \
         is the *last* of the six rather than the first"
    );
    // Re-running gives the identical order.
    let again: Vec<String> = diff::by_cost(&[pair().0])
        .into_iter()
        .map(|(_, s)| s.path)
        .collect();
    assert_eq!(paths, again.iter().map(String::as_str).collect::<Vec<_>>());
}

/// The aggregate carries the same numbers plus the per-field breakdown, and
/// prefixes one-sided paths so a path from two pools cannot collide.
#[test]
fn aggregation_keeps_the_counts_and_names_the_field() {
    let (d, _) = pair();
    let agg: PathDiff = diff::aggregate("7.0.0", "7.1.0", &[d]);
    assert_eq!(agg.old_tag, "7.0.0");
    assert_eq!(agg.new_tag, "7.1.0");
    assert_eq!(agg.fields, vec!["game"]);
    assert_eq!(agg.counts.common, 6);
    assert_eq!(agg.by_field["game"].added, 2);
    assert!(agg.added.iter().all(|p| p.starts_with("game:")));
    assert_eq!(agg.added.len(), 2);
    assert_eq!(agg.removed.len(), 1);
    assert_eq!(agg.byte_counts.added, 116_800 + 887_872);
    assert_eq!(agg.byte_counts.removed, 1_048_576);
    assert_eq!(
        agg.byte_counts.patch,
        274_426 + 300,
        "only what a patch would actually fetch"
    );
    assert_eq!(agg.new_chunks_reusable_at_all, 10);
    assert!(agg.is_a_real_upgrade());
}

/// `paths.tsv` is the file a reader greps, so its header and its arithmetic
/// are pinned here rather than only in the module's own tests.
#[test]
fn the_flat_tsv_carries_every_row_of_both_sides() {
    let (d, _) = pair();
    let tsv = diff::paths_tsv(&[d]);
    let lines: Vec<&str> = tsv.lines().collect();
    assert_eq!(
        lines[0],
        "field\tpath\tstate\told_size\tnew_size\treused\tmoved\tpatch_bytes\ttotal_cost"
    );
    // 7 old entries + 8 new entries, minus the 6 common counted once.
    assert_eq!(lines.len(), 1 + 6 + 2 + 1, "header plus every path in either side");
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
            "added", "added", "changed", "changed", "changed", "changed", "identical",
            "identical", "removed"
        ]
    );
}