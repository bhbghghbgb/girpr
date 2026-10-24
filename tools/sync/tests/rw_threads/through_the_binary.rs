//! Through the real binary: the same answer at every limit.
//!
//! The end-to-end half. `argv`, `run` and `flag_spellings` are in the parent because
//! the flag-surface cases above invoke the same way.

use super::*;

use std::num::NonZeroUsize;

use girsync::{cmd_compare, cmd_sync, cmd_update, UpdateOpts};

/// **`sync` reports the same actions under every rw setting, through the binary.**
///
/// The resolver-level test above pins equal effective maps; this one pins the
/// surface a caller actually reads, end to end through clap, config and the
/// applier — so a setting that fails to *reach* a phase cannot pass.
///
/// Compared as a **multiset** with the count asserted separately, because actions
/// are emitted as each one completes: order is a function of I/O timing and is not
/// part of any contract. The count is what stops a duplicate hiding inside a set.
#[test]
fn sync_reports_the_same_actions_under_every_rw_setting() {
    let acts = |recs: &[serde_json::Value]| -> (BTreeSet<String>, usize) {
        let set: BTreeSet<String> = recs
            .iter()
            .filter(|r| r["event"].as_str().is_some_and(is_action))
            .map(|r| r.to_string())
            .collect();
        let n = recs
            .iter()
            .filter(|r| r["event"].as_str().is_some_and(is_action))
            .count();
        (set, n)
    };

    let mut baseline: Option<(BTreeSet<String>, usize)> = None;
    for flags in flag_spellings() {
        let t = temp_root("rw_sync");
        let (src, dst) = fixture(&t);
        let (code, recs) = run_env(
            &argv("sync", &src, &dst, &flags),
            &[("GIRSYNC_TEST_CACHE", "memory")],
        );
        assert_eq!(code, 0, "{flags:?} exited {code}");

        let got = acts(&recs);
        assert!(
            got.1 > 0,
            "{flags:?}: the fixture must make the run do something"
        );
        // And every file really landed -- the action stream is not the filesystem.
        // (Actions are not only copies; `mkdir` is one too, which is why this
        // checks the copies individually rather than against the action count.)
        for r in recs.iter().filter(|r| r["event"] == "copy") {
            let path = r["path"].as_str().unwrap();
            assert!(
                dst.join(path).is_file(),
                "{path} was copied but is not on disk"
            );
        }

        match &baseline {
            None => baseline = Some(got),
            Some(want) => {
                assert_eq!(&got.0, &want.0, "{flags:?} reported different actions");
                assert_eq!(
                    got.1, want.1,
                    "{flags:?} reported a different number of actions, so a duplicate \
                     or a dropped one is hiding in the set"
                );
            }
        }
    }
}

/// **`--dry-run` is unaffected by the rw limit.**
///
/// The dry-run contract has two halves and the limiter could plausibly break
/// either: the run must still *do the work* (stat, read cached digests, hash
/// whatever stat cannot settle) and must still *write nothing*. A limiter change
/// that suppressed the hashing would keep the verdict identical while answering a
/// different question, which is exactly the failure `SyncStats` exists to catch.
#[test]
fn a_dry_run_is_unaffected_by_the_rw_limit() {
    let t = temp_root("rw_dry");
    let (src, dst) = fixture(&t);
    let sp = |d: &Path| d.join(CACHE_PREFIX);

    // Warm both caches, so the dry run has something real to leave alone.
    assert_eq!(
        cmd_compare(compare(src.clone(), dst.clone()), &log()).unwrap(),
        4,
        "the fixture differs, so the caches were written"
    );

    let (before_src, before_dst) = (
        std::fs::read(sp(&src)).unwrap(),
        std::fs::read(sp(&dst)).unwrap(),
    );

    let mut o = sync_dry(src.clone(), dst.clone());
    o.common.rw = RwLimits::Shared(NonZeroUsize::new(4).unwrap());
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    assert_eq!(
        std::fs::read(sp(&src)).unwrap(),
        before_src,
        "src's cache is byte-identical"
    );
    assert_eq!(
        std::fs::read(sp(&dst)).unwrap(),
        before_dst,
        "dst's cache is byte-identical"
    );
}

/// Map a `--rw-*` flag spelling to the `RwLimits` it resolves to, mirroring
/// the table in [`settings`](super::settings).
fn limits_of_flags(flags: &[&'static str]) -> RwLimits {
    let mut rw_threads: Option<usize> = None;
    let mut rw_threads_src: Option<usize> = None;
    let mut rw_threads_dst: Option<usize> = None;
    let mut rw_dual_drive = false;
    let mut i = 0;
    while i < flags.len() {
        match flags[i] {
            "--rw-threads" => {
                rw_threads = Some(flags[i + 1].parse().unwrap());
                i += 2;
            }
            "--rw-threads-src" => {
                rw_threads_src = Some(flags[i + 1].parse().unwrap());
                i += 2;
            }
            "--rw-threads-dst" => {
                rw_threads_dst = Some(flags[i + 1].parse().unwrap());
                i += 2;
            }
            "--rw-dual-drive" => {
                rw_dual_drive = true;
                i += 1;
            }
            _ => i += 1,
        }
    }
    RwLimits::try_from(RwArgs {
        rw_threads,
        rw_threads_src,
        rw_threads_dst,
        rw_dual_drive,
    })
    .unwrap_or_else(|e| panic!("{e:#}"))
}

/// **`update` hashes the same set of files at every limit.**
///
/// `update` rehashes every file, so the limit applies to it like any other reader.
/// Asserting it reaches the same read count at
/// every setting is what stops the knob from silently changing how much work the
/// command does -- `update`'s entire output is the cache, and a run that hashed a
/// different set would produce a different cache while still exiting 0.
///
/// Driven in-process via `cmd_update` instead of spawning the binary six times:
/// the invariant is about the planner's read set, not about the CLI boundary.
#[test]
fn update_hashes_the_same_set_at_every_limit() {
    for flags in flag_spellings() {
        let t = temp_root("rw_update");
        let dir = t.subdir("w");
        for i in 0..40usize {
            let rel: &'static str = Box::leak(format!("u{i:02}.bin").into_boxed_str());
            wfile(&dir, rel, &vec![b'z'; 256 + i]);
        }
        let common = CommonOpts {
            rw: limits_of_flags(&flags),
            ..common::opts()
        };
        let code = cmd_update(
            UpdateOpts {
                dir: dir.clone(),
                common,
            },
            &log(),
        )
        .unwrap();
        assert_eq!(code, 0, "{flags:?} exited {code}");

        let rows = recs_of(&dir);
        let files = rows.values().filter(|r| !r.is_dir()).count();
        assert_eq!(
            files, 40,
            "{flags:?} must record every file, so the counts below mean something"
        );
        for (rel, rec) in &rows {
            if rec.is_dir() {
                continue;
            }
            assert_eq!(
                rec.hashes.len(),
                1,
                "{flags:?}: {rel} lost its md5, so this setting hashed less"
            );
        }
    }
}
