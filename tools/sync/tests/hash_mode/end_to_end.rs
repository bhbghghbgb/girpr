//! End to end: `any-of` as the remedy a stricter mode could only name.
//!
//! A record holding one algorithm fails an all-of `compare` and passes an any-of
//! one against the same folder, with the same fixture.

use super::*;

/// **`any-of` is the remedy the all-of error can only name.** A record that
/// holds md5 fails an all-of `compare` against a folder and passes an any-of one,
/// with the same fixture and the same folder.
#[test]
fn any_of_answers_a_record_that_a_single_algorithm_run_would_reject() {
    let t = temp_root("hm_any_of_record");
    let rec_home = t.subdir("rec");
    let folder = t.subdir("live");
    for dir in [&rec_home, &folder] {
        wfile(dir, "a.txt", b"hello");
    }
    sync_mtime(&rec_home.join("a.txt"), &folder.join("a.txt"));
    // The record holds md5 only, which is the shape the coverage error is about:
    // one of the two requested algorithms, and not the other.
    cmd_update(
        girsync::UpdateOpts {
            dir: rec_home.clone(),
            common: asking(&["md5"], HashMode::AllOf),
        },
        &log(),
    )
    .unwrap();
    cmd_update(
        girsync::UpdateOpts {
            dir: folder.clone(),
            common: asking(&["md5", "sha256"], HashMode::AllOf),
        },
        &log(),
    )
    .unwrap();
    let record = recs_of(&rec_home);
    assert!(
        record["a.txt"].hashes.get_named("md5").is_some()
            && record["a.txt"].hashes.get_named("sha256").is_none(),
        "the fixture is a one-algorithm record: {:?}",
        digests_of(&rec_home, "a.txt")
    );

    let err = cmd_compare(
        girsync::CompareOpts {
            src: rec_home.join(CACHE_PREFIX),
            dst: folder.clone(),
            trust: Default::default(),
            common: asking(&["md5", "sha256"], HashMode::AllOf),
        },
        &log(),
    )
    .expect_err("all-of cannot be answered from one algorithm");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("sha256"),
        "and it names what is missing: {msg}"
    );
    // The single algorithm that covers is offered, and `any-of` is **not** offered
    // alongside it. Here `--hash-any-of md5 --hash-any-of sha256` would also work —
    // the record holds one of the two — but `--hash-all-of md5` asks for strictly
    // less for the same coverage, so offering both would leave the user choosing
    // between two flags with no stated preference. The `any-of` line is for the case
    // where *nothing* single covers, which is the case where widening the mode is
    // the only move left.
    assert!(
        msg.contains("--hash-all-of md5"),
        "offers the one algorithm the record already holds: {msg}"
    );
    assert!(
        !msg.contains("--hash-any-of"),
        "and does not offer a broader request when a narrower one works: {msg}"
    );

    assert_eq!(
        cmd_compare(
            girsync::CompareOpts {
                src: rec_home.join(CACHE_PREFIX),
                dst: folder.clone(),
                trust: Default::default(),
                common: asking(&["md5", "sha256"], HashMode::AnyOf),
            },
            &log(),
        )
        .unwrap(),
        0,
        "and any-of asks for the one the record has, so the pair is answerable"
    );
}

/// **`update` stores every algorithm it was asked for, in either mode.** Under
/// any-of the flag has nothing to do — there is no counterpart to choose between,
/// and a record that stored only one algorithm would make the next *comparison*
/// fail. It warns and stores both, following the `--no-trust-cached-hashes` /
/// `compare-self` precedent: a script passing the flag everywhere keeps working.
#[test]
fn update_populates_every_algorithm_under_any_of() {
    let t = temp_root("hm_update_any_of");
    let dir = t.subdir("w");
    wfile(&dir, "a.txt", b"hello");
    cmd_update(
        girsync::UpdateOpts {
            dir: dir.clone(),
            common: asking_mem(&["md5", "sha256"], HashMode::AnyOf),
        },
        &log(),
    )
    .unwrap();
    let got = &recs_of(&dir)["a.txt"].hashes;
    assert!(
        got.get_named("md5").is_some() && got.get_named("sha256").is_some(),
        "both, because a record stores rather than chooses: {got:?}"
    );
}

/// **`xxhash64` is a supported flag value, not a typo that survives as a foreign
/// row.** Both flags accept it, `update` stores it in the eight-byte typed slot,
/// and the bytes are the published XXH64 vector for an empty file.
#[test]
fn xxhash64_is_accepted_by_both_flags_and_stored_as_a_digest() {
    let t = temp_root("hm_xxhash64_flag");
    let dir = t.subdir("w");
    wfile(&dir, "a.txt", b"");
    let d = dir.display().to_string();

    for flag in ["--hash-all-of", "--hash-any-of"] {
        let (code, stderr) = bin(&["update", "--dir", &d, flag, "xxhash64"]);
        assert_eq!(code, 0, "{flag} accepts xxhash64: {stderr}");
    }

    let got = &recs_of(&dir)["a.txt"].hashes;
    assert_eq!(
        got.get_algo(HashAlg::Xxhash64),
        Some(&[0xef, 0x46, 0xdb, 0x37, 0x51, 0xd8, 0xe9, 0x99][..]),
        "the empty-file XXH64 vector, as raw big-endian bytes"
    );
    assert!(
        got.get_algo(HashAlg::Md5).is_none(),
        "only xxhash64 was asked for"
    );
}

/// One in-process invocation: exit code and the error text a caller reads on
/// stderr — clap's parse-error text or the `anyhow` string `main` logs as fatal.
fn bin(args: &[&str]) -> (i32, String) {
    common::run_error_text(args)
}

/// **`--hash-any-of none` is rejected.** "Any of the requested algorithms", with
/// nothing requested, is not a weaker request — it is no request, and `--hash-all-of
/// none` is the flag that says so. Accepting it would give a stat-only audit a
/// name that claims a digest was optional.
#[test]
fn any_of_with_none_is_rejected() {
    let t = temp_root("hm_none");
    let dir = t.subdir("w");
    wfile(&dir, "a.txt", b"hello");
    let d = dir.display().to_string();
    let (code, stderr) = bin(&["update", "--dir", &d, "--hash-any-of", "none"]);
    assert_ne!(code, 0, "a stat-only audit is --hash-all-of none");
    assert!(
        stderr.contains("--hash-all-of none"),
        "and the error says which flag does it: {stderr}"
    );
}

/// **The two modes are mutually exclusive**, checked at the flag layer so the
/// answer is a usage error rather than a run that quietly picked one.
///
/// Clap's conflict check ignores defaulted values, so `--hash-all-of`'s `md5`
/// default does not collide with `--hash-any-of` on its own — which is the only
/// reason this can be a clap conflict at all rather than a hand-written check.
#[test]
fn the_two_modes_cannot_be_asked_for_at_once() {
    let t = temp_root("hm_both");
    let dir = t.subdir("w");
    wfile(&dir, "a.txt", b"hello");
    let d = dir.display().to_string();
    let (code, stderr) = bin(&[
        "update",
        "--dir",
        &d,
        "--hash-all-of",
        "md5",
        "--hash-any-of",
        "sha256",
    ]);
    assert_ne!(code, 0, "the run would have to pick one");
    assert!(
        stderr.contains("hash-all-of") && stderr.contains("hash-any-of"),
        "clap names both: {stderr}"
    );

    // And each alone is fine, which is the other half of the claim: the default
    // on `--hash-all-of` must not make every invocation a conflict.
    assert_eq!(
        bin(&["update", "--dir", &d, "--hash-any-of", "sha256"]).0,
        0
    );
    assert_eq!(bin(&["update", "--dir", &d]).0, 0);
}

/// `--hash` is not an accepted spelling of `--hash-all-of`. It is rejected by name
/// rather than silently ignored, so a script carrying it says so instead of quietly
/// running with no digests at all.
#[test]
fn the_short_hash_spelling_is_rejected_by_name() {
    let t = temp_root("hm_old");
    let dir = t.subdir("w");
    wfile(&dir, "a.txt", b"hello");
    let d = dir.display().to_string();
    let (code, stderr) = bin(&["update", "--dir", &d, "--hash", "md5"]);
    assert_ne!(code, 0, "not silently accepted");
    assert!(
        stderr.contains("--hash"),
        "and clap says which flag it did not recognise: {stderr}"
    );
    // Clap's suggestion is what a user who types `--hash` will actually read, so
    // assert the suggestion rather than restating the rule: `--hash` must not be
    // accepted as an alias for anything.
    assert!(
        stderr.contains("--hash-all-of") || stderr.contains("--hash-any-of"),
        "and it points at one of the two names it could mean: {stderr}"
    );
}
