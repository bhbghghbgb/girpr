//! The verdict: `--no-trust-mtime` is the flag that can change one.
//!
//! A file whose mtime moved but whose bytes did not is `CHANGED` on the strength of
//! the timestamp alone. Distrusting mtime is how you ask whether that is really so, and
//! it is the only one of the two flags whose answer can come back different.

use super::*;

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
    let t = temp_root("st_mtime_verdict");
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
    let t1 = temp_root("st_mtime_alone");
    let src1 = t1.subdir("one_src");
    let dst1 = t1.subdir("one_dst");
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
    let t = temp_root("st_size_verdict");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
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
