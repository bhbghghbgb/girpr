//! Refusing the combination at the flag layer.
//!
//! `--hash-all-of none` with either distrust flag is refused rather than warned: the
//! pair would be reported equal with nobody having compared its content.

use super::*;

/// **The refusal, through the real binary.**
///
/// The flags are a request to consult a digest. With `--hash-all-of none` there is no
/// digest to consult, so the request cannot be honoured: a distrusted difference would
/// have nothing to fall back on and the pair would be reported equal on no evidence.
/// That is a contradiction rather than a weaker guarantee, so it is refused — the same
/// call `parse_any_of` already makes for `none`, for the same reason.
///
/// One fixture is shared across all three flag spellings so the temp dir + cache are
/// created once rather than per variant.
#[test]
fn no_digest_available_with_a_distrusted_field_is_refused() {
    let t = temp_root("st_refuse");
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
///
/// Driven in-process via `cmd_compare` instead of spawning the binary three times:
/// the claim is that the supported combinations still produce a verdict.
#[test]
fn the_refusal_leaves_every_supported_combination_alone() {
    let t = temp_root("st_narrow");
    let (src, dst) = size_differs(&t);
    // The stat-only audit: `--hash-all-of none`, no distrust — size settles it.
    assert_eq!(
        cmd_compare(
            girsync::CompareOpts {
                src: src.clone(),
                dst: dst.clone(),
                trust: Default::default(),
                common: distrusting(&[], StatTrust::default()),
            },
            &log(),
        )
        .unwrap(),
        4,
        "size differs, so CHANGED"
    );
    // A digest named alongside a distrusted field: md5 decides, content differs.
    assert_eq!(
        cmd_compare(
            girsync::CompareOpts {
                src: src.clone(),
                dst: dst.clone(),
                trust: Default::default(),
                common: distrusting(&["md5"], StatTrust::default().without_size()),
            },
            &log(),
        )
        .unwrap(),
        4,
        "size distrusted, digest decides: CHANGED"
    );
    // A distrusted field with the default request: md5 by default decides.
    assert_eq!(
        cmd_compare(
            girsync::CompareOpts {
                src: src.clone(),
                dst: dst.clone(),
                trust: Default::default(),
                common: distrusting(&["md5"], StatTrust::default().without_mtime()),
            },
            &log(),
        )
        .unwrap(),
        4,
        "mtime distrusted, md5 decides: CHANGED"
    );
}

/// `update` never pairs anything, so it has no verdict to get wrong — but it must
/// refuse on the same grounds, because the refusal is a property of the *request* and
/// not of the command that happens to receive it. Uniformity here is the point: a
/// script passing the flags to every subcommand should learn about it once, the same
/// way, whichever subcommand it hit first.
#[test]
fn the_refusal_is_uniform_across_subcommands() {
    let t = temp_root("st_refuse_update");
    let dir = t.subdir("w");
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
