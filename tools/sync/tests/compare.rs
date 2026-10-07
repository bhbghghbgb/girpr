//! End-to-end runs of `cmd_update` and `cmd_compare` (real FS + real cache).

mod common;

use common::{
    TempRoot, compare, compare_dry, compare_self_opts, has_backup_sibling, log, recs_of, rfile, rw,
    strip_algo, sync, sync_mtime, update, update_dry, wfile,
};
use girsync::cache::{CACHE_PREFIX, FileRec, open_db};
use girsync::config::{CommonOpts, ScanMode};
use girsync::effective::{
    EffRec, ScanStats, SideScan, classify, ensure_distinct_sides, open_side, resolve_side,
};
use girsync::planner::{SideRequest, plan_pairs};
use girsync::{cmd_compare, cmd_compare_self, cmd_sync, cmd_update};

/// A cache file's bytes, for "byte-identical" claims. Length alone would pass on
/// a rewrite that happened to produce the same row count.
fn bytes(p: &std::path::Path) -> Vec<u8> {
    std::fs::read(p).unwrap()
}

/// Every file in a tree, with its bytes, for asserting no file changed.
fn tree_state(root: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((
                    p.strip_prefix(root).unwrap().display().to_string(),
                    std::fs::read(&p).unwrap(),
                ));
            }
        }
    }
    out.sort();
    out
}

/// A two-tree fixture that makes the planner do real work: one equal pair that
/// must be hashed to confirm, one that is already decided by size, one src-only,
/// one dst-only, and one nested path.
fn two_trees(t: &TempRoot) -> (std::path::PathBuf, std::path::PathBuf) {
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"same");
    wfile(&dst, "a.txt", b"same");
    // The load-bearing file: same length, so size cannot settle it, and the
    // mtime pinned below so stat cannot either. Only reading both sides finds
    // the difference — a dry run that skipped hashing could not match it.
    wfile(&src, "changed.txt", b"src-version");
    wfile(&dst, "changed.txt", b"dst-version");
    // A pair stat already settles, so it must cost no read.
    wfile(&src, "sized.txt", b"longer-src-content");
    wfile(&dst, "sized.txt", b"short");
    wfile(&src, "src_only.txt", b"only in src");
    wfile(&dst, "dst_only.txt", b"only in dst");
    wfile(&src, "sub/nested.txt", b"nested");
    for rel in ["a.txt", "changed.txt"] {
        sync_mtime(&src.join(rel), &dst.join(rel));
    }
    (src, dst)
}

/// Resolve both sides the way `cmd_compare` does and hand back the maps, the
/// per-side counters, and the total hashed count, so two runs can be compared on
/// more than their verdicts.
fn effective_maps(
    src: &std::path::Path,
    dst: &std::path::Path,
    dry_run: bool,
) -> (
    std::collections::HashMap<String, EffRec>,
    std::collections::HashMap<String, EffRec>,
    ScanStats,
    usize,
) {
    let common = CommonOpts {
        dry_run,
        ..common::opts()
    };
    let (s, d) = (classify(src), classify(dst));
    ensure_distinct_sides(&s, &d).unwrap();
    let mode = ScanMode {
        no_trust_cached_hashes: false,
        dry_run,
    };
    let mut so = open_side(&s, &common, mode).unwrap();
    let mut do_ = open_side(&d, &common, mode).unwrap();
    // Labels only so a coverage error can name a side; these fixtures are both
    // folders and fully covered, so it never fires.
    let s_label = format!("{}", s.cache_path().display());
    let d_label = format!("{}", d.cache_path().display());
    let plans = plan_pairs(
        SideRequest {
            entries: &so.phase_a.map,
            algos: &common.algos,
            no_trust: false,
            cap: so.cap,
            label: &s_label,
        },
        SideRequest {
            entries: &do_.phase_a.map,
            algos: &common.algos,
            no_trust: false,
            cap: do_.cap,
            label: &d_label,
        },
        common.case_sensitive,
    )
    .unwrap();
    let sm: SideScan = resolve_side(&mut so, mode, &plans.src).unwrap();
    let dm: SideScan = resolve_side(&mut do_, mode, &plans.dst).unwrap();
    // `ScanStats` derives `PartialEq`, so this can be asserted whole rather than
    // field by field — which is the point: a new counter added later cannot
    // quietly diverge between the two modes without this test noticing.
    (
        sm.map.clone(),
        dm.map.clone(),
        sm.stats,
        sm.stats.hashed + dm.stats.hashed,
    )
}

/// **The general form of the dry-run contract: same counters, not merely same
/// verdict.** `--dry-run` promises the run answers the same question, so every
/// number it reports must match a real run's — including `pruned`, which is a
/// *decision* (how many rows would be dropped) and not a side effect.
///
/// This is the test that catches the whole class of bug rather than one instance
/// of it. `pruned` was the instance found: the prune loop sat behind the write
/// handle, so a dry run over a tree with an orphan row reported `0` where a real
/// run reports `1`. Nothing else compared the counters, so it passed. `ScanStats`
/// derives `PartialEq` so a future counter is covered by this assertion without
/// anyone remembering to extend it.
#[test]
fn dry_run_reports_the_same_counters_as_a_real_run() {
    let t = TempRoot::new("dry_stats");
    let (src, dst) = two_trees(&t);

    // An orphan row on each side, so `pruned` is non-zero and the two modes have
    // something to disagree about.
    for dir in [&src, &dst] {
        let p = dir.join(CACHE_PREFIX);
        let db = open_db(&p, true, rw()).unwrap();
        db.put(
            "vanished.txt",
            &FileRec {
                kind: "file".into(),
                size: 7,
                mtime_ns: 1234,
                hashes: [("md5".to_string(), vec![3u8; 16])].into_iter().collect(),
            },
        )
        .unwrap();
    }
    // And a stat-moved row, whose digests a real run drops and a dry run must
    // equally decide to drop.
    wfile(&src, "moved.txt", b"newer-and-longer");
    for dir in [&src, &dst] {
        let p = dir.join(CACHE_PREFIX);
        let db = open_db(&p, true, rw()).unwrap();
        db.put(
            "moved.txt",
            &FileRec {
                kind: "file".into(),
                size: 4,
                mtime_ns: 1,
                hashes: [("md5".to_string(), vec![9u8; 16])].into_iter().collect(),
            },
        )
        .unwrap();
    }

    let dry = effective_maps(&src, &dst, true);
    let real = effective_maps(&src, &dst, false);

    assert_eq!(
        dry.2.pruned, real.2.pruned,
        "a dry run must reach the same prune decision as a real run"
    );
    assert!(
        real.2.pruned >= 1,
        "the fixture must leave an orphan row, or this asserts nothing"
    );
    assert_eq!(dry.2, real.2, "every counter must match, not just `pruned`");
    assert_eq!(dry.3, real.3, "and so must the total hashed count");
    assert_eq!(dry.0, real.0, "src map");
    assert_eq!(dry.1, real.1, "dst map");
}

/// The direct check on the fix: a dry run *decides* to prune and reports it, and
/// still leaves the row on disk. The row surviving is the point — a dry run that
/// dropped it would be writing.
#[test]
fn a_dry_run_decides_to_prune_without_dropping_the_row() {
    let t = TempRoot::new("dry_prune");
    let (src, dst) = two_trees(&t);
    let p = src.join(CACHE_PREFIX);
    {
        let db = open_db(&p, true, rw()).unwrap();
        db.put("vanished.txt", &FileRec::dir()).unwrap();
    }
    assert!(
        recs_of(&src).contains_key("vanished.txt"),
        "the orphan row is there"
    );

    let dry = effective_maps(&src, &dst, true);
    assert!(
        dry.2.pruned >= 1,
        "a dry run reached the prune decision: pruned={}",
        dry.2.pruned
    );
    assert!(
        recs_of(&src).contains_key("vanished.txt"),
        "and did not act on it"
    );

    let real = effective_maps(&src, &dst, false);
    assert_eq!(dry.2.pruned, real.2.pruned, "same decision, both modes");
    assert!(
        !recs_of(&src).contains_key("vanished.txt"),
        "a real run drops it"
    );
}

/// The fixture must actually force hashing, or `compare_dry_run_computes_the_same_
/// digests` proves nothing: equal maps would fall out of both runs doing no work.
///
/// `changed.txt` is the load-bearing file — equal size and mtime, different
/// content — so `plan_pairs` cannot settle it from stat and both sides must read
/// it. `a.txt` and `sub/nested.txt` are equal pairs for the same reason, so they
/// hash too; the size-differing and one-sided paths must *not* appear in the
/// total, which is also checked so a regression to eager hashing is visible here.
#[test]
fn the_dry_run_fixture_forces_hashing_of_exactly_the_undecided_pairs() {
    let t = TempRoot::new("cmp_dry_fixture");
    let (src, dst) = two_trees(&t);
    let (sm, dm, _, hashed) = effective_maps(&src, &dst, false);

    // a.txt and changed.txt are equal-stat pairs, so the planner cannot settle
    // them and both sides are read: 2 pairs x 2 sides. changed.txt is the one
    // that differs in content, which is what forces the read to be *useful*
    // rather than merely required.
    assert_eq!(hashed, 4, "two undecided pairs, read on both sides");
    assert!(!sm["changed.txt"].hashes.is_empty(), "src read it");
    assert!(!dm["changed.txt"].hashes.is_empty(), "dst read it");
    assert_ne!(
        sm["changed.txt"].hashes, dm["changed.txt"].hashes,
        "the equal-stat pair really did differ, so hashing was required"
    );

    // Everything stat already settled costs nothing.
    assert!(
        sm["sized.txt"].hashes.is_empty() && dm["sized.txt"].hashes.is_empty(),
        "a size-differing pair needs no digest"
    );
    assert!(
        sm["src_only.txt"].hashes.is_empty(),
        "a src-only path needs no digest"
    );
    assert!(
        dm["dst_only.txt"].hashes.is_empty(),
        "a dst-only path needs no digest"
    );
}

#[test]
fn run_update_then_compare_equal() {
    let t = TempRoot::new("upd_eq");
    let dir = t.mkdirs("a");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "sub/b.txt", b"world");

    let code = cmd_update(update(dir.clone()), &log()).unwrap();
    assert_eq!(code, 0);
    assert!(
        dir.join(girsync::cache::CACHE_PREFIX).is_file(),
        "update creates the cache file"
    );

    // A second folder with the same content. mtime is part of identity, so pin
    // the copy's stamps to the original's; a same-root copy is not an option,
    // since `compare` refuses to name one cache twice.
    let mirror = t.mkdirs("a-mirror");
    wfile(&mirror, "a.txt", b"hello");
    wfile(&mirror, "sub/b.txt", b"world");
    for rel in ["a.txt", "sub/b.txt"] {
        sync_mtime(&dir.join(rel), &mirror.join(rel));
    }

    let code = cmd_compare(compare(dir.clone(), mirror.clone()), &log()).unwrap();
    assert_eq!(code, 0);

    // Record vs folder is equal without touching anything else.
    let record = dir.join(girsync::cache::CACHE_PREFIX);
    let code = cmd_compare(compare(record, mirror), &log()).unwrap();
    assert_eq!(code, 0);
}

/// A run must never name one cache twice: redb locks the file, and a folder
/// side rewrites its cache as it scans.
///
/// The self-collision this forbids is not merely wasteful. A folder side
/// populates its cache while building the effective map, so a record compared
/// against its own folder is diffed against a view the run is still mutating —
/// paths reported as drifted are written into the record before the report is
/// even printed, and a follow-up run over the same pair comes back clean. The
/// audit repairs the drift it was supposed to surface.
#[test]
fn run_compare_rejects_same_cache() {
    let t = TempRoot::new("cmp_self");
    let dir = t.mkdirs("a");
    wfile(&dir, "a.txt", b"hello");
    cmd_update(update(dir.clone()), &log()).unwrap();
    let record = dir.join(girsync::cache::CACHE_PREFIX);

    // The same folder on both sides.
    assert!(cmd_compare(compare(dir.clone(), dir.clone()), &log()).is_err());
    // The same record on both sides.
    assert!(
        cmd_compare(compare(record.clone(), record.clone()), &log()).is_err(),
        "one record cannot be compared against itself"
    );
    // A folder and the record inside it, in either order.
    assert!(
        cmd_compare(compare(dir.clone(), record.clone()), &log()).is_err(),
        "the record is the dst folder's own cache"
    );
    assert!(
        cmd_compare(compare(record.clone(), dir.clone()), &log()).is_err(),
        "the record is the src folder's own cache"
    );
    // Two spellings of one folder are one target, not two.
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let spelled = dir.join("sub").join("..");
    assert!(cmd_compare(compare(dir.clone(), spelled), &log()).is_err());

    // A genuinely different folder on the other side still works.
    let other = t.mkdirs("b");
    wfile(&other, "a.txt", b"hello");
    assert!(cmd_compare(compare(dir.clone(), other), &log()).is_ok());
}

#[test]
fn run_compare_detects_diff_then_sync_converges() {
    let t = TempRoot::new("diff_sync");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "keep.txt", b"same");
    wfile(&dst, "keep.txt", b"same");
    wfile(&src, "changed.txt", b"src-new-content-much-longer");
    wfile(&dst, "changed.txt", b"dst-old");
    wfile(&src, "src_only.txt", b"only in src");
    wfile(&dst, "dst_only.txt", b"only in dst");
    wfile(&src, "sub/nested.txt", b"nested");

    let code = cmd_compare(compare(src.clone(), dst.clone()), &log()).unwrap();
    assert_eq!(code, 4, "differences must exit 4");

    let mut o = sync(src.clone(), dst.clone());
    o.jobs = 2;
    let code = cmd_sync(o, &log()).unwrap();
    assert_eq!(code, 0);

    assert_eq!(rfile(&dst, "keep.txt"), b"same");
    assert_eq!(rfile(&dst, "changed.txt"), b"src-new-content-much-longer");
    assert_eq!(rfile(&dst, "src_only.txt"), b"only in src");
    assert_eq!(rfile(&dst, "sub/nested.txt"), b"nested");
    assert!(
        !dst.join("dst_only.txt").exists(),
        "extra deleted by default"
    );

    let code = cmd_compare(compare(src.clone(), dst.clone()), &log()).unwrap();
    assert_eq!(code, 0, "dst must equal src after sync");
}

/// `--dry-run` on `compare` must write nothing *and* answer the same question.
///
/// The flag exists because `compare` does write: a folder side updates its own
/// cache as it resolves, so without it there is no way to audit two folders and
/// leave both caches byte-identical.
///
/// The contract being pinned is the one that has never been enforced — a dry run
/// must do the same *work* and take the same *decisions*, differing only in the
/// writes. Asserting the exit code alone would not catch a dry run that skipped
/// the hashing: it would still print the same verdicts. So this compares the
/// resolved effective maps, which hold the digests themselves and cannot agree by
/// coincidence.
#[test]
fn compare_dry_run_writes_nothing_and_answers_the_same() {
    let t = TempRoot::new("cmp_dry");
    let (src, dst) = two_trees(&t);
    let src_cache = src.join(CACHE_PREFIX);
    let dst_cache = dst.join(CACHE_PREFIX);

    // Cold caches, so the dry run has the most to not-do: creating two cache
    // files, then populating them.
    assert_eq!(
        cmd_compare(compare_dry(src.clone(), dst.clone()), &log()).unwrap(),
        4
    );
    assert!(!src_cache.exists(), "a dry run does not create a cache");
    assert!(!dst_cache.exists(), "a dry run does not create a cache");

    // The real run creates both, so the dry-run-on-warm-cache case below has
    // something to leave alone.
    assert_eq!(
        cmd_compare(compare(src.clone(), dst.clone()), &log()).unwrap(),
        4
    );
    let (src_before, dst_before) = (bytes(&src_cache), bytes(&dst_cache));

    let src_stat = tree_state(&src);
    let dst_stat = tree_state(&dst);

    assert_eq!(
        cmd_compare(compare_dry(src.clone(), dst.clone()), &log()).unwrap(),
        4,
        "the dry run reaches the same verdict as the real one"
    );

    assert_eq!(
        bytes(&src_cache),
        src_before,
        "src's cache is byte-identical"
    );
    assert_eq!(
        bytes(&dst_cache),
        dst_before,
        "dst's cache is byte-identical"
    );
    assert_eq!(tree_state(&src), src_stat, "no file tree change either");
    assert_eq!(tree_state(&dst), dst_stat, "no file tree change either");
}

/// The strongest form of the same contract: two runs that differ only in
/// `--dry-run` must produce *equal effective maps*, digests included.
///
/// Separate from the test above because that one compares side effects, which a
/// run can get right while still having hashed a different set of files. Map
/// equality catches that — `a.txt` matching on stat and `changed.txt` decided by
/// size are different digests in the map, whatever the verdict says.
#[test]
fn compare_dry_run_computes_the_same_digests() {
    let t = TempRoot::new("cmp_dry_map");
    let (src, dst) = two_trees(&t);

    // Cold, so the dry run really does have to hash to answer. It gets an
    // in-memory cache, hashes everything the planner asks for, and keeps the
    // result only in the map.
    let dry = effective_maps(&src, &dst, true);
    let real = effective_maps(&src, &dst, false);

    assert_eq!(
        dry.0, real.0,
        "a dry run resolves src to the same effective map"
    );
    assert_eq!(
        dry.1, real.1,
        "a dry run resolves dst to the same effective map"
    );
    assert!(
        dry.3 > 0 && real.3 > 0,
        "both runs actually hashed something, so the comparison above is meaningful \
         (dry hashed {0}, real hashed {1})",
        dry.3,
        real.3
    );
    assert_eq!(
        dry.2, real.2,
        "and every counter agrees too — see `dry_run_reports_the_same_counters_as_a_real_run`"
    );
}

/// `update`'s entire output is the cache, so `--dry-run` there answers "what
/// would a repopulate do, and how much would it read?" without committing.
///
/// It must still hash everything — `update` trusts no cached digest, and a dry
/// run that skipped the hashing would not be answering the same question.
#[test]
fn update_dry_run_reads_everything_and_writes_nothing() {
    let t = TempRoot::new("upd_dry");
    let dir = t.mkdirs("a");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "sub/b.txt", b"world");
    let cache = dir.join(CACHE_PREFIX);

    assert_eq!(cmd_update(update_dry(dir.clone()), &log()).unwrap(), 0);
    assert!(!cache.exists(), "a dry run creates no cache");

    // A warm cache must come out byte-identical, with no backup sibling either.
    cmd_update(update(dir.clone()), &log()).unwrap();
    let before = bytes(&cache);
    assert_eq!(cmd_update(update_dry(dir.clone()), &log()).unwrap(), 0);
    assert_eq!(bytes(&cache), before, "the cache is byte-identical");
    assert!(!has_backup_sibling(&cache), "a dry run takes no backup");
}

/// `compare-self` never writes, so `--dry-run` is redundant rather than
/// contradictory. It must warn and carry on, not error: a script that passes
/// `--dry-run` to every subcommand to be safe should not break on the one
/// command that was already safe.
#[test]
fn compare_self_accepts_dry_run_with_a_warning() {
    let t = TempRoot::new("cs_dry");
    let dir = t.mkdirs("a");
    wfile(&dir, "a.txt", b"hello");
    cmd_update(update(dir.clone()), &log()).unwrap();
    let cache = dir.join(CACHE_PREFIX);
    let before = bytes(&cache);

    let mut o = compare_self_opts(dir.clone());
    o.common.dry_run = true;
    let code = cmd_compare_self(o, &log()).unwrap();
    assert_eq!(code, 0, "the run still completes");
    assert_eq!(
        bytes(&cache),
        before,
        "and still writes nothing, because it never did"
    );
}

/// The coverage rule, end-to-end, through the same planner `compare-self` uses.
///
/// The fixture is a record against a folder that started as a copy of it: same
/// content, mtimes pinned, both caches warmed, then `md5` stripped from **both**
/// rows.
///
/// Stripping it from the folder's row too is what makes this double as the ordering
/// gate. With no digest on either side, a run that reached phase C would hash
/// `a.txt` on the folder side and write the digest back - so the row *gaining* an
/// `md5` is the observable, and its absence is the proof that phase C never ran.
///
/// **Rows, not bytes.** Byte-identity is the stronger claim and it is available
/// under `--dry-run` - `compare_dry_run_writes_nothing_and_answers_the_same` relies
/// on it - but this is a *real* run, whose folder cache is opened read-write, and
/// redb may touch a file merely by being opened. Comparing bytes here would be a
/// test of redb's open-time behaviour wearing the costume of a test of the
/// coverage rule.
#[test]
fn an_uncovered_record_fails_before_the_folder_side_is_resolved() {
    let t = TempRoot::new("cmp_cover");
    let rec_home = t.mkdirs("rec");
    let folder = t.mkdirs("live");
    for dir in [&rec_home, &folder] {
        wfile(dir, "a.txt", b"hello");
    }
    sync_mtime(&rec_home.join("a.txt"), &folder.join("a.txt"));
    cmd_update(update(rec_home.clone()), &log()).unwrap();
    cmd_update(update(folder.clone()), &log()).unwrap();
    strip_algo(&rec_home, "a.txt", "md5");
    strip_algo(&folder, "a.txt", "md5");
    assert_eq!(
        recs_of(&folder)["a.txt"].hashes.len(),
        0,
        "the fixture starts with nothing for the folder side to reuse"
    );

    let err = cmd_compare(compare(rec_home.join(CACHE_PREFIX), folder.clone()), &log())
        .expect_err("an uncovered record cannot answer the question");
    let msg = format!("{:#}", err);
    assert!(msg.contains("girpr-cache"), "names the record:\n{msg}");
    assert!(msg.contains("a.txt"), "and an example path:\n{msg}");
    assert!(msg.contains("girsync update"), "and a remedy:\n{msg}");

    assert_eq!(
        recs_of(&folder)["a.txt"].hashes.len(),
        0,
        "the folder side was never resolved: nothing hashed, nothing written"
    );
}

/// `--no-trust-cached-hashes` on a **record** side must not make it uncovered.
///
/// Trust asks "re-read rather than reuse", and a record has nothing to re-read:
/// it has no filesystem. Folding trust into availability would make
/// `--no-trust-cached-hashes src` fatal for every record, including one holding
/// exactly the right digests - which is what `convert.rs`'s unrequested-algorithm
/// case does, so this is a live regression and not a hypothetical.
///
/// Availability is `cached` plus `hashable`, and trust is absent from it on purpose.
#[test]
fn distrusting_a_record_does_not_make_it_uncovered() {
    let t = TempRoot::new("cmp_notrust_rec");
    let rec_home = t.mkdirs("rec");
    let folder = t.mkdirs("live");
    for dir in [&rec_home, &folder] {
        wfile(dir, "a.txt", b"hello");
    }
    sync_mtime(&rec_home.join("a.txt"), &folder.join("a.txt"));
    cmd_update(update(rec_home.clone()), &log()).unwrap();
    cmd_update(update(folder.clone()), &log()).unwrap();

    let mut o = compare(rec_home.join(CACHE_PREFIX), folder.clone());
    o.trust.no_trust_src = true;
    o.trust.no_trust_dst = true;
    assert_eq!(
        cmd_compare(o, &log()).unwrap(),
        0,
        "the record holds md5, so distrusting its cache changes nothing it owes"
    );
}
