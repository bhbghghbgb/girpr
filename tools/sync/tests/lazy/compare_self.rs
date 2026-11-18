//! `compare-self`: what an audit reads, and what it concludes.
//!
//! The read counts here run opposite to what a reader guesses, and that is the
//! point rather than a surprise: `compare-self` hashes the disk side against
//! *nothing* (that is what makes it an audit), so a drifted tree is cheap — stat
//! settles the differing pairs — while a folder genuinely in step has every pair
//! undecided and must read every file.
//!
//! `audited` and `self_audit` live here rather than in the parent because they
//! drive this command's particular phase sequence, not `compare`'s.

use super::*;

/// One folder, `update`d, with `n_diff` of its files aged so their stat no
/// longer matches the record. The remaining `n_eq` stay stat-equal.
///
/// `update` is what makes the record describe the tree, so this is the state an
/// audit is run against: `n_eq` pairs that can only be settled by a digest, and
/// `n_diff` that size and mtime have already settled.
fn audited(tag: &str, n_eq: usize, n_diff: usize) -> (TempDir, PathBuf) {
    let t = temp_root(tag);
    let dir = t.subdir("w");
    for i in 0..n_eq {
        wfile(&dir, &format!("eq{i}.txt"), b"aaaa");
    }
    for i in 0..n_diff {
        wfile(&dir, &format!("df{i}.txt"), b"bbbb");
    }
    girsync::cmd_update(
        UpdateOpts {
            dir: dir.clone(),
            common: opts_mem(),
        },
        &log(),
    )
    .unwrap();
    for i in 0..n_diff {
        age(&dir, &format!("df{i}.txt"), 60);
    }
    (t, dir)
}

/// What one `compare-self` audit produced: its verdict and how much it read.
///
/// `cmd_compare_self` writes nothing by design, so there is no cache footprint to
/// read a counter out of — the only honest observable of how much was read is the
/// counter itself. `compare_self_detailed` is the command's own pipeline handed
/// back as data: the read-only record handle, the empty disk handle, the labels
/// and the `ScanMode`/`RwSide` threading all live there, not in a second copy here.
/// The command's wiring to it is pinned end-to-end by
/// `hidden_content_drift_is_reported_without_a_flag` and
/// `run_compare_self_reports_drift_with_record_as_src` in `tests/compare_self.rs`.
fn self_audit(dir: &Path) -> Run {
    let opts = girsync::CompareSelfOpts {
        dir: dir.to_path_buf(),
        no_trust_cached_hashes: false,
        common: opts_mem(),
    };
    let outcome = girsync::commands::compare_self_detailed(opts, &log())
        .expect("an audited folder is covered");
    Run {
        diff: outcome.diff,
        hashed: outcome.dst_stats.hashed,
        // The record side cannot hash from disk, so its digest *request* is the
        // honest "was it asked to hash" observable — the same count the harness
        // used to read off `plans.src.pending()`.
        src_hashed: outcome.src_digests,
        dst_hashed: outcome.dst_stats.hashed,
    }
}

/// **The win, and the one worth having.** Every pair differs in stat, so size and
/// mtime have already settled all of them and no digest is needed anywhere: an
/// audit that gave the disk side the record's cache would have read all sixteen
/// files to conclude things their sizes had decided for free.
#[test]
fn an_audit_where_every_pair_differs_in_stat_reads_nothing() {
    let t = temp_root("lz_self_alldiffer");
    let (_t, dir) = audited("lz_self_alldiffer", 0, 16);
    let r = self_audit(&dir);
    assert_eq!(
        (r.hashed, r.diff.changed.len()),
        (0, 16),
        "size settles every pair; all sixteen are CHANGED and none is read"
    );
    let _ = &t;
}

/// The largest win laziness has, and it is real — but only because the reads it
/// skips are the ones that were never needed, not the ones that carry the verdict.
/// Two stat-equal pairs remain undecided, so exactly two files are read.
#[test]
fn an_audit_of_a_snapshot_of_another_version_reads_only_the_undecided_pairs() {
    let (_t, dir) = audited("lz_self_snapshot", 2, 14);
    let r = self_audit(&dir);
    assert_eq!(
        (r.hashed, r.diff.changed.len()),
        (2, 14),
        "fourteen settled by stat, two needing a digest"
    );
}

/// **The reallocation, stated so it is not mistaken for an inefficiency.** An
/// audit of a folder that is genuinely in step has to read every file: the
/// record's stat agrees with disk, so the pair is undecided, and the only thing
/// that can settle it is a digest of the folder's bytes.
///
/// A high count here is the command working, not a cost to be optimised: the
/// disk side is hashed against nothing, so there is no cached digest it could
/// have settled the pair with instead.
#[test]
fn an_audit_of_an_in_step_folder_reads_every_undecided_pair() {
    let (_t, dir) = audited("lz_self_instep", 8, 0);
    let r = self_audit(&dir);
    assert_eq!(
        (r.hashed, r.diff.changed.len()),
        (8, 0),
        "eight undecided pairs, eight reads, and a clean report"
    );
}

/// Laziness moved where the reads happen and did not move a verdict. Same four
/// shapes as the three cases above plus the half-drifted one, with the `changed`
/// counts an *eager* audit would produce — every pair's verdict is decided by
/// stat, and stat is all an eager audit used for the differing pairs.
///
/// The `src_hashed` figures are the other half of the claim: the record side is
/// never asked for anything, because a record cannot hash from disk. A planner
/// that requested from it would report non-zero here.
#[test]
fn moving_the_reads_did_not_move_a_single_verdict() {
    for (tag, n_eq, n_diff, want_changed, want_hashed) in [
        ("lz_self_v_instp", 8, 0, 0, 8),
        ("lz_self_v_half", 8, 8, 8, 8),
        ("lz_self_v_all", 0, 16, 16, 0),
        ("lz_self_v_snap", 2, 14, 14, 2),
    ] {
        let (_t, dir) = audited(tag, n_eq, n_diff);
        let r = self_audit(&dir);
        assert_eq!(r.diff.changed.len(), want_changed, "{tag}: CHANGED");
        assert_eq!(r.hashed, want_hashed, "{tag}: reads");
        assert_eq!(r.src_hashed, 0, "{tag}: the record is never asked to hash");
    }
}
