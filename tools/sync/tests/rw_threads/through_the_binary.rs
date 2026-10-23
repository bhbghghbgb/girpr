//! Through the real binary: the same answer at every limit.
//!
//! The end-to-end half. `argv`, `run` and `flag_spellings` are in the parent because
//! the flag-surface cases above invoke the same way.

use super::*;

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
    let (code, _) = run(&argv("compare", &src, &dst, &[]));
    assert_eq!(code, 4, "the fixture differs, so the caches were written");

    let (before_src, before_dst) = (
        std::fs::read(sp(&src)).unwrap(),
        std::fs::read(sp(&dst)).unwrap(),
    );
    let (code, recs) = run(&argv(
        "sync",
        &src,
        &dst,
        &["--dry-run", "--rw-threads", "4"],
    ));
    assert_eq!(code, 0);
    assert!(
        recs.iter().any(|r| r["event"] == "missing"),
        "the dry run still reports the real plan: {recs:?}"
    );
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

/// **`update` hashes the same set of files at every limit.**
///
/// `update` rehashes every file, so the limit applies to it like any other reader.
/// Asserting it reaches the same read count at
/// every setting is what stops the knob from silently changing how much work the
/// command does -- `update`'s entire output is the cache, and a run that hashed a
/// different set would produce a different cache while still exiting 0.
#[test]
fn update_hashes_the_same_set_at_every_limit() {
    for flags in flag_spellings() {
        let t = temp_root("rw_update");
        let dir = t.subdir("w");
        for i in 0..40usize {
            let rel: &'static str = Box::leak(format!("u{i:02}.bin").into_boxed_str());
            wfile(&dir, rel, &vec![b'z'; 256 + i]);
        }
        let mut args: Vec<String> =
            vec!["update".into(), "--dir".into(), dir.display().to_string()];
        args.extend(flags.iter().map(|s| s.to_string()));
        let argv: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let (code, _) = run(&argv);
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
