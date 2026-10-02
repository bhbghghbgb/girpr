//! End-to-end runs of `cmd_update` and `cmd_compare` (real FS + real cache).

mod common;

use common::{
    TempRoot, compare, has_backup_sibling, log, pair, resolve_both, resolve_both_dry, rfile, sync,
    sync_mtime, update, wfile, with_algos,
};
use girsync::cache::CACHE_PREFIX;
use girsync::commands::verdict;
use girsync::diff::diff_maps;
use girsync::{TrustOpts, cmd_compare, cmd_sync, cmd_update};

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

/// `--dry-run` must answer the *same* question as a real run, with the writes
/// removed and nothing else. That is the whole contract, and it has two halves
/// that are easy to conflate: the decisions must be identical, and the writes
/// must be absent.
///
/// The decisions half is the one that was never enforced, and it is easy to break
/// by accident — the tempting "optimisation" is to skip hashing under a dry run
/// on the grounds that the digest would only be written back. That silently
/// changes the verdict, because a stat-equal pair whose contents differ is only
/// `CHANGED` *because* of the digest.
///
/// So `lying.txt` here is same size and same mtime with different bytes. A run
/// that stops hashing calls it EQUAL; a run that hashes calls it CHANGED. It must
/// say CHANGED either way, and the dry run must read exactly what the real run
/// read.
#[test]
fn compare_dry_run_reaches_the_same_verdict_and_hashes_the_same() {
    let t = TempRoot::new("cmp_dry_verdict");
    let (src, dst) = pair(
        &t,
        &[
            ("lying.txt", Some(b"AAAA"), Some(b"BBBB")),
            ("differ.txt", Some(b"alpha-longer"), Some(b"bbb")),
        ],
        &["md5"],
        // Cold, so `lying.txt` is settled by a digest this run must *read*, not
        // by one already in the cache. `differ.txt` differs in size, so stat
        // settles it and it costs no read — hence exactly one digest per side.
        false,
    );
    let algos = ["md5".to_string()];

    // Dry run *first*, deliberately. The two runs must be compared from the same
    // starting state, and a real run populates the cache — so running it first
    // would leave the dry run reading a warm cache and hashing nothing, which
    // says nothing about the invariant. A dry run writes nothing, so running it
    // first leaves both sides cold for the real run.
    let dry = resolve_both_dry(&src, &dst);
    let dry_lines = verdict(&diff_maps(&dry.0.map, &dry.1.map, &algos, true));
    assert_eq!(
        dry_lines,
        [
            ("changed", "CHANGED differ.txt".to_string()),
            ("changed", "CHANGED lying.txt".to_string()),
            (
                "summary",
                "SUMMARY missing=0 extra=0 changed=2 type_conflict=0 case_mismatch=0 total_diff=2"
                    .to_string()
            )
        ],
        "a stat-equal pair with different content is CHANGED — only a digest can say so"
    );
    assert_eq!(
        dry.0.stats.hashed + dry.1.stats.hashed,
        2,
        "the dry run reads the digest it needs, even though it will not keep it"
    );

    let real = resolve_both(&src, &dst, TrustOpts::default());
    let real_lines = verdict(&diff_maps(&real.0.map, &real.1.map, &algos, true));
    assert_eq!(
        dry_lines, real_lines,
        "a dry run must reach the identical verdict"
    );

    // The strongest available form of the contract: not just the same
    // conclusion, but the *same effective map* on both sides. Verdict equality
    // alone is weaker — it can hold while a side hashed something different and
    // arrived at the same answer by luck. This is also what makes `sync`'s plan
    // exact, since `build_plan` reads these maps and nothing else.
    assert_eq!(dry.0.map, real.0.map, "src effective map is identical");
    assert_eq!(dry.1.map, real.1.map, "dst effective map is identical");
    assert_eq!(
        dry.0.stats.hashed + dry.1.stats.hashed,
        real.0.stats.hashed + real.1.stats.hashed,
        "a dry run reads exactly what a real run reads; suppressing those reads \
         would change the verdict, not merely the side effects"
    );
}

/// The other half: a dry run writes nothing, *including* not creating a cache
/// that did not exist. A read/write open would create one per side and write its
/// `meta`, so this is the difference between "no writes" and "no file-tree writes".
#[test]
fn compare_dry_run_creates_no_cache() {
    let t = TempRoot::new("cmp_dry_nocache");
    let (src, dst) = pair(
        &t,
        &[("a.txt", Some(b"alpha"), Some(b"bbb"))],
        &["md5"],
        false,
    );
    assert!(!src.join(CACHE_PREFIX).exists());

    let mut o = compare(src.clone(), dst.clone());
    o.dry_run = true;
    assert_eq!(
        cmd_compare(o, &log()).unwrap(),
        4,
        "still reports the difference"
    );

    for d in [&src, &dst] {
        assert!(
            !d.join(CACHE_PREFIX).exists(),
            "compare --dry-run leaves {} without a cache",
            d.display()
        );
        assert!(
            !has_backup_sibling(&d.join(CACHE_PREFIX)),
            "compare --dry-run makes no backups"
        );
    }
}

/// An existing cache is opened read-only, so the file comes out byte-identical —
/// not merely "not grown". This is the guarantee `compare-self` already relies on,
/// and `compare --dry-run` now inherits it.
///
/// `sha256` is asked for and not cached, so phase C *must* compute a digest it
/// then discards: a real run has every opportunity to write here.
#[test]
fn compare_dry_run_leaves_an_existing_cache_byte_identical() {
    let t = TempRoot::new("cmp_dry_bytes");
    let (src, dst) = pair(
        &t,
        &[("a.txt", Some(b"alpha"), Some(b"alpha"))],
        &["md5"],
        true,
    );
    let before = std::fs::read(src.join(CACHE_PREFIX)).unwrap();

    let mut o = compare(src.clone(), dst.clone());
    o.common = with_algos(&["sha256"]);
    o.dry_run = true;
    assert_eq!(cmd_compare(o, &log()).unwrap(), 0);

    assert_eq!(
        std::fs::read(src.join(CACHE_PREFIX)).unwrap(),
        before,
        "a dry run leaves an existing cache byte-identical"
    );
}
