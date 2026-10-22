//! The other commands take the flags too.
//!
//! `sync` and `compare-self` are where the flags stop being an optimisation: with the
//! short circuit off, `sync` copies what the default would have left alone.

use super::*;

/// `sync` takes the flags, and with the short circuit off it *copies* what the default
/// only rehashes — because a size-changed pair that now needs a digest is still
/// `CHANGED`, and `CHANGED` in a sync is a copy.
#[test]
fn sync_with_the_short_circuit_off_still_converges() {
    let t = temp_root("st_sync");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"short");
    wfile(
        &dst,
        "a.txt",
        b"a much longer body than the other side has here",
    );

    let off = StatTrust::default().without_size().without_mtime();
    assert_eq!(
        cmd_sync(
            girsync::SyncOpts {
                src: src.clone(),
                dst: dst.clone(),
                trust: Default::default(),
                missing_only: false,
                keep_extra: false,
                common: distrusting(&["md5"], off),
            },
            &log(),
        )
        .unwrap(),
        0
    );
    assert_eq!(
        std::fs::read(dst.join("a.txt")).unwrap(),
        b"short",
        "dst now holds src's bytes"
    );
    assert_eq!(
        cmd_sync(
            girsync::SyncOpts {
                src,
                dst: dst.clone(),
                trust: Default::default(),
                missing_only: false,
                keep_extra: false,
                common: distrusting(&["md5"], off),
            },
            &log(),
        )
        .unwrap(),
        0,
        "and a second run is a no-op, so the flags do not make a sync non-idempotent"
    );
}

/// `compare-self` inherits the check, which for an audit is the more interesting one:
/// its disk side has no cache, so `--no-trust-mtime` turns "the cache says this file
/// changed" into "the cache says this file changed *and here is the content that
/// proves it*".
#[test]
fn compare_self_reads_a_touched_file_when_mtime_is_distrusted() {
    let t = temp_root("st_self");
    let dir = t.subdir("w");
    wfile(&dir, "a.txt", b"identical");
    cmd_update(update(dir.clone()), &log()).unwrap();
    age(&dir, "a.txt", 60);

    let self_opts = |stat: StatTrust| girsync::CompareSelfOpts {
        dir: dir.clone(),
        no_trust_cached_hashes: false,
        common: distrusting(&["md5"], stat),
    };
    assert_eq!(
        cmd_compare_self(self_opts(StatTrust::default()), &log()).unwrap(),
        4,
        "default: the record's mtime disagrees with disk, so CHANGED"
    );
    assert_eq!(
        cmd_compare_self(self_opts(StatTrust::default().without_mtime()), &log(),).unwrap(),
        0,
        "distrusting mtime: the bytes are identical, so the cache is in step"
    );
}
