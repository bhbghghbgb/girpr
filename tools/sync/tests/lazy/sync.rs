//! `sync`, and what a folder side does to its own cache.
//!
//! `cmd_sync` returns an exit code and nothing else, so the counters are not
//! reachable from outside it. Asserting them would mean either scraping the log —
//! which pins a log format as a contract — or adding a return value for a test.
//!
//! So `sync` gets two layers instead. The harness below pins what the *phases*
//! decide, which is where the number is defined; and `a_lazy_sync_records_no_digest
//! _it_never_needed` pins that the *command* reaches those phases, by looking at
//! the cache a run leaves behind. The second is the one that would actually fail if
//! `cmd_sync` kept calling the monolithic scan.

use super::*;

/// Per-side `hashed` for a `sync`, computed the way `cmd_sync` computes it.
fn sync_counts(src: &Path, dst: &Path, common: &CommonOpts, trust: TrustOpts) -> (usize, usize) {
    let mode = |no_trust| ScanMode {
        no_trust_cached_hashes: no_trust,
        dry_run: false,
    };
    let src_db = open_folder_cache(src, common, mode(trust.no_trust_src), false).unwrap();
    let dst_db = open_folder_cache(dst, common, mode(trust.no_trust_dst), false).unwrap();
    let src_a = scan_stat_only(src, &src_db, common, mode(trust.no_trust_src)).unwrap();
    let dst_a = scan_stat_only(dst, &dst_db, common, mode(trust.no_trust_dst)).unwrap();
    // `common.case_sensitive`, not `true`: the planner runs *before* the rename
    // pass, so it has to pair the sides as they are on disk. See
    // `a_case_only_difference_in_content_is_copied_not_merely_renamed`.
    //
    // Both sides are folders, so coverage cannot fail here — which is the point
    // of `sync` sharing the check rather than skipping it.
    let s_label = format!("folder {}", src.display());
    let d_label = format!("folder {}", dst.display());
    let plans = plan_pairs(
        SideRequest {
            entries: &src_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_src,
            cap: SideCapability::for_folder(&src_db, mode(trust.no_trust_src)),
            label: &s_label,
        },
        SideRequest {
            entries: &dst_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_dst,
            cap: SideCapability::for_folder(&dst_db, mode(trust.no_trust_dst)),
            label: &d_label,
        },
        CasePairing::from(common.case_sensitive),
        common.hash_mode,
        common.stat,
    )
    .unwrap();
    let sm = resolve_folder(
        src,
        &src_db,
        mode(trust.no_trust_src),
        &src_a,
        &plans.src,
        RwSide::Src,
        &serial_rw(),
    )
    .unwrap();
    let dm = resolve_folder(
        dst,
        &dst_db,
        mode(trust.no_trust_dst),
        &dst_a,
        &plans.dst,
        RwSide::Dst,
        &serial_rw(),
    )
    .unwrap();
    (sm.stats.hashed, dm.stats.hashed)
}

/// A cold tree where every pair differs in size costs the scan nothing. This is
/// the same shape as `cold_all_stat_differing_pairs_read_nothing`, and it is the
/// larger win: `sync` is where the I/O volume lives.
#[test]
fn a_sync_scan_reads_nothing_when_every_pair_differs_in_size() {
    let t = TempRoot::new("lz_sync_differ");
    let (src, dst) = four_differing(&t);
    assert_eq!(
        sync_counts(&src, &dst, &opts(), TrustOpts::default()),
        (0, 0),
        "size decides every pair, so no digest is needed on either side"
    );
}

/// A src-only file is copied without ever needing a digest at scan time: the pair
/// is `MISSING`, which presence settles, and `copy_one` digests src as it copies
/// and re-hashes what it wrote to verify, returning the record it caches. So a
/// src-only file costs the *scan* nothing, and the copy itself reads src once.
#[test]
fn a_src_only_file_costs_the_sync_scan_no_digest() {
    let t = TempRoot::new("lz_sync_srconly");
    let (src, dst) = pair(
        &t,
        &[
            ("shared.txt", Some(b"shared"), Some(b"shared")),
            ("only_src.txt", Some(b"only-here"), None),
            ("only_dst.txt", None, Some(b"only-there")),
        ],
        &["md5"],
        false,
    );
    // Cold caches, so nothing is cached and only the shared stat-equal pair counts.
    assert_eq!(
        sync_counts(&src, &dst, &opts(), TrustOpts::default()),
        (1, 1),
        "only the shared stat-equal pair is undecided"
    );
}

/// The end-to-end proof that `cmd_sync` reached the planner, and the gate that
/// would fail if it did not.
///
/// Laziness is not observable in a return value here, but it *is* observable in
/// what a run leaves in the cache. On a tree whose every pair differs in size, an
/// eager scan writes an md5 for every file it read; a lazy one decides those pairs
/// from stat, never reads them, and writes a **stat-only row** instead. The
/// fingerprints are incompatible, so this fails loudly either way.
///
/// It also pins where digests still come from, which is the part that is easy to
/// break by over-applying laziness: `copy_one` hashes the copy to verify it and
/// returns dst's record, so **dst** ends the run with digests even though its
/// scan read nothing.
#[test]
fn a_lazy_sync_records_no_digest_it_never_needed() {
    let t = TempRoot::new("lz_sync_lazy");
    let (src, dst) = four_differing(&t);
    let mut o = sync_opts(src.clone(), dst.clone());
    o.common.case_sensitive = true;
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    // Every pair differed in size, so every pair was decided by stat.
    for rel in ["a.txt", "b.txt", "sub/c.txt", "sub/deep/d.txt"] {
        assert_eq!(rfile(&dst, rel), rfile(&src, rel), "{rel} was mirrored");
        assert!(
            digests_of(&src, rel).is_empty(),
            "{rel}: src's scan decided this pair from stat, so it read nothing \
             and recorded stat only. An md5 here means the scan is still eager."
        );
    }

    // dst's digests come from `copy_one`'s verify, not from its scan — which is
    // the correct outcome, and the reason laziness does not leave dst unrecorded.
    assert_eq!(
        digests_of(&dst, "a.txt"),
        vec!["md5"],
        "the copy verified the bytes and cached the record"
    );
}

/// `update` must remain fully eager. It is the command that *defines* a complete
/// record, so a lazy `update` would leave a folder cache that the all-of rule
/// later rejects.
#[test]
fn update_still_hashes_every_file() {
    let t = TempRoot::new("lz_update");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"alpha");
    wfile(&dir, "sub/b.txt", b"bravo");
    let o = UpdateOpts {
        dir: dir.clone(),
        common: opts(),
    };
    assert_eq!(girsync::cmd_update(o, &log()).unwrap(), 0);
    assert_eq!(digests_of(&dir, "a.txt"), vec!["md5"]);
    assert_eq!(digests_of(&dir, "sub/b.txt"), vec!["md5"]);
}

/// Both cache handles stay live across all three phases now, so the
/// self-collision rule has to keep firing *before* any open. Two writable
/// handles on one file would be refused by redb with a far worse message.
#[test]
fn the_wider_handle_window_still_refuses_a_self_collision() {
    let t = TempRoot::new("lz_self");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"alpha");
    girsync::cmd_update(
        UpdateOpts {
            dir: dir.clone(),
            common: opts(),
        },
        &log(),
    )
    .unwrap();
    let o = girsync::CompareOpts {
        src: dir.clone(),
        dst: dir.join(CACHE_PREFIX),
        trust: TrustOpts::default(),
        common: opts(),
    };
    let err = girsync::cmd_compare(o, &log()).unwrap_err();
    assert!(
        format!("{:#}", err).contains("same cache"),
        "a folder against the record inside it is still refused: {:#}",
        err
    );
}

/// The `Side` re-export is what `classify` returns; assert the classification
/// itself so a case's fixture is not silently the wrong shape.
#[test]
fn fixtures_classify_as_folders() {
    let t = TempRoot::new("lz_classify");
    let dir = t.mkdirs("w");
    assert!(matches!(classify(&dir), Side::Folder(_)));
    assert!(matches!(classify(&dir.join(CACHE_PREFIX)), Side::Record(_)));
}

/// A folder side still corrects its own cache before the planner ever sees it:
/// a row whose stat no longer matches disk loses its digests, and a row whose
/// file is gone is dropped outright. Neither needs the other side.
///
/// The ordering is what makes this safe. The planner reads `SideEntry`, and a stale
/// digest surviving into that map would judge the pair against a pre-change hash —
/// so the correction has to happen inside the scan, not as a step someone remembers
/// to run later.
///
/// Covers both triggers at once: `stale.txt` grows (row survives, digests
/// dropped), `gone.txt` is deleted (row dropped entirely). `keep.txt` is the
/// control — untouched, so its digests must still be there afterwards.
#[test]
fn a_folder_prunes_its_own_cache_before_the_planner_sees_it() {
    let t = TempRoot::new("lz_prune");
    let (src, dst) = pair(
        &t,
        &[
            ("keep.txt", Some(b"keep"), Some(b"keep")),
            ("stale.txt", Some(b"stale"), Some(b"stale")),
            ("gone.txt", Some(b"gone"), Some(b"gone")),
        ],
        &["md5"],
        true,
    );
    assert_eq!(digests_of(&src, "stale.txt"), ["md5"]);
    assert!(recs_of(&src).contains_key("gone.txt"));

    std::fs::write(src.join("stale.txt"), b"stale-and-longer").unwrap();
    std::fs::remove_file(src.join("gone.txt")).unwrap();

    let (sm, dm) = resolve_both(&src, &dst, TrustOpts::default());

    // The orphan is pruned, counted, and gone from the cache.
    assert_eq!(sm.stats.pruned, 1, "the deleted file's row is pruned");
    assert_eq!(dm.stats.pruned, 0, "dst was not touched");
    assert!(
        !recs_of(&src).contains_key("gone.txt"),
        "the orphan row is dropped, not merely ignored"
    );

    // The stale row survives — the file is still there — but with no digests.
    assert!(
        recs_of(&src).contains_key("stale.txt"),
        "a stat-changed row is corrected, not removed"
    );
    assert!(
        digests_of(&src, "stale.txt").is_empty(),
        "a stale row's digests are dropped before the planner can read them"
    );
    assert_eq!(
        digests_of(&src, "keep.txt"),
        ["md5"],
        "an untouched row keeps its digests"
    );

    // And the verdicts are unaffected: pruning corrected the cache, it did not
    // change what the tree says. `gone.txt` was deleted from *src*, so dst still
    // has it — `EXTRA`, and still reported even though the orphan row on the src
    // side is gone.
    // Built by hand rather than from a plan, so the trust is stated here too —
    // and trusting both is what makes this the same diff the commands would produce.
    let d = diff_maps(
        &sm.map,
        &dm.map,
        &Required {
            by_rel: HashMap::new(),
            fallback: vec![HashAlg::Md5],
        },
        StatTrust::default(),
        CasePairing::Exact,
        false,
    );
    let reported: Vec<Value> = verdict(&d, false, false).iter().map(|r| r.json()).collect();
    assert_eq!(
        reported,
        [
            json!({"event": "extra", "path": "gone.txt"}),
            json!({"event": "changed", "path": "stale.txt"}),
            json!({"event": "summary", "missing": 0, "extra": 1, "changed": 1,
                   "type_conflict": 0, "case_mismatch": 0, "total_diff": 2}),
        ]
    );
}

/// Pruning is a correctness step, so `--no-trust-cached-hashes` must not be able
/// to switch it off. Distrusting a cache means "do not *reuse* it", never "keep
/// the rows that are wrong".
#[test]
fn distrusting_the_cache_does_not_suppress_pruning() {
    let t = TempRoot::new("lz_prune_nt");
    let (src, dst) = pair(
        &t,
        &[("gone.txt", Some(b"gone"), Some(b"gone"))],
        &["md5"],
        true,
    );
    std::fs::remove_file(src.join("gone.txt")).unwrap();

    let (sm, _dm) = resolve_both(
        &src,
        &dst,
        TrustOpts {
            no_trust_src: true,
            no_trust_dst: false,
        },
    );
    assert_eq!(sm.stats.pruned, 1);
    assert!(!recs_of(&src).contains_key("gone.txt"));
}
