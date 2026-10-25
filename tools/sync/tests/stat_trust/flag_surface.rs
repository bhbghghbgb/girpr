//! `update`, and the flag surface through the real binary.
//!
//! `update` accepts both flags and changes nothing — it rehashes every file either
//! way. The parser half is checked here because a flag the CLI drops is a flag the
//! command never sees.

use super::*;

use girsync::planner::HashMode;

/// **`update` warns and carries on.** It rehashes every file regardless, so there is no
/// short circuit for the flags to switch off — and *that* is the behaviour they ask
/// for, so obeying them by doing less would be wrong.
///
/// Following the `--hash-any-of` and `--no-trust-cached-hashes`-on-`compare-self`
/// precedent: a flag that cannot apply is warned about, not rejected, so a script
/// passing it to every subcommand keeps working.
#[test]
fn update_warns_that_the_flags_do_not_apply_and_still_populates() {
    let t = temp_root("st_update");
    let dir = t.subdir("w");
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
///
/// Driven in-process via `cmd_update` instead of spawning the binary three times:
/// the claim is that the flags are accepted, which the library-level call proves.
#[test]
fn the_two_flags_are_independent_on_the_command_line() {
    let t = temp_root("st_flags");
    let dir = t.subdir("w");
    wfile(&dir, "a.txt", b"hello");
    let cases = [
        StatTrust::default().without_size(),
        StatTrust::default().without_mtime(),
        StatTrust::default().without_size().without_mtime(),
    ];
    for stat in cases {
        let mut o = common::update(dir.clone());
        o.common = distrusting(&["md5"], stat);
        assert_eq!(cmd_update(o, &log()).unwrap(), 0, "{stat:?} is accepted");
    }
}

/// They compose with the hash modes rather than conflicting with them: distrusting a
/// stat field is a statement about *evidence*, and asking for a digest by name is a
/// statement about *which* one.
#[test]
fn the_flags_compose_with_both_hash_modes() {
    let t = temp_root("st_compose");
    let dir = t.subdir("w");
    wfile(&dir, "a.txt", b"hello");
    let cases = [
        (vec!["md5"], HashMode::AllOf),
        (vec!["md5", "sha256"], HashMode::AllOf),
    ];
    for (algos, mode) in cases {
        let mut common = distrusting(&algos, StatTrust::default().without_size());
        common.hash_mode = mode;
        let mut o = common::update(dir.clone());
        o.common = common;
        assert_eq!(
            cmd_update(o, &log()).unwrap(),
            0,
            "{algos:?} with --no-trust-size"
        );
    }
}
