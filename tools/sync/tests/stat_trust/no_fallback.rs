//! With no digest to fall back on.
//!
//! Distrusting a stat field asks the pair to be decided some *other* way. With nothing
//! else available, `unverifiable` is the honest verdict — "cannot tell", reported as
//! `CHANGED` rather than as equal.

use super::*;

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
    let t = temp_root("st_unverifiable");
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
    let t = temp_root("st_unverifiable_mt");
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

/// **`sync`, where the hole does real damage.** Without this rule a run would
/// report success and leave `dst` holding content it had just declared identical
/// to `src`.
#[test]
fn sync_copies_a_pair_it_cannot_confirm_rather_than_declaring_it_equal() {
    let t = temp_root("st_unverifiable_sync");
    let (src, dst) = size_differs(&t);
    cmd_sync(
        girsync::SyncOpts {
            src: src.clone(),
            dst: dst.clone(),
            trust: Default::default(),
            missing_only: false,
            keep_extra: false,
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
    let t = temp_root("st_unverifiable_self");
    let dir = t.subdir("w");
    wfile(&dir, "a.txt", b"short");
    cmd_update(update_mem(dir.clone()), &log()).unwrap();
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
