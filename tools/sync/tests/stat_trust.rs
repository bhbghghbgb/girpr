//! `--no-trust-size` / `--no-trust-mtime`: switching the stat short circuit off.
//!
//! The short circuit is an optimisation, so it has to be switchable. These two flags
//! name the stat fields it consults, and **both together is "disable it"** — no third
//! spelling is needed to reach the whole thing off.
//!
//! What each one does is stated in the planner's tests; what is left here is what can
//! only be seen from outside:
//!
//! - **the counters.** A flag that widens the undecided set has to actually cost the
//!   reads, and the counts are the only honest observable. Every case states the
//!   `all-of`-style default as a control, because a test asserting only "the flag
//!   reads more" would pass against a planner that always read everything.
//! - **the verdict.** `--no-trust-mtime` can *change* one — a touched file with
//!   identical bytes comes back equal. That is the flag's whole point, so it is
//!   asserted end to end rather than left to the planner's arithmetic.
//! - **the cache.** This is what `--no-trust-size` is really for, and it is invisible
//!   in both counters and verdicts.
//! - **the flags themselves**, through the real binary.

mod common;

use common::{TempRoot, age, log, opts, recs_of, serial_rw, sync_mtime, update, wfile};
use girsync::planner::StatTrust;
use girsync::{CommonOpts, cmd_compare, cmd_compare_self, cmd_sync, cmd_update};

/// `opts()` asking for `algos`, with stat trust set.
///
/// The flags are on `CommonOpts` rather than on a command's own opts because they are a
/// property of the *comparison*, not of any one side — the short circuit compares the
/// two sides' stats against each other, so there is no per-side version to have. This
/// is unlike `--no-trust-cached-hashes`, which distrusts one side's digest and so does
/// take a side.
fn distrusting(algos: &[&str], stat: StatTrust) -> CommonOpts {
    CommonOpts {
        algos: algos.iter().map(|a| a.to_string()).collect(),
        stat,
        ..opts()
    }
}

/// Two folders, four files, one per interesting pair state.
///
/// ```text
///   equal.txt    identical bytes, mtime pinned   -> undecided, hashed either way
///   sized.txt    dst is longer, mtime pinned     -> free by size
///   mtime'd.txt  identical bytes, dst's mtime moved -> free by mtime
///   gone.txt     dst only                        -> EXTRA, free
/// ```
///
/// `equal.txt` is the control: it is already undecided, so it hashes in every
/// configuration and any change in the other three is attributable.
fn four_states(t: &TempRoot) -> (std::path::PathBuf, std::path::PathBuf) {
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    for rel in ["equal.txt", "sized.txt", "mtime'd.txt"] {
        let body = format!("body of {rel}");
        wfile(&src, rel, body.as_bytes());
        wfile(&dst, rel, body.as_bytes());
    }
    // `sized.txt` differs in length on dst, mtime pinned so size is the only signal.
    wfile(
        &dst,
        "sized.txt",
        b"a much longer body than src has for this path",
    );
    for rel in ["equal.txt", "sized.txt", "mtime'd.txt"] {
        sync_mtime(&src.join(rel), &dst.join(rel));
    }
    // `mtime'd.txt` then gets its mtime pushed forward: identical bytes, same size,
    // so mtime is the only thing that differs.
    age(&dst, "mtime'd.txt", 60);
    wfile(&dst, "gone.txt", b"dst only");
    (src, dst)
}

/// Files opened by a two-sided `compare`, summed over both sides.
///
/// Resolving a folder side **writes to its cache**, so this is not a pure function of
/// its inputs and a case must not measure two configurations against one fixture — the
/// first run backfills whatever the second would then find cached. Every case here
/// builds a fresh fixture per configuration.
fn hashed(src: &std::path::Path, dst: &std::path::Path, common: &CommonOpts) -> usize {
    use girsync::config::ScanMode;
    use girsync::effective::{classify, ensure_distinct_sides, open_side, resolve_side};
    use girsync::planner::{SideRequest, plan_pairs};
    use girsync::rw::RwSide;

    let (s_side, d_side) = (classify(src), classify(dst));
    ensure_distinct_sides(&s_side, &d_side).unwrap();
    let mode = ScanMode {
        no_trust_cached_hashes: false,
        dry_run: false,
    };
    let mut s = open_side(&s_side, common, mode).unwrap();
    let mut d = open_side(&d_side, common, mode).unwrap();
    let s_label = format!("src {}", s_side.cache_path().display());
    let d_label = format!("dst {}", d_side.cache_path().display());
    let plans = plan_pairs(
        SideRequest {
            entries: &s.phase_a.map,
            algos: &common.algos,
            no_trust: false,
            cap: s.cap,
            label: &s_label,
        },
        SideRequest {
            entries: &d.phase_a.map,
            algos: &common.algos,
            no_trust: false,
            cap: d.cap,
            label: &d_label,
        },
        common.case_sensitive,
        common.hash_mode,
        common.stat,
    )
    .unwrap();
    let sm = resolve_side(&mut s, mode, &plans.src, RwSide::Src, &serial_rw()).unwrap();
    let dm = resolve_side(&mut d, mode, &plans.dst, RwSide::Dst, &serial_rw()).unwrap();
    sm.stats.hashed + dm.stats.hashed
}

// -- the counters --------------------------------------------------------------

/// **The headline: distrusting mtime costs exactly the pairs it widens.**
///
/// `mtime'd.txt` is free by default and needs a digest with the flag on. Nothing else
/// moves: `sized.txt` is still settled by size, `equal.txt` was already undecided, and
/// `gone.txt` is `EXTRA`.
#[test]
fn no_trust_mtime_costs_the_mtime_changed_pairs_and_nothing_else() {
    let t_default = TempRoot::new("st_mtime_off");
    let t_flagged = TempRoot::new("st_mtime_on");
    let (a_src, a_dst) = four_states(&t_default);
    let (b_src, b_dst) = four_states(&t_flagged);

    let default = hashed(&a_src, &a_dst, &distrusting(&["md5"], StatTrust::default()));
    let flagged = hashed(
        &b_src,
        &b_dst,
        &distrusting(&["md5"], StatTrust::default().without_mtime()),
    );
    assert_eq!(default, 2, "only `equal.txt`, on its two sides");
    assert_eq!(
        flagged, 4,
        "that plus `mtime'd.txt` on both sides: 2 files x 2 sides"
    );
}

/// And the same fixture with the *other* flag, which is what makes the two distinct
/// rather than one switch wearing two names.
#[test]
fn no_trust_size_costs_the_size_changed_pairs_and_nothing_else() {
    let t_default = TempRoot::new("st_size_off");
    let t_flagged = TempRoot::new("st_size_on");
    let (a_src, a_dst) = four_states(&t_default);
    let (b_src, b_dst) = four_states(&t_flagged);

    let default = hashed(&a_src, &a_dst, &distrusting(&["md5"], StatTrust::default()));
    let flagged = hashed(
        &b_src,
        &b_dst,
        &distrusting(&["md5"], StatTrust::default().without_size()),
    );
    assert_eq!(default, 2);
    assert_eq!(
        flagged, 4,
        "that plus `sized.txt` on both sides, and NOT `mtime'd.txt`"
    );
}

/// **Both flags is the whole thing off.** Every file on both sides is now undecided, so
/// the count is every file on both sides and nothing is free.
#[test]
fn distrusting_both_fields_reads_every_file_on_both_sides() {
    let t = TempRoot::new("st_both_off");
    let (src, dst) = four_states(&t);
    let off = StatTrust::default().without_size().without_mtime();
    assert_eq!(
        hashed(&src, &dst, &distrusting(&["md5"], off)),
        6,
        "3 *shared* files x 2 sides. `gone.txt` is dst-only, so it is EXTRA and free."
    );
}

/// **The exact count for the whole tree, stated against the eager baseline.** A
/// stat-settled tree costs nothing by default, and these flags let a user pay for
/// reading it deliberately. The number it should cost is *the same as `update`
/// would* for those files — one pass per side, not two reads per file.
///
/// Asserted against `hashed` rather than against a digest count, and against the
/// default as a control. `hash_file` computes every algorithm in one pass, so a
/// two-algorithm request reads each file once either way; the flags are about *which
/// files*, never about how many times.
#[test]
fn switching_the_short_circuit_off_costs_one_pass_per_side_not_one_read_per_algorithm() {
    let t = TempRoot::new("st_digests");
    let (src, dst) = four_states(&t);
    let off = StatTrust::default().without_size().without_mtime();
    let files = hashed(&src, &dst, &distrusting(&["md5", "sha256"], off));
    assert_eq!(
        files, 6,
        "3 shared files x 2 sides, with two algorithms asked for and one pass each"
    );
}

// -- the verdict ---------------------------------------------------------------

/// **The one flag that can change a verdict, and the case that makes it worth having.**
///
/// `mtime'd.txt` has identical bytes on both sides. Its mtime differs, so the default
/// reports `CHANGED` on the strength of the timestamp alone. With
/// `--no-trust-mtime` it is compared on content and comes back **equal**.
///
/// This is the difference between "these files differ" and "these files were touched",
/// and only a flag that distrusts mtime can tell them apart.
#[test]
fn no_trust_mtime_clears_a_touched_file_whose_bytes_never_changed() {
    let t = TempRoot::new("st_mtime_verdict");
    let (src, dst) = four_states(&t);
    let trust = StatTrust::default();

    // Default: mtime alone is enough to call it changed.
    assert_eq!(
        cmd_compare(
            girsync::CompareOpts {
                src: src.clone(),
                dst: dst.clone(),
                trust: Default::default(),
                common: distrusting(&["md5"], trust),
            },
            &log(),
        )
        .unwrap(),
        4,
        "the timestamp says changed, and that is the whole of the evidence"
    );

    // Distrusting mtime: the bytes are compared, and they agree.
    assert_eq!(
        cmd_compare(
            girsync::CompareOpts {
                src: src.clone(),
                dst: dst.clone(),
                trust: Default::default(),
                common: distrusting(&["md5"], trust.without_mtime()),
            },
            &log(),
        )
        .unwrap(),
        4,
        "the other two differences in the fixture are still real: sized.txt and gone.txt"
    );

    // And with just the one file, the verdict is clean.
    //
    // A single-file fixture rather than a filtered view of the four-state one, so the
    // expected exit code is 0 rather than 4-for-other-reasons. Filtering would have
    // left the assertion "still 4" unable to say *why*, which is the property under
    // test.
    let t1 = TempRoot::new("st_mtime_alone");
    let src1 = t1.mkdirs("one_src");
    let dst1 = t1.mkdirs("one_dst");
    wfile(&src1, "t.txt", b"identical");
    wfile(&dst1, "t.txt", b"identical");
    sync_mtime(&src1.join("t.txt"), &dst1.join("t.txt"));
    age(&dst1, "t.txt", 60);

    assert_eq!(
        cmd_compare(
            girsync::CompareOpts {
                src: src1.clone(),
                dst: dst1.clone(),
                trust: Default::default(),
                common: distrusting(&["md5"], trust),
            },
            &log(),
        )
        .unwrap(),
        4,
        "default: touched is CHANGED"
    );
    assert_eq!(
        cmd_compare(
            girsync::CompareOpts {
                src: src1.clone(),
                dst: dst1.clone(),
                trust: Default::default(),
                common: distrusting(&["md5"], trust.without_mtime()),
            },
            &log(),
        )
        .unwrap(),
        0,
        "no-trust-mtime: the bytes agree, so the pair is in step"
    );
}

/// **`--no-trust-size` cannot change a verdict**, because different lengths are
/// different content — which is the asymmetry between the two flags and the reason
/// `--no-trust-size` exists for the cache rather than for the answer.
#[test]
fn no_trust_size_never_turns_a_changed_file_into_an_unchanged_one() {
    let t = TempRoot::new("st_size_verdict");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"short");
    wfile(
        &dst,
        "a.txt",
        b"a considerably longer body than the other side",
    );
    sync_mtime(&src.join("a.txt"), &dst.join("a.txt"));

    let off = StatTrust::default().without_size().without_mtime();
    assert_eq!(
        cmd_compare(
            girsync::CompareOpts {
                src: src.clone(),
                dst: dst.clone(),
                trust: Default::default(),
                common: distrusting(&["md5"], off),
            },
            &log(),
        )
        .unwrap(),
        4,
        "with the short circuit fully off, a size difference is still CHANGED"
    );
}

// -- what no-trust-size is actually for ----------------------------------------

/// **The cache repair, which is the reason `no-trust-size` exists and the reason no
/// counter or verdict would ever reveal it.**
///
/// A row whose size no longer matches disk arrives in phase A with its digests
/// *dropped* — a stale row's are all suspect, that is the carry rule — and it is
/// stat-only from then on. Under the short circuit nothing ever asks for those digests
/// again, because a size-changed pair is always settled for free. So the row is stuck:
/// permanently stat-only, and the content it describes is unverifiable.
///
/// `--no-trust-size` puts it back in the undecided set, so the digest is recomputed and
/// stored.
///
/// Asserted on the **row**, which is the whole point: both runs report the same verdict
/// and the same `CHANGED`, and only one of them repairs the cache.
#[test]
fn no_trust_size_rehashes_a_stale_row_that_the_short_circuit_would_leave_stat_only() {
    // dst starts as a copy of src, so the row is fresh and both sides have a digest.
    let t = TempRoot::new("st_repair");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"original");
    wfile(&dst, "a.txt", b"original");
    sync_mtime(&src.join("a.txt"), &dst.join("a.txt"));
    cmd_update(update(src.clone()), &log()).unwrap();
    cmd_update(update(dst.clone()), &log()).unwrap();
    assert!(
        !recs_of(&dst)["a.txt"].hashes.is_empty(),
        "the row starts warm, so the repair has something to repair"
    );

    // dst's content changes length. Phase A drops dst's digest for being stale, and
    // the pair is now settled by size for free — so nothing rehashes it.
    //
    // The mtime is **re-pinned after the rewrite**, which is load-bearing: a rewrite
    // moves the mtime too, and a pair whose mtime *also* differs is still settled by
    // the field we are trusting, so `without_size()` alone would widen nothing and the
    // fixture would appear not to work. That is correct behaviour — the flag only acts
    // when the distrusted field is the *only* difference — and
    // `distrusting_mtime_leaves_a_size_difference_settling_the_pair` states the
    // converse in the planner's tests.
    wfile(&dst, "a.txt", b"original but longer now");
    sync_mtime(&src.join("a.txt"), &dst.join("a.txt"));
    let (d_src, d_dst) = (src.clone(), dst.clone());
    cmd_compare(
        girsync::CompareOpts {
            src: d_src,
            dst: d_dst,
            trust: Default::default(),
            common: distrusting(&["md5"], StatTrust::default()),
        },
        &log(),
    )
    .unwrap();
    assert!(
        recs_of(&dst)["a.txt"].hashes.is_empty(),
        "under the short circuit the row is left stat-only, and stays that way"
    );

    // Now the same edit, with the flag: the digest comes back.
    let t2 = TempRoot::new("st_repair2");
    let src2 = t2.mkdirs("src");
    let dst2 = t2.mkdirs("dst");
    wfile(&src2, "a.txt", b"original");
    wfile(&dst2, "a.txt", b"original");
    sync_mtime(&src2.join("a.txt"), &dst2.join("a.txt"));
    cmd_update(update(src2.clone()), &log()).unwrap();
    cmd_update(update(dst2.clone()), &log()).unwrap();
    wfile(&dst2, "a.txt", b"original but longer now");
    // Re-pinned for the same reason as the first half: size must be the only
    // difference, or the trusted mtime settles the pair and the flag does nothing.
    sync_mtime(&src2.join("a.txt"), &dst2.join("a.txt"));

    cmd_compare(
        girsync::CompareOpts {
            src: src2,
            dst: dst2.clone(),
            trust: Default::default(),
            common: distrusting(&["md5"], StatTrust::default().without_size()),
        },
        &log(),
    )
    .unwrap();
    let repaired = &recs_of(&dst2)["a.txt"];
    assert!(
        !repaired.hashes.is_empty(),
        "with --no-trust-size the digest is recomputed and stored: {repaired:?}"
    );
    assert_eq!(
        repaired.size, 23,
        "and the row describes the file as it is now, not as it was"
    );
}

// -- the other commands --------------------------------------------------------

/// `sync` takes the flags, and with the short circuit off it *copies* what the default
/// only rehashes — because a size-changed pair that now needs a digest is still
/// `CHANGED`, and `CHANGED` in a sync is a copy.
#[test]
fn sync_with_the_short_circuit_off_still_converges() {
    let t = TempRoot::new("st_sync");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
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
                jobs: 1,
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
                jobs: 1,
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
    let t = TempRoot::new("st_self");
    let dir = t.mkdirs("w");
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

// -- update, and the flag surface ---------------------------------------------

/// One invocation of the real binary: exit code and stderr.
fn bin(args: &[String]) -> (i32, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .args(args)
        .output()
        .expect("spawn girsync");
    (
        out.status.code().expect("girsync exits with a code"),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// **`update` warns and carries on.** It rehashes every file regardless, so there is no
/// short circuit for the flags to switch off — and *that* is the behaviour they ask
/// for, so obeying them by doing less would be wrong.
///
/// Following the `--hash-any-of` and `--no-trust-cached-hashes`-on-`compare-self`
/// precedent: a flag that cannot apply is warned about, not rejected, so a script
/// passing it to every subcommand keeps working.
#[test]
fn update_warns_that_the_flags_do_not_apply_and_still_populates() {
    let t = TempRoot::new("st_update");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    let d = dir.display().to_string();

    let (code, stderr) = bin(&[
        "update".to_string(),
        "--dir".to_string(),
        d.clone(),
        "--no-trust-size".to_string(),
        "--no-trust-mtime".to_string(),
    ]);
    assert_eq!(code, 0, "it carries on rather than erroring");
    assert!(
        stderr.contains("no-trust"),
        "and says why they do nothing here: {stderr}"
    );
    assert!(
        !recs_of(&dir)["a.txt"].hashes.is_empty(),
        "and the cache is populated, which is what the flags were asking for"
    );

    // Unflagged, no warning: the default run must stay silent about them.
    let (code, stderr) = bin(&["update".to_string(), "--dir".to_string(), d]);
    assert_eq!(code, 0);
    assert!(
        !stderr.contains("no-trust"),
        "an unflagged run must not mention them: {stderr}"
    );
}

/// Both flags compose on the command line, and each alone is accepted — the two are
/// independent, and "both is disable it" has to be reachable from the flags themselves.
#[test]
fn the_two_flags_are_independent_on_the_command_line() {
    let t = TempRoot::new("st_flags");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    let d = dir.display().to_string();
    for extra in [
        vec!["--no-trust-size".to_string()],
        vec!["--no-trust-mtime".to_string()],
        vec![
            "--no-trust-size".to_string(),
            "--no-trust-mtime".to_string(),
        ],
    ] {
        let mut args = vec!["update".to_string(), "--dir".to_string(), d.clone()];
        args.extend(extra.clone());
        assert_eq!(bin(&args).0, 0, "{extra:?} is accepted");
    }
}

/// They compose with the hash modes rather than conflicting with them: distrusting a
/// stat field is a statement about *evidence*, and asking for a digest by name is a
/// statement about *which* one.
#[test]
fn the_flags_compose_with_both_hash_modes() {
    let t = TempRoot::new("st_compose");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    let d = dir.display().to_string();
    for hash in [
        vec!["--hash-all-of".to_string(), "md5".to_string()],
        vec![
            "--hash-all-of".to_string(),
            "md5".to_string(),
            "--hash-all-of".to_string(),
            "sha256".to_string(),
        ],
    ] {
        let mut args = vec![
            "update".to_string(),
            "--dir".to_string(),
            d.clone(),
            "--no-trust-size".to_string(),
        ];
        args.extend(hash.clone());
        assert_eq!(bin(&args).0, 0, "{hash:?} with the flags");
    }
}

// -- no digest to fall back on ------------------------------------------------

/// Restore a file's mtime to a value captured earlier.
///
/// Load-bearing wherever a *rewrite* is part of the fixture: rewriting moves the mtime,
/// and a pair whose mtime also differs is settled by the field this run still trusts —
/// so the flag under test would widen nothing and the fixture would appear broken.
fn restore_mtime(p: &std::path::Path, t: std::time::SystemTime) {
    std::fs::OpenOptions::new()
        .write(true)
        .open(p)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

/// src and dst differing in length, with the mtime pinned so size is the only signal.
fn size_differs(t: &TempRoot) -> (std::path::PathBuf, std::path::PathBuf) {
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"short");
    wfile(
        &dst,
        "a.txt",
        b"a considerably longer body than the other side",
    );
    sync_mtime(&src.join("a.txt"), &dst.join("a.txt"));
    (src, dst)
}

/// **The false clean, in the form it actually takes.**
///
/// `--hash-all-of none` leaves `Required` empty, so `required.of(rel)` is `&[]` and
/// `hashes_differ` loops over nothing and returns `false` — the digest half of the
/// diff's predicate is *vacuous*, leaving `stat.settles` as the only thing standing
/// between the pair and a verdict. Distrust the very field the pair differs in and the
/// predicate is `false || false`: the path lands in none of `missing`/`extra`/
/// `type_conflict`/`changed`, and is reported **equal**.
///
/// Built through `cmd_compare` with a hand-assembled `CommonOpts` rather than through
/// the binary, because the binary refuses this combination — see
/// `no_digest_available_with_a_distrusted_field_is_refused`. This test covers the path
/// a refusal cannot reach: a caller that constructs `CommonOpts` directly, which is
/// every other test in this file and every library user.
#[test]
fn a_distrusted_field_with_no_digest_to_fall_back_on_is_never_reported_equal() {
    let t = TempRoot::new("st_unverifiable");
    let (src, dst) = size_differs(&t);
    let exit = |stat: StatTrust| {
        cmd_compare(
            girsync::CompareOpts {
                src: src.clone(),
                dst: dst.clone(),
                trust: Default::default(),
                common: distrusting(&[], stat),
            },
            &log(),
        )
        .unwrap()
    };
    assert_eq!(
        exit(StatTrust::default()),
        4,
        "no flags: size settles the pair, so CHANGED"
    );
    assert_eq!(
        exit(StatTrust::default().without_size()),
        4,
        "size distrusted and no digest left to decide it: the pair cannot be confirmed \
         identical, and `cannot be confirmed` is not `equal`"
    );
}

/// The mtime half of the same hole, which is the one that can hide a real edit: a file
/// touched but not rewritten is the most common way for two sides to agree on size and
/// differ on nothing.
#[test]
fn a_distrusted_mtime_with_no_digest_to_fall_back_on_is_never_reported_equal() {
    let t = TempRoot::new("st_unverifiable_mt");
    let (src, dst) = size_differs(&t);
    // Make the two sides byte-identical, then leave only the mtime differing.
    wfile(&dst, "a.txt", b"short");
    sync_mtime(&src.join("a.txt"), &dst.join("a.txt"));
    age(&dst, "a.txt", 60);
    let exit = |stat: StatTrust| {
        cmd_compare(
            girsync::CompareOpts {
                src: src.clone(),
                dst: dst.clone(),
                trust: Default::default(),
                common: distrusting(&[], stat),
            },
            &log(),
        )
        .unwrap()
    };
    assert_eq!(exit(StatTrust::default()), 4, "mtime settles it: CHANGED");
    assert_eq!(
        exit(StatTrust::default().without_mtime()),
        4,
        "mtime distrusted, nothing to replace it, so it stays CHANGED — the whole \
         point of the flag is that this pair is *unverified*, and an unverified pair \
         must not be reported equal"
    );
}

/// **`sync`, where the hole does real damage.** Before the fix this reported success
/// and left `dst` holding content it had just declared identical to `src`.
#[test]
fn sync_copies_a_pair_it_cannot_confirm_rather_than_declaring_it_equal() {
    let t = TempRoot::new("st_unverifiable_sync");
    let (src, dst) = size_differs(&t);
    cmd_sync(
        girsync::SyncOpts {
            src: src.clone(),
            dst: dst.clone(),
            trust: Default::default(),
            missing_only: false,
            keep_extra: false,
            jobs: 1,
            common: distrusting(&[], StatTrust::default().without_size()),
        },
        &log(),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(dst.join("a.txt")).unwrap(),
        b"short",
        "dst must hold src's bytes: not being able to confirm the pair is not a \
         reason to leave it stale"
    );
}

/// **`compare-self`, where the claim is about the tool's own cache being in step with
/// its own disk.** This is the most misleading form of the false clean, because the
/// audit is the thing a user trusts to tell them their cache is trustworthy.
#[test]
fn compare_self_does_not_claim_in_step_with_a_pair_it_cannot_verify() {
    let t = TempRoot::new("st_unverifiable_self");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"short");
    cmd_update(update(dir.clone()), &log()).unwrap();
    let stamped = std::fs::metadata(dir.join("a.txt"))
        .unwrap()
        .modified()
        .unwrap();
    wfile(&dir, "a.txt", b"a considerably longer body than before");
    restore_mtime(&dir.join("a.txt"), stamped);

    let run = |common: CommonOpts| {
        cmd_compare_self(
            girsync::CompareSelfOpts {
                dir: dir.clone(),
                no_trust_cached_hashes: false,
                common,
            },
            &log(),
        )
        .unwrap()
    };
    assert_eq!(
        run(distrusting(&[], StatTrust::default())),
        4,
        "no flags: size settles it, so the cache is reported out of step"
    );
    assert_eq!(
        run(distrusting(&[], StatTrust::default().without_size())),
        4,
        "size distrusted with nothing to replace it: the audit cannot verify the row, \
         and a self-audit that answers `in step` without verifying is the worst case \
         of this bug"
    );
}

// -- refusing the combination at the flag layer -------------------------------

/// **The refusal, through the real binary.**
///
/// The flags are a request to consult a digest. With `--hash-all-of none` there is no
/// digest to consult, so the request cannot be honoured: a distrusted difference would
/// have nothing to fall back on and the pair would be reported equal on no evidence.
/// That is a contradiction rather than a weaker guarantee, so it is refused — the same
/// call `parse_any_of` already makes for `none`, for the same reason.
#[test]
fn no_digest_available_with_a_distrusted_field_is_refused() {
    let t = TempRoot::new("st_refuse");
    let (src, dst) = size_differs(&t);
    let (s, d) = (src.display().to_string(), dst.display().to_string());
    for flags in [
        vec!["--no-trust-size".to_string()],
        vec!["--no-trust-mtime".to_string()],
        vec![
            "--no-trust-size".to_string(),
            "--no-trust-mtime".to_string(),
        ],
    ] {
        let mut args = vec![
            "compare".to_string(),
            "--src".to_string(),
            s.clone(),
            "--dst".to_string(),
            d.clone(),
            "--hash-all-of".to_string(),
            "none".to_string(),
            "--case-sensitive".to_string(),
        ];
        args.extend(flags.clone());
        let (code, stderr) = bin(&args);
        assert_ne!(
            code, 0,
            "{flags:?} with no digest cannot be honoured, so it must not exit 0 — and \
             exit 0 here is a false clean"
        );
        assert!(
            stderr.contains("no-trust"),
            "the message names the flag the user typed: {stderr}"
        );
        assert!(
            stderr.contains("hash-all-of"),
            "and the one that has to change for it to work: {stderr}"
        );
    }
}

/// **The refusal must be narrow, or it has simply traded a false clean for a
/// false alarm.** Every supported combination still runs, and — the part that matters —
/// the stat-only audit is untouched, because trusting both fields means the predicate
/// never becomes vacuous.
#[test]
fn the_refusal_leaves_every_supported_combination_alone() {
    let t = TempRoot::new("st_narrow");
    let (src, dst) = size_differs(&t);
    let (s, d) = (src.display().to_string(), dst.display().to_string());
    let base = |extra: &[String]| {
        let mut args = vec![
            "compare".to_string(),
            "--src".to_string(),
            s.clone(),
            "--dst".to_string(),
            d.clone(),
            "--case-sensitive".to_string(),
        ];
        args.extend(extra.iter().cloned());
        bin(&args)
    };
    // The stat-only audit, which is the mode `--hash-all-of none` exists for.
    let (code, stderr) = base(&["--hash-all-of".into(), "none".into()]);
    assert_eq!(
        code, 4,
        "and it still answers CHANGED on its own terms: {stderr}"
    );
    // A digest named alongside a distrusted field: the supported combination.
    let (code, stderr) = base(&[
        "--hash-all-of".into(),
        "md5".into(),
        "--no-trust-size".into(),
    ]);
    assert_eq!(
        code, 4,
        "size distrusted, digest decides: CHANGED: {stderr}"
    );
    // A distrusted field with the default request: likewise.
    let (code, stderr) = base(&["--no-trust-mtime".into()]);
    assert_eq!(
        code, 4,
        "mtime distrusted, md5 by default decides: {stderr}"
    );
}

/// `update` never pairs anything, so it has no verdict to get wrong — but it must
/// refuse on the same grounds, because the refusal is a property of the *request* and
/// not of the command that happens to receive it. Uniformity here is the point: a
/// script passing the flags to every subcommand should learn about it once, the same
/// way, whichever subcommand it hit first.
#[test]
fn the_refusal_is_uniform_across_subcommands() {
    let t = TempRoot::new("st_refuse_update");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    let (code, stderr) = bin(&[
        "update".to_string(),
        "--dir".to_string(),
        dir.display().to_string(),
        "--hash-all-of".to_string(),
        "none".to_string(),
        "--no-trust-size".to_string(),
    ]);
    assert_ne!(code, 0, "refused, rather than warned-and-ignored: {stderr}");
    assert!(
        stderr.contains("no-trust"),
        "with the same message every other subcommand gives: {stderr}"
    );
}
