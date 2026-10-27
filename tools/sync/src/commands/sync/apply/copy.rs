//! The copy/verify machinery: `copy_one` (stream copy + digest + verify) and
//! its helpers `copy_and_digest`, `verify_digests`, and the test-only
//! `test_no_fsync` hook, plus their unit tests.

use anyhow::{Context, Result, bail};
use std::io::{Read, Write};
use std::path::Path;
use tracing::{error, trace};

use crate::cache::FileRec;
use crate::hash::{AlgoSet, CHUNK, DigestSet, Digests, HashAlg, hash_file};
use crate::util::mtime_ns_of;

/// Copy `src` over `dst`, digesting the bytes as they pass, and return the
/// digest of what was **read**.
///
/// The digest is of the source stream, which is the point: `copy_one` needs a
/// digest of src either way, and this is the one read that can produce it for
/// free. It is emphatically *not* a substitute for the check the caller does
/// afterwards — see [`verify_digests`], which is where the reasoning lives.
///
/// With no algorithms requested there is nothing to fold into the loop, so
/// `io::copy` is left alone and keeps whatever fast path it has between two
/// files. Under `--hash none` that path is the entire copy, and there is no
/// digest to justify giving it up.
fn copy_and_digest(
    src: &mut std::fs::File,
    dst: &mut std::fs::File,
    algos: &AlgoSet,
) -> Result<DigestSet> {
    if algos.is_empty() {
        std::io::copy(src, dst)?;
        return Ok(DigestSet::default());
    }
    let mut digests = Digests::new(algos);
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n: usize = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n])?;
        digests.update(&buf[..n]);
    }
    Ok(digests.finish())
}

/// Compare the two digests a copy produced, and fail if they disagree.
///
/// Two *separate* arguments, and that is the whole point. `src` is the digest
/// of the bytes the copy streamed; `dst` is a fresh read of the file on disk,
/// taken after `sync_all()`. A single pass that digested both sides from the
/// same chunks would hand this the same map twice and it would have nothing to
/// say — which is why the signature takes two, and why a caller that cannot
/// produce two has no verify to run at all.
///
/// Only the requested `algos` are judged. A digest the run did not ask for is
/// not evidence about this copy either way.
fn verify_digests(path: &Path, algos: &[HashAlg], src: &DigestSet, dst: &DigestSet) -> Result<()> {
    for a in algos {
        if src.get_named(a.as_str()) != dst.get_named(a.as_str()) {
            error!(algo = %a, path = %path.display(), "verify hash mismatch");
            bail!("verify hash({}) {}", a, path.display());
        }
    }
    Ok(())
}

/// Whether the test-only `GIRSYNC_TEST_FAIL_COPY` hook names `rel`, i.e. this
/// copy must fail before it writes anything.
///
/// Default **off**: an unset variable matches nothing. A test that needs a
/// mid-copy failure - to observe what the cache committed before the abort -
/// names one relative path and gets a deterministic failure in that copy only.
/// The failure is raised before the dst file is opened, so the fixture asserts
/// the cache's durability without also having to reason about a truncated file.
fn test_fail_copy(rel: &str) -> bool {
    static FAIL_REL: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    FAIL_REL
        .get_or_init(|| std::env::var("GIRSYNC_TEST_FAIL_COPY").ok())
        .as_deref()
        == Some(rel)
}

/// Whether the test-only `GIRSYNC_TEST_NO_FSYNC` hook is on.
///
/// Default **off**: an unset variable, or any value other than an unambiguous
/// truthy one (`1`, `true`, `yes`, `on`), leaves the per-file `sync_all` in
/// place. Production therefore always flushes a copied file before the verify
/// re-read. Tests opt in through `.config/nextest.toml` so the HDD copy suites
/// stop paying a flush per file; the value is read once per process, not once
/// per copied file. The bytes written, the size/mtime checks and the cache
/// records are unchanged, so content and reopen assertions still hold — the
/// tests merely stop exercising the `sync_all` itself.
fn test_no_fsync() -> bool {
    static NO_FSYNC: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *NO_FSYNC.get_or_init(|| {
        matches!(
            std::env::var("GIRSYNC_TEST_NO_FSYNC").as_deref(),
            Ok("1") | Ok("true") | Ok("yes") | Ok("on")
        )
    })
}

/// Copy one file src -> dst, preserving mtime, then verify before returning the
/// record to cache. `File::create` truncates in place, so a killed run can
/// leave a truncated file; the caller drops the cache entry first so a rerun
/// re-copies rather than trusting it.
///
/// **Verify is two independent reads, and both are load-bearing.** src is
/// digested as the copy streams it; dst is then re-read from disk and digested
/// again, and the two are compared. A single pass that digested both sides as
/// the bytes went past would compare a value with itself — equal by
/// construction, so `verify hash mismatch` could never fire and would read as a
/// guarantee while checking nothing. Because the re-read happens after
/// `sync_all()` (or, under the test-only hook above, after the write is
/// dropped), it also hashes the bytes that actually reached the disk, which is
/// the only way this catches silent corruption on the way down.
///
/// **What the src-side digest is not.** It describes the bytes src held *during*
/// the copy, not after it. So a src file rewritten mid-copy is not caught here:
/// dst is a faithful copy of a src that no longer exists, and the pair looks
/// clean. That is a race the tool cannot win anyway — phase A's stat sees the new
/// mtime on the next run and re-copies.
#[tracing::instrument(skip_all, fields(rel, bytes = tracing::field::Empty))]
pub(super) fn copy_one(
    src_root: &Path,
    dst_root: &Path,
    rel: &str,
    algos: &[HashAlg],
) -> Result<FileRec> {
    trace!("copy_one start");
    if test_fail_copy(rel) {
        bail!("test: injected copy failure for {rel}");
    }
    // A link *above* the destination is not ours to write through: the OS would
    // resolve it as an intermediate component, so `File::create` would truncate
    // a file in the target - possibly outside `dst`. The final component is
    // handled below, because a link *at* the path is the type conflict this copy
    // resolves.
    if let Some(link) = crate::link::link_ancestor_above(dst_root, rel)
        .with_context(|| format!("check link ancestors for {rel}"))?
    {
        bail!(
            "refusing to copy `{rel}` through dst link `{link}`: \
             remove the link, or exclude it with --exclude"
        );
    }
    let s = src_root.join(rel);
    let d = dst_root.join(rel);
    // dst dir that is in the way of a file copy: remove first.
    //
    // A link to a directory is *not* a directory to recurse into. Unlinking the
    // link is what resolves the type conflict toward src's kind (a file); the
    // target is somebody else's tree and is left alone. `is_dir` follows the
    // link, so it is true for a Unix directory symlink and for a Windows
    // junction, and `is_link` covers both plus a directory symlink.
    if d.is_dir() {
        if crate::link::try_is_link(&d).with_context(|| format!("check link {}", d.display()))? {
            crate::link::remove(&d).with_context(|| format!("unlink {}", d.display()))?;
        } else {
            std::fs::remove_dir_all(&d).with_context(|| format!("rmdir {}", d.display()))?;
        }
    }
    if let Some(parent) = d.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let smeta = std::fs::metadata(&s).with_context(|| format!("stat {}", s.display()))?;
    let ssize = smeta.len();
    let smtime_sys = smeta.modified().ok();
    let smtime_ns = mtime_ns_of(&smeta);
    // stream copy (truncate + write in place), digesting src as it goes
    let mut fin = std::fs::File::open(&s).with_context(|| format!("open {}", s.display()))?;
    // A dst symlink to a *file* is followed and written through here, by design
    // (the README documents write-through). Only a directory link is unlinked
    // above, because `File::create` cannot truncate one.
    let fout = std::fs::File::create(&d).with_context(|| format!("create {}", d.display()))?;
    // File::create truncates existing, and `copy_and_digest` writes in place.
    let mut fout = fout;
    // The request is a slice; what the digester wants is a set. One conversion per
    // copy, from a list of at most two, and it borrows nothing — the slice stays the
    // thing `verify_digests` walks below, because only the *requested* digests are
    // judged.
    let set = AlgoSet::from_iter(algos.iter().copied());
    let sh = copy_and_digest(&mut fin, &mut fout, &set)
        .with_context(|| format!("copy {} -> {}", s.display(), d.display()))?;
    if !test_no_fsync() {
        fout.sync_all().ok();
    }
    drop(fout);
    // preserve mtime
    //
    // **This is load-bearing, and its failure aborts the copy.** The bytes are
    // already durable at this point, so a `set_modified` failure leaves a correct
    // file with the wrong stamp: the run exits 3, and a rerun sees the mtime
    // difference and copies the file again. Fail-fast rather than a silent
    // mismatch, but "aborts an otherwise-good copy" is the documented shape
    // (README, `copy` step) - a test needs a platform-specific read-only/ACL
    // setup, so none injects this failure.
    if let Some(st) = smtime_sys {
        let f = std::fs::OpenOptions::new().write(true).open(&d)?;
        f.set_modified(st)
            .with_context(|| format!("set mtime {}", d.display()))?;
    }
    // verify: size + mtime + hash
    let dmeta = std::fs::metadata(&d).with_context(|| format!("stat {}", d.display()))?;
    if dmeta.len() != ssize {
        bail!(
            "verify size {} (want {} got {})",
            d.display(),
            ssize,
            dmeta.len()
        );
    }
    if algos.is_empty() {
        // size+mtime only
        let dmtime = mtime_ns_of(&dmeta);
        if dmtime != smtime_ns {
            bail!("verify mtime {}", d.display());
        }
        return Ok(FileRec {
            kind: "file".into(),
            size: ssize,
            mtime_ns: smtime_ns,
            hashes: DigestSet::default(),
        });
    }
    let dh = hash_file(&d, &set).with_context(|| format!("hash {}", d.display()))?;
    verify_digests(&d, algos, &sh, &dh)?;
    tracing::Span::current().record("bytes", ssize);
    trace!(bytes = ssize, "copy_one done");
    Ok(FileRec {
        kind: "file".into(),
        size: ssize,
        mtime_ns: smtime_ns,
        hashes: dh,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn algos(names: &[&str]) -> Vec<HashAlg> {
        names.iter().map(|s| s.parse().unwrap()).collect()
    }

    /// The same algorithms as a set, for the digester. `verify_digests` keeps
    /// taking the list: it walks the *request*, not the digests.
    fn aset(names: &[&str]) -> AlgoSet {
        AlgoSet::from_iter(algos(names))
    }

    /// The digest `copy_and_digest` returns has to be **the file's**, not the
    /// loop's. If the chunk boundary is off by one — a stale `n`, a slice taken
    /// before the read rather than after — the copy would still be byte-correct
    /// and every real verify would fail, which is the kind of failure that only
    /// shows up on a file larger than one buffer.
    #[test]
    fn the_digest_of_a_copy_is_the_digest_of_the_file() {
        let dir = girpr_tempdir::TempDir::new("girsync_copy", "digest");
        // Longer than `CHUNK`, so the loop really does iterate rather than
        // doing one read and one write.
        let body: Vec<u8> = (0..(CHUNK * 2 + 12345)).map(|i| (i % 251) as u8).collect();
        let src = dir.join("src.bin");
        let dst = dir.join("dst.bin");
        std::fs::write(&src, &body).unwrap();

        let set = aset(&["md5", "sha256"]);
        let mut fin = std::fs::File::open(&src).unwrap();
        let mut fout = std::fs::File::create(&dst).unwrap();
        let sh = copy_and_digest(&mut fin, &mut fout, &set).unwrap();
        drop(fout);

        assert_eq!(std::fs::read(&dst).unwrap(), body, "the bytes landed");
        assert_eq!(
            sh,
            hash_file(&src, &set).unwrap(),
            "the digest taken during the copy is the digest of the file, \
             however the copy loop split it"
        );
    }

    /// `--hash none` takes the other arm of `copy_and_digest`, and that arm is
    /// the only thing standing between a stat-only sync and `io::copy`'s fast
    /// path. It must still copy every byte — "no digest" is not "no copy" — and
    /// must report no digest rather than an empty one for some other reason.
    #[test]
    fn no_digest_requested_still_copies_every_byte() {
        let dir = girpr_tempdir::TempDir::new("girsync_copy", "noalgo");
        // Bigger than `CHUNK`, so "it went through the other arm and came out
        // whole" is a statement about more than one buffer.
        let body: Vec<u8> = (0..(CHUNK * 3 + 7)).map(|i| (i % 97) as u8).collect();
        let src = dir.join("src.bin");
        let dst = dir.join("dst.bin");
        std::fs::write(&src, &body).unwrap();

        let mut fin = std::fs::File::open(&src).unwrap();
        let mut fout = std::fs::File::create(&dst).unwrap();
        let sh = copy_and_digest(&mut fin, &mut fout, &AlgoSet::NONE).unwrap();
        drop(fout);

        assert!(
            sh.is_empty(),
            "nothing was requested, so nothing is reported"
        );
        assert_eq!(
            std::fs::read(&dst).unwrap(),
            body,
            "the bytes landed, through the arm that does not look at them"
        );
    }

    /// An empty file is where a hand-rolled copy loop and `io::copy` most easily
    /// disagree — the loop never enters its body at all.
    #[test]
    fn an_empty_file_copies_and_hashes() {
        let dir = girpr_tempdir::TempDir::new("girsync_copy", "empty");
        let src = dir.join("src.bin");
        let dst = dir.join("dst.bin");
        std::fs::write(&src, b"").unwrap();

        let set = aset(&["md5", "sha256"]);
        let mut fin = std::fs::File::open(&src).unwrap();
        let mut fout = std::fs::File::create(&dst).unwrap();
        let sh = copy_and_digest(&mut fin, &mut fout, &set).unwrap();
        drop(fout);

        assert_eq!(std::fs::read(&dst).unwrap(), Vec::<u8>::new());
        assert_eq!(sh, hash_file(&src, &set).unwrap());
    }

    /// One digest per algorithm, every one holding `byte` — so two maps built
    /// from the same call differ exactly where a caller makes them differ.
    fn digests(algos: &[&str], byte: u8) -> DigestSet {
        let mut set = DigestSet::default();
        for a in algos {
            set.insert_named(a, &[byte; 16]);
        }
        set
    }

    /// **This is the test the check exists for.** A verify that compares the two
    /// digests a copy produced has to be able to *fail* when they disagree, and
    /// the way to show that without arranging for a disk to corrupt itself is to
    /// hand the comparison two maps that disagree. Every copy in the suite is
    /// byte-correct, so without this the comparison could be written as a value
    /// against itself and pass every other test in the crate unchanged.
    ///
    /// The two stderr strings are contract, so they are asserted rather than
    /// left to inspection: an operator seeing a failed run is owed the
    /// algorithm and the path.
    #[test]
    fn a_disagreement_between_the_two_digests_is_a_failure() {
        let path = Path::new("dst/file.bin");
        // Same key set, one digest differs: the shape silent on-disk corruption
        // takes, and the only one a real copy can produce.
        let src = digests(&["md5", "sha256"], 1);
        let mut dst = src.clone();
        dst.insert_named("md5", &[2u8; 16]);

        let err = verify_digests(path, &algos(&["md5", "sha256"]), &src, &dst)
            .expect_err("two digests that disagree are not a verified copy");
        let msg = err.to_string();
        assert!(
            msg.contains("verify hash(md5)"),
            "names the algorithm: {msg}"
        );
        assert!(msg.contains("dst/file.bin"), "names the path: {msg}");
    }

    /// The other half of the pair, and the reason test 1 above is trustworthy:
    /// it fails for the disagreement it names rather than for anything about the
    /// maps, and it would fail identically if the comparison were inverted.
    /// `algos` is non-empty on purpose — an empty request has nothing to judge,
    /// and would make this pass for the wrong reason.
    #[test]
    fn agreement_is_not_a_failure() {
        let both = digests(&["md5", "sha256"], 1);
        assert!(
            verify_digests(
                Path::new("dst/file.bin"),
                &algos(&["md5", "sha256"]),
                &both,
                &both
            )
            .is_ok(),
            "two reads of the same bytes agreeing is the whole point"
        );
    }

    /// A digest one side has and the other does not is **not** agreement. It is
    /// the case a loop written as `for (k, v) in src { if dst.get(k) != Some(v) }`
    /// skips silently, because it walks the map instead of the request — so the
    /// missing entry is never looked up at all. `src.get(a) != dst.get(a)` sees
    /// `Some(..) != None`.
    #[test]
    fn a_digest_that_only_one_side_has_is_a_failure() {
        let src = digests(&["md5"], 1);
        let dst = DigestSet::default();

        let err = verify_digests(Path::new("dst/file.bin"), &algos(&["md5"]), &src, &dst)
            .expect_err("a digest dst never produced is not one it agrees with");
        assert!(
            err.to_string().contains("verify hash(md5)"),
            "the missing digest is the one reported: {err}"
        );
    }

    /// Only what the run asked for is evidence about this copy. A digest nobody
    /// requested is not a disagreement, and treating it as one would fail every
    /// copy whose maps happen to carry more than `algos` names.
    ///
    /// This pins which side the loop walks: `algos`, not the maps. A comparison
    /// iterating the digests instead would fail here, having found a real
    /// difference in a value no caller asked about.
    #[test]
    fn only_the_requested_algorithms_are_judged() {
        let src = digests(&["md5", "sha256"], 1);
        let mut dst = src.clone();
        dst.insert_named("sha256", &[2u8; 16]);

        assert!(
            verify_digests(Path::new("dst/file.bin"), &algos(&["md5"]), &src, &dst).is_ok(),
            "sha256 was not requested, so its disagreement is not this copy's problem"
        );
    }
}
