//! The copy/verify machinery: `copy_one` (stream copy + digest + verify) and
//! its helpers — the destination guards, `copy_and_digest`, `verify_digests`,
//! and the test-only `test_no_fsync` hook — plus their unit tests.

use anyhow::{Context, Result, bail};
use std::io::{self, Write};
use std::path::Path;
use tracing::{error, trace};

use crate::cache::FileRec;
use crate::config::TestHooks;
use crate::hash::{AlgoSet, CHUNK, DigestSet, Digests, hash_file};
use crate::util::{mtime_ns_of, path_within};

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
/// files. Under `--hash-all-of none` that path is the entire copy, and there is no
/// digest to justify giving it up.
///
/// `buf` is the caller's scratch space: a worker hands in the same buffer for
/// every file it copies, so the chunk allocation and zero-fill happen once per
/// worker rather than once per file. It is resized only on the digesting path;
/// the `io::copy` path leaves it untouched. Only `buf[..n]` is ever read back,
/// because the tail beyond the last read may still hold an earlier chunk.
fn copy_and_digest(
    src: &mut std::fs::File,
    dst: &mut std::fs::File,
    algos: &AlgoSet,
    buf: &mut Vec<u8>,
) -> Result<DigestSet> {
    if algos.is_empty() {
        std::io::copy(src, dst)?;
        return Ok(DigestSet::default());
    }
    let mut digests = Digests::new(algos);
    buf.resize(CHUNK, 0);
    // The shared loop writes each chunk before it absorbs it; the `io::copy`
    // fast path above stays here, and so does the choice of destination.
    digests.absorb_reader(src, buf.as_mut_slice(), |chunk| dst.write_all(chunk))?;
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
/// Only the algorithms in `algos` are judged, and they are the ones this copy
/// computed: a digest the run did not ask for is not evidence about this copy
/// either way, and one the copy did not produce could not be compared at all.
/// The set carries no order of its own, so a disagreement is named in canonical
/// order — the order the diff's `digest-differs` tag uses as well.
fn verify_digests(path: &Path, algos: &AlgoSet, src: &DigestSet, dst: &DigestSet) -> Result<()> {
    for a in algos.iter() {
        if src.get_algo(a) != dst.get_algo(a) {
            error!(algo = %a, path = %path.display(), "verify hash mismatch");
            bail!("verify hash({}) {}", a, path.display());
        }
    }
    Ok(())
}

/// Flush one copied file, or return the injected test failure.
///
/// The result is **not** discarded: a failed flush must not be allowed to pass
/// the verify below (which would re-read the same OS cache), cache a digest, and
/// let the run exit 0 claiming durability. The caller turns this into the run
/// error (exit 3), and the pre-drop has already removed the row, so a rerun
/// recopies.
///
/// `hooks.fail_flush` names the relative path whose flush must fail — the same
/// fault the old `GIRSYNC_TEST_FAIL_FLUSH` variable injected, but carried as
/// data so an in-process caller can reach it without a child.
fn flush_file(fout: &std::fs::File, rel: &str, hooks: &TestHooks) -> io::Result<()> {
    if hooks.fail_flush.as_deref() == Some(rel) {
        return Err(io::Error::other(format!(
            "test: injected flush failure for {rel}"
        )));
    }
    fout.sync_all()
}

/// Refuse a copy whose destination is reached **through** a link.
///
/// A link *above* the destination is not ours to write through: the OS would
/// resolve it as an intermediate component, so `File::create` would truncate a
/// file in the target - possibly outside `dst`. The final component is handled
/// by [`refuse_final_link_into_src`], because a link *at* the path is the type
/// conflict this copy resolves.
fn refuse_link_ancestor(dst_root: &Path, rel: &str) -> Result<()> {
    if let Some(link) = crate::link::link_ancestor_above(dst_root, rel)
        .with_context(|| format!("check link ancestors for {rel}"))?
    {
        bail!(
            "refusing to copy `{rel}` through dst link `{link}`: \
             remove the link, or exclude it with --exclude"
        );
    }
    Ok(())
}

/// Refuse a copy whose destination is a link that resolves into `src`.
///
/// Belt and braces for the final component, symmetric with the ancestor check
/// above and with `validate_dst_links` at plan time. A link *at* this path is
/// normally replaceable (a directory link) or written through (a file link),
/// but not when it resolves into src: a file link there would make
/// `File::create` truncate a source file. Ask the disk again, because the
/// plan was computed before this run changed anything. A missing path is not
/// a link, and a dangling one fails to resolve rather than being written
/// through.
fn refuse_final_link_into_src(d: &Path, rel: &str, src_canonical: &Path) -> Result<()> {
    if crate::link::try_is_link(d).with_context(|| format!("check link {}", d.display()))? {
        let target = girpr_fs_names::canonical_no_verbatim(d)
            .with_context(|| format!("resolve dst link `{rel}`"))?;
        if path_within(&target, src_canonical) {
            bail!(
                "src and dst must be disjoint: dst link `{rel}` resolves to `{}`, \
                 which is inside src `{}`; refusing to copy through it (remove the \
                 link, or exclude it with --exclude)",
                target.display(),
                src_canonical.display()
            );
        }
        if path_within(src_canonical, &target) {
            bail!(
                "src and dst must be disjoint: dst link `{rel}` resolves to `{}`, \
                 which contains src `{}`; refusing to copy through it (remove the \
                 link, or exclude it with --exclude)",
                target.display(),
                src_canonical.display()
            );
        }
    }
    Ok(())
}

/// Remove a directory that is in the way of a file copy.
///
/// A link to a directory is *not* a directory to recurse into. Unlinking the
/// link is what resolves the type conflict toward src's kind (a file); the
/// target is somebody else's tree and is left alone. `is_dir` follows the
/// link, so it is true for a Unix directory symlink and for a Windows
/// junction, and `is_link` covers both plus a directory symlink.
fn clear_blocking_dir(d: &Path) -> Result<()> {
    if d.is_dir() {
        if crate::link::try_is_link(d).with_context(|| format!("check link {}", d.display()))? {
            crate::link::remove(d).with_context(|| format!("unlink {}", d.display()))?;
        } else {
            std::fs::remove_dir_all(d).with_context(|| format!("rmdir {}", d.display()))?;
        }
    }
    Ok(())
}

/// Make the destination's parent directories.
///
/// The copy's own path is never a directory, so the parents are all it needs;
/// `create_dir_all` is happy with the ones that are already there.
fn create_parent(d: &Path) -> Result<()> {
    if let Some(parent) = d.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    Ok(())
}

/// Refuse a destination that is a second name for another file.
///
/// A hard link is a second name for this file, not a link this crate can
/// classify: `try_is_link` is false and the walk reports it as ordinary
/// content. `File::create` truncates the shared inode, so writing here would
/// change the other name too, which may be outside `dst` and in the reported
/// shape is a source file the run is reading. Refuse conservatively rather
/// than silently break the mirror's "never touch src" promise; the user can
/// remove the extra link and rerun.
///
/// The count is the complete check: comparing the destination's identity with
/// this copy's source misses a dst hard link to a *different* src file, which
/// is exactly the reported shape (`dst/a.txt` linked to `src/b.txt` while
/// copying `src/a.txt`).
///
/// A missing destination has nothing to truncate and is allowed through; any
/// other error is reported rather than read as a count.
fn refuse_hard_linked_dst(d: &Path, rel: &str) -> Result<()> {
    match crate::hardlink::count(d) {
        Ok(1) => Ok(()),
        Ok(links) => bail!(
            "refusing to copy `{rel}`: dst `{}` has {links} hard links, so truncating \
             it would change another path that shares the file (possibly one inside \
             src); remove the extra link or replace the file, then rerun",
            d.display()
        ),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("hard-link count {}", d.display())),
    }
}

/// What the verify judges the destination against.
///
/// The size and the mtime are the source's own, taken before the copy, and
/// `src_digests` is the digest of the bytes the copy streamed out of the
/// source. The verify holds all three against what it reads back from the
/// destination.
struct VerifyInputs {
    size: u64,
    mtime_ns: i64,
    src_digests: DigestSet,
}

/// Copy the source over the destination, flush it, and stamp it with the
/// source's mtime.
///
/// The three steps are in that order, and each is load-bearing:
///
/// - **copy**: the source is stat'ed first and opened second, so the size and
///   the mtime the verify uses are the ones the copy actually read;
/// - **flush**: the bytes are written through, so the verify's re-read hashes
///   what reached the disk rather than the page cache. A failed flush is a
///   failed copy: `sync_all` is propagated, not ignored, or the verify could
///   re-read the same cache the flush never wrote through, cache the digest,
///   and let the run exit 0 claiming durability it did not obtain;
/// - **stamp**: the mtime follows the write and precedes the close, and it is
///   applied to the handle that wrote the bytes, so there is no window in
///   which the file is closed but unstamped. Its failure aborts the copy: the
///   bytes are already durable, so the run exits 3 and a rerun sees the mtime
///   difference and copies the file again.
///
/// The digest of the source comes from the copy's own read, which is why it
/// is free; it is *not* the verify, which re-reads the destination
/// separately.
fn copy_and_stamp(
    s: &Path,
    d: &Path,
    rel: &str,
    algos: &AlgoSet,
    buf: &mut Vec<u8>,
    hooks: &TestHooks,
) -> Result<VerifyInputs> {
    let smeta = std::fs::metadata(s).with_context(|| format!("stat {}", s.display()))?;
    let ssize = smeta.len();
    let smtime_sys = smeta.modified().ok();
    let smtime_ns = mtime_ns_of(&smeta);
    // stream copy (truncate + write in place), digesting src as it goes
    let mut fin = std::fs::File::open(s).with_context(|| format!("open {}", s.display()))?;
    // A dst symlink to a *file* is followed and written through here, by design
    // (the README documents write-through). Only a directory link is unlinked
    // above, because `File::create` cannot truncate one.
    let mut fout = std::fs::File::create(d).with_context(|| format!("create {}", d.display()))?;
    // The digester wants a set, and so does the verify: the same one. It is
    // built once per phase and handed down, so the conversion from the run's
    // request is not repeated per file, and the digests computed and the
    // digests judged are the same set by construction.
    let src_digests = copy_and_digest(&mut fin, &mut fout, algos, buf)
        .with_context(|| format!("copy {} -> {}", s.display(), d.display()))?;
    if !hooks.no_fsync {
        flush_file(&fout, rel, hooks).with_context(|| format!("flush {}", d.display()))?;
    }
    if let Some(st) = smtime_sys {
        let ft = filetime::FileTime::from_system_time(st);
        // Still stamped on the handle that wrote the bytes: the flush above has
        // made them durable, and the handle closes only after this succeeds.
        #[allow(deprecated)] // filetime 0.2.30 defers these setters to std
        filetime::set_file_handle_times(&fout, None, Some(ft))
            .with_context(|| format!("set mtime {}", d.display()))?;
    }
    drop(fout);
    Ok(VerifyInputs {
        size: ssize,
        mtime_ns: smtime_ns,
        src_digests,
    })
}

/// Copy one file src -> dst, preserving mtime, then verify before returning the
/// record to cache. `File::create` truncates in place, so a killed run can
/// leave a truncated file; the caller drops the cache entry first so a rerun
/// re-copies rather than trusting it.
///
/// **The body is an ordered pipeline — guard, prepare, copy, flush, stamp,
/// verify — and the order is load-bearing.** Every guard is a named helper, so
/// the shape is visible without reconstructing it from interleaved control
/// flow:
///
/// 1. [`refuse_link_ancestor`] — a link *above* the destination is not ours to
///    write through;
/// 2. [`refuse_final_link_into_src`] — a link *at* the destination that
///    resolves into `src` would be written through by `File::create`;
/// 3. [`clear_blocking_dir`] — a directory in the way of a file copy goes
///    first, and a link to a directory is unlinked rather than recursed into;
/// 4. [`create_parent`] — the destination's parents are made;
/// 5. [`refuse_hard_linked_dst`] — a destination with more than one hard link
///    is refused before `File::create` truncates the shared inode;
/// 6. [`copy_and_stamp`] — the source is stat'ed, streamed over the
///    destination and digested as it goes, flushed, and stamped with the
///    source's mtime;
/// 7. the verify, below, which re-reads the destination from disk.
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
/// **A failed flush is a failed copy.** `sync_all` is propagated, not ignored:
/// otherwise the verify below could re-read the same page cache the flush never
/// wrote through, cache the digest, and let the run exit 0 claiming durability
/// it did not obtain.
///
/// **A destination link into `src` is refused before anything is unlinked or
/// created.** `src_canonical` is the resolved source root; a link at the final
/// component that resolves inside it (or contains it) would otherwise be written
/// through by `File::create`, truncating a source file while the verify read the
/// same file back through the link. `validate_dst_links` refuses that shape at
/// plan time; [`refuse_final_link_into_src`] is the per-copy re-check, so a plan
/// and a disk that disagree still cannot reach the source.
///
/// **A destination with more than one hard link is refused for the same reason.**
/// The other name is the same file, so `File::create` would truncate it too, and
/// the verify would read the changed inode back and agree with itself. The link
/// count is the complete check: comparing `d`'s identity with this copy's source
/// misses a dst hard link to a *different* src file, which is exactly the
/// reported shape (`dst/a.txt` linked to `src/b.txt` while copying `src/a.txt`).
/// Refusing every count above one is conservative — a shared inode is outside a
/// mirror's write contract wherever it lives. The check is
/// [`refuse_hard_linked_dst`].
///
/// **What the src-side digest is not.** It describes the bytes src held *during*
/// the copy, not after it. So a src file rewritten mid-copy is not caught here:
/// dst is a faithful copy of a src that no longer exists, and the pair looks
/// clean. That is a race the tool cannot win anyway — phase A's stat sees the new
/// mtime on the next run and re-copies.
///
/// **One algorithm argument, and it is the set.** `algos` says which digests
/// this copy computes, and the verify judges exactly those: a digest the run
/// did not ask for is not evidence about this copy, and one the copy did not
/// produce could not be compared anyway. The set is the same for every copy in
/// a phase, so the caller builds it once from the run's request and passes it
/// down — and because it is the only algorithm argument, the digests computed
/// and the digests judged cannot drift apart.
#[tracing::instrument(skip_all, fields(rel, bytes = tracing::field::Empty))]
pub(super) fn copy_one(
    src_root: &Path,
    src_canonical: &Path,
    dst_root: &Path,
    rel: &str,
    algos: AlgoSet,
    buf: &mut Vec<u8>,
    hooks: &TestHooks,
) -> Result<FileRec> {
    trace!("copy_one start");
    if hooks.fail_copy.as_deref() == Some(rel) {
        bail!("test: injected copy failure for {rel}");
    }
    refuse_link_ancestor(dst_root, rel)?;
    let s = src_root.join(rel);
    let d = dst_root.join(rel);
    refuse_final_link_into_src(&d, rel, src_canonical)?;
    clear_blocking_dir(&d)?;
    create_parent(&d)?;
    refuse_hard_linked_dst(&d, rel)?;
    let inputs = copy_and_stamp(&s, &d, rel, &algos, buf, hooks)?;
    // verify: size + mtime + hash
    let dmeta = std::fs::metadata(&d).with_context(|| format!("stat {}", d.display()))?;
    if dmeta.len() != inputs.size {
        bail!(
            "verify size {} (want {} got {})",
            d.display(),
            inputs.size,
            dmeta.len()
        );
    }
    if algos.is_empty() {
        // size+mtime only
        let dmtime = mtime_ns_of(&dmeta);
        if dmtime != inputs.mtime_ns {
            bail!("verify mtime {}", d.display());
        }
        return Ok(FileRec {
            kind: "file".into(),
            size: inputs.size,
            mtime_ns: inputs.mtime_ns,
            hashes: DigestSet::default(),
        });
    }
    let dh = hash_file(&d, &algos).with_context(|| format!("hash {}", d.display()))?;
    verify_digests(&d, &algos, &inputs.src_digests, &dh)?;
    tracing::Span::current().record("bytes", inputs.size);
    trace!(bytes = inputs.size, "copy_one done");
    Ok(FileRec {
        kind: "file".into(),
        size: inputs.size,
        mtime_ns: inputs.mtime_ns,
        hashes: dh,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::HashAlg;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::prelude::*;

    /// One captured event: its fields, as name/value pairs.
    type CapturedFields = Vec<(String, String)>;

    /// The events a case captured, with their fields.
    ///
    /// A `tracing` event carries its payload as **named fields**, and the log
    /// file a run writes is exactly those fields in JSON. So a parser reading
    /// the log reads the field — `algo` — and not the rendered line, which
    /// means a case that pins the message alone leaves the other channel
    /// unpinned. This collects both.
    ///
    /// The events are keyed by their **message**: that is the field the
    /// console renders and the one the log file carries, so it is what a
    /// reader looks at. The event's own name is the callsite it came from.
    #[derive(Clone, Default)]
    struct CapturedEvents(Arc<Mutex<Vec<CapturedFields>>>);

    impl CapturedEvents {
        /// How many events with this message were captured.
        fn count(&self, message: &str) -> usize {
            self.0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .filter(|fields| fields.iter().any(|(k, v)| k == "message" && v == message))
                .count()
        }

        /// The value one field of the event with this message carried, if it
        /// carried one.
        fn field(&self, message: &str, field: &str) -> Option<String> {
            self.0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .find(|fields| fields.iter().any(|(k, v)| k == "message" && v == message))
                .and_then(|fields| fields.iter().find(|(k, _)| k == field))
                .map(|(_, value)| value.clone())
        }
    }

    /// A visitor that keeps every field as a name/value pair.
    struct FieldVisitor(Vec<(String, String)>);

    impl tracing::field::Visit for FieldVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            // A `Display` field (`%value`) arrives here as its text and a
            // `Debug` field (`?value`) as its quoted form. The quotes belong to
            // the rendering and not to the value, so they are trimmed: what a
            // reader of the log sees is the name, not the name in quotes.
            let text = format!("{value:?}");
            let text = text
                .strip_prefix('"')
                .and_then(|t| t.strip_suffix('"'))
                .unwrap_or(&text);
            self.0.push((field.name().to_string(), text.to_string()));
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.push((field.name().to_string(), value.to_string()));
        }
    }

    impl<S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>>
        tracing_subscriber::Layer<S> for CapturedEvents
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut visitor = FieldVisitor(Vec::new());
            event.record(&mut visitor);
            self.0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(visitor.0);
        }
    }

    /// Run `f` with a subscriber that collects the events it emits.
    ///
    /// The interest cache is rebuilt first: a callsite that another test in
    /// this process already triggered was registered against the subscriber
    /// that was installed then, and a cached "never" would swallow the event
    /// here. Rebuilding re-registers every known callsite against the
    /// subscriber this closure installs.
    fn capture_events<T>(f: impl FnOnce() -> T) -> (T, CapturedEvents) {
        let events = CapturedEvents::default();
        let subscriber = tracing_subscriber::registry().with(events.clone());
        let out = tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            f()
        });
        (out, events)
    }

    /// The algorithms a copy computes and judges, as the set the caller hands
    /// it. There is no list beside it: the set is the whole request, so there
    /// is no second spelling of it that could drift.
    fn aset(names: &[&str]) -> AlgoSet {
        AlgoSet::from_iter(names.iter().map(|s| s.parse::<HashAlg>().unwrap()))
    }

    /// Create a link to a directory: a symlink where the platform allows one,
    /// and a junction on Windows, which needs no elevation.
    ///
    /// `cmd /c mklink /J` creates a directory reparse point that the crate
    /// treats as a link either way. A platform that refuses both is reported
    /// rather than swallowed, so a case can skip itself and say why instead
    /// of passing as if it had covered the shape.
    fn dir_link(target: &Path, link: &Path) -> io::Result<()> {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link)
        }
        #[cfg(windows)]
        {
            let first = match std::os::windows::fs::symlink_dir(target, link) {
                Ok(()) => return Ok(()),
                Err(e) => e,
            };
            // `mklink` is a `cmd` builtin and reads a forward slash as a switch:
            // a target like `dst/sub` would fail with "Invalid switch". std
            // accepts either separator on Windows, so hand it native ones.
            let native = |p: &Path| PathBuf::from(p.to_string_lossy().replace('/', "\\"));
            let created = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(native(link))
                .arg(native(target))
                .output();
            match created {
                Ok(out) if out.status.success() => Ok(()),
                Ok(out) => Err(io::Error::other(format!(
                    "{first}; junction fallback failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ))),
                Err(e) => Err(io::Error::other(format!(
                    "{first}; junction fallback failed: {e}"
                ))),
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (target, link);
            Err(io::Error::other("no link API"))
        }
    }

    /// The canonical form of a root, which is what the guards are handed.
    fn canonical(root: &Path) -> PathBuf {
        dunce::canonicalize(root).unwrap()
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
        let mut buf = Vec::new();
        let sh = copy_and_digest(&mut fin, &mut fout, &set, &mut buf).unwrap();
        drop(fout);

        assert_eq!(std::fs::read(&dst).unwrap(), body, "the bytes landed");
        assert_eq!(
            sh,
            hash_file(&src, &set).unwrap(),
            "the digest taken during the copy is the digest of the file, \
             however the copy loop split it"
        );
    }

    /// `--hash-all-of none` takes the other arm of `copy_and_digest`, and that arm is
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
        let mut buf = Vec::new();
        let sh = copy_and_digest(&mut fin, &mut fout, &AlgoSet::NONE, &mut buf).unwrap();
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
        let mut buf = Vec::new();
        let sh = copy_and_digest(&mut fin, &mut fout, &set, &mut buf).unwrap();
        drop(fout);

        assert_eq!(std::fs::read(&dst).unwrap(), Vec::<u8>::new());
        assert_eq!(sh, hash_file(&src, &set).unwrap());
    }

    /// Reusing a worker's buffer across files must not let an earlier, longer
    /// file leak into the next digest: only the bytes a read actually produced
    /// are written and folded in. A big file runs first so the tail beyond the
    /// small file's last read still holds its bytes, which is exactly the state
    /// a reused buffer is handed in with.
    #[test]
    fn a_reused_buffer_digests_only_the_bytes_that_were_read() {
        let dir = girpr_tempdir::TempDir::new("girsync_copy", "reusedbuf");
        let set = aset(&["md5", "sha256"]);
        let big_body: Vec<u8> = (0..(CHUNK + 4096)).map(|i| (i % 251) as u8).collect();
        let big_src = dir.join("big.bin");
        let big_dst = dir.join("big.copy");
        std::fs::write(&big_src, &big_body).unwrap();

        let mut buf = Vec::new();
        let mut fin = std::fs::File::open(&big_src).unwrap();
        let mut fout = std::fs::File::create(&big_dst).unwrap();
        copy_and_digest(&mut fin, &mut fout, &set, &mut buf).unwrap();
        drop(fout);

        let small_body = b"small".to_vec();
        let small_src = dir.join("small.bin");
        let small_dst = dir.join("small.copy");
        std::fs::write(&small_src, &small_body).unwrap();
        let mut fin = std::fs::File::open(&small_src).unwrap();
        let mut fout = std::fs::File::create(&small_dst).unwrap();
        let sh = copy_and_digest(&mut fin, &mut fout, &set, &mut buf).unwrap();
        drop(fout);

        assert_eq!(
            std::fs::read(&small_dst).unwrap(),
            small_body,
            "the small file's bytes landed, and only those"
        );
        assert_eq!(
            sh,
            hash_file(&small_src, &set).unwrap(),
            "the digest covers the read bytes, not whatever the buffer held before"
        );
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

        let err = verify_digests(path, &aset(&["md5", "sha256"]), &src, &dst)
            .expect_err("two digests that disagree are not a verified copy");
        let msg = err.to_string();
        assert!(
            msg.contains("verify hash(md5)"),
            "names the algorithm: {msg}"
        );
        assert!(msg.contains("dst/file.bin"), "names the path: {msg}");
    }

    /// **The algorithm a failed copy names is the canonical one, not the
    /// request's.**
    ///
    /// The case above lets one digest disagree, so it says nothing about *which*
    /// algorithm a failure names when two disagree: with only one disagreement
    /// there is only one name to give. This one lets both disagree, and pins the
    /// answer to the order the crate uses everywhere else — `md5` before
    /// `sha256`, the order [`AlgoSet::iter`] yields and the order the diff's
    /// `digest-differs` tag names them in.
    ///
    /// The request is spelled the **other** way round, on purpose. The set the
    /// verify judges carries no order of its own, so the order the user typed is
    /// not available to it; a set that kept the order it was built in — or a
    /// verify that read the request instead of the set — would name `sha256` here
    /// and fail this case.
    #[test]
    fn a_failed_copy_names_the_algorithm_in_canonical_order() {
        let path = Path::new("dst/file.bin");
        let algos = aset(&["sha256", "md5"]);
        // Both digests differ, each in the slot its own algorithm owns.
        let mut src = DigestSet::default();
        src.insert_named("md5", &[1u8; 16]);
        src.insert_named("sha256", &[1u8; 32]);
        let mut dst = DigestSet::default();
        dst.insert_named("md5", &[2u8; 16]);
        dst.insert_named("sha256", &[2u8; 32]);

        let err = verify_digests(path, &algos, &src, &dst)
            .expect_err("two digests that disagree are not a verified copy");
        let msg = err.to_string();
        assert!(
            msg.contains("verify hash(md5)"),
            "names the first algorithm in canonical order: {msg}"
        );
        assert!(
            !msg.contains("sha256"),
            "and not the one the request happened to spell first: {msg}"
        );
    }

    /// **The algorithm a failed verify names is in the log event too.**
    ///
    /// The bail message is one channel and the error event is the other, and a
    /// parser reading the log file reads the **fields** — not the rendered
    /// line. So the message and the event have to agree, and a change that
    /// kept the message while dropping or renaming the field would leave the
    /// message's case passing with the other channel broken.
    ///
    /// The event's **message** says what happened (`verify hash mismatch`) and
    /// it carries the algorithm as a field beside it. The algorithm is the
    /// **canonical** one, the same the message names: the set carries no order
    /// of its own, so the first disagreement in canonical order is the one
    /// reported.
    #[test]
    fn a_failed_verify_names_the_algorithm_in_the_error_event() {
        let path = Path::new("dst/file.bin");
        let algos = aset(&["sha256", "md5"]);
        let mut src = DigestSet::default();
        src.insert_named("md5", &[1u8; 16]);
        src.insert_named("sha256", &[1u8; 32]);
        let mut dst = DigestSet::default();
        dst.insert_named("md5", &[2u8; 16]);
        dst.insert_named("sha256", &[2u8; 32]);

        let (result, events) = capture_events(|| verify_digests(path, &algos, &src, &dst));

        assert!(
            result.is_err(),
            "two digests that disagree are not a verified copy"
        );
        assert_eq!(
            events.count("verify hash mismatch"),
            1,
            "the failure is logged once"
        );
        assert_eq!(
            events.field("verify hash mismatch", "algo").as_deref(),
            Some("md5"),
            "the event names the same algorithm the message names: the canonical \
             one, not the request's"
        );
        assert_eq!(
            events.field("verify hash mismatch", "path").as_deref(),
            Some("dst/file.bin"),
            "and the event names the path it is about"
        );
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
                &aset(&["md5", "sha256"]),
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

        let err = verify_digests(Path::new("dst/file.bin"), &aset(&["md5"]), &src, &dst)
            .expect_err("a digest dst never produced is not one it agrees with");
        assert!(
            err.to_string().contains("verify hash(md5)"),
            "the missing digest is the one reported: {err}"
        );
    }

    /// Only what the run asked for is evidence about this copy. A digest nobody
    /// requested is not a disagreement, and treating it as one would fail every
    /// copy whose maps happen to carry more names than the set.
    ///
    /// This pins which side the loop walks: the set, not the maps. A comparison
    /// iterating the digests instead would fail here, having found a real
    /// difference in a value no caller asked about.
    #[test]
    fn only_the_requested_algorithms_are_judged() {
        let src = digests(&["md5", "sha256"], 1);
        let mut dst = src.clone();
        dst.insert_named("sha256", &[2u8; 16]);

        assert!(
            verify_digests(Path::new("dst/file.bin"), &aset(&["md5"]), &src, &dst).is_ok(),
            "sha256 was not requested, so its disagreement is not this copy's problem"
        );
    }

    /// A copy computes the algorithms it was given and judges exactly those, so
    /// the digests it produced and the digests it compares are the same set by
    /// construction.
    ///
    /// The record the copy hands back carries the digests it computed, so the
    /// set is visible in what the cache is given: a digest outside it was
    /// neither computed nor judged.
    #[test]
    fn a_copy_computes_exactly_the_algorithms_it_was_given() {
        let dir = girpr_tempdir::TempDir::new("girsync_copy", "algoset");
        let src = dir.subdir("src");
        let dst = dir.subdir("dst");
        std::fs::write(src.join("a.txt"), b"content").unwrap();
        let src_canonical = canonical(&src);
        let mut buf = Vec::new();

        let rec = copy_one(
            &src,
            &src_canonical,
            &dst,
            "a.txt",
            aset(&["md5"]),
            &mut buf,
            &TestHooks::default(),
        )
        .expect("the copy succeeds");
        assert_eq!(
            std::fs::read(dst.join("a.txt")).unwrap(),
            b"content",
            "the bytes landed"
        );
        assert!(
            rec.hashes.get_algo(HashAlg::Md5).is_some(),
            "md5 was asked for, so it was computed"
        );
        assert!(
            rec.hashes.get_algo(HashAlg::Sha256).is_none(),
            "sha256 was not asked for, so it was neither computed nor judged"
        );
    }

    /// A link at the destination that resolves **into** src is refused, and
    /// refused with the text an operator matches on.
    ///
    /// The shape is the dangerous one: `File::create` follows such a link, so a
    /// copy through it would truncate a source file while the verify read the
    /// same file back through the link and agreed with itself. The refusal
    /// names the overlap, so the text is asserted rather than left to
    /// inspection.
    #[test]
    fn a_final_component_link_into_src_is_refused() {
        let dir = girpr_tempdir::TempDir::new("girsync_copy", "link_into_src");
        let src = dir.subdir("src");
        let dst = dir.subdir("dst");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub/keep.txt"), b"source").unwrap();
        let link = dst.join("a.txt");
        if let Err(e) = dir_link(&src.join("sub"), &link) {
            eprintln!("skip: this platform refused a directory link: {e}");
            return;
        }
        let src_canonical = canonical(&src);

        let err = refuse_final_link_into_src(&link, "a.txt", &src_canonical)
            .expect_err("a link that resolves into src must be refused");
        let resolved = canonical(&src.join("sub"));
        assert_eq!(
            err.to_string(),
            format!(
                "src and dst must be disjoint: dst link `a.txt` resolves to `{}`, \
                 which is inside src `{}`; refusing to copy through it (remove the \
                 link, or exclude it with --exclude)",
                resolved.display(),
                src_canonical.display()
            ),
            "the refusal names the overlap, and its text is what an operator matches on"
        );
        assert_eq!(
            std::fs::read(src.join("sub/keep.txt")).unwrap(),
            b"source",
            "the refusal came before anything was written"
        );
    }

    /// A link that resolves **outside** src is not refused. It is the
    /// documented write-through, and the guard is not "refuse every link": a
    /// guard that refused these would refuse ordinary mirrored content.
    #[test]
    fn a_link_outside_src_is_not_refused() {
        let dir = girpr_tempdir::TempDir::new("girsync_copy", "link_outside");
        let src = dir.subdir("src");
        let dst = dir.subdir("dst");
        let other = dir.subdir("other");
        std::fs::write(other.join("keep.txt"), b"theirs").unwrap();
        let link = dst.join("a.txt");
        if let Err(e) = dir_link(&other, &link) {
            eprintln!("skip: this platform refused a directory link: {e}");
            return;
        }
        let src_canonical = canonical(&src);

        refuse_final_link_into_src(&link, "a.txt", &src_canonical)
            .expect("a link to anywhere else is ordinary content");
    }

    /// A directory in the way of a file copy is removed, and what it held goes
    /// with it: the copy's destination is a file, so the directory is the type
    /// conflict this step resolves.
    #[test]
    fn a_directory_at_the_destination_is_removed() {
        let dir = girpr_tempdir::TempDir::new("girsync_copy", "blocking_dir");
        let dst = dir.subdir("dst");
        let d = dst.join("a.txt");
        std::fs::create_dir_all(d.join("inside")).unwrap();
        std::fs::write(d.join("inside/keep.txt"), b"x").unwrap();

        clear_blocking_dir(&d).expect("a directory in the way is removed");
        assert!(!d.exists(), "the directory is gone");
        assert!(
            !d.join("inside/keep.txt").exists(),
            "and the tree under it went with it"
        );
    }

    /// A link to a directory is **unlinked**, not recursed into. The target is
    /// somebody else's tree: unlinking the link is what resolves the type
    /// conflict toward src's kind, and the target is left alone.
    #[test]
    fn a_link_to_a_directory_is_unlinked_rather_than_recursed() {
        let dir = girpr_tempdir::TempDir::new("girsync_copy", "dir_link");
        let dst = dir.subdir("dst");
        let target = dir.subdir("target");
        std::fs::write(target.join("keep.txt"), b"theirs").unwrap();
        let d = dst.join("a.txt");
        if let Err(e) = dir_link(&target, &d) {
            eprintln!("skip: this platform refused a directory link: {e}");
            return;
        }

        clear_blocking_dir(&d).expect("the link in the way is unlinked");
        assert!(
            std::fs::symlink_metadata(&d).is_err(),
            "the link itself is gone, not what it pointed at"
        );
        assert_eq!(
            std::fs::read(target.join("keep.txt")).unwrap(),
            b"theirs",
            "the target's own tree is left alone"
        );
    }

    /// A destination with more than one name is refused, and refused before
    /// anything truncates the shared inode.
    ///
    /// The count is the whole check, so the refusal is the conservative one:
    /// any count above one, whatever the second name is and wherever it lives.
    #[test]
    fn a_hard_linked_destination_is_refused_before_it_is_truncated() {
        let dir = girpr_tempdir::TempDir::new("girsync_copy", "hardlink");
        let src = dir.subdir("src");
        let dst = dir.subdir("dst");
        std::fs::write(src.join("a.txt"), b"NEW").unwrap();
        let d = dst.join("a.txt");
        std::fs::write(&d, b"ORIGINAL").unwrap();
        std::fs::hard_link(&d, dst.join("shared.txt")).unwrap();
        let src_canonical = canonical(&src);
        let mut buf = Vec::new();

        let err = copy_one(
            &src,
            &src_canonical,
            &dst,
            "a.txt",
            aset(&["md5"]),
            &mut buf,
            &TestHooks::default(),
        )
        .expect_err("a destination with two names must be refused");
        let msg = err.to_string();
        assert!(msg.contains("hard links"), "names the hazard: {msg}");
        assert!(msg.contains("a.txt"), "names the copy: {msg}");
        assert_eq!(
            std::fs::read(&d).unwrap(),
            b"ORIGINAL",
            "the refusal came before any truncation, so the shared inode is intact"
        );
        assert_eq!(
            std::fs::read(dst.join("shared.txt")).unwrap(),
            b"ORIGINAL",
            "and so is the other name for it"
        );
    }

    /// The count of one is the ordinary case, and a destination that is not
    /// there has nothing to truncate. Both have to be let through, or every
    /// copy would be refused.
    #[test]
    fn a_destination_with_one_name_or_none_at_all_is_allowed() {
        let dir = girpr_tempdir::TempDir::new("girsync_copy", "one_link");
        let dst = dir.subdir("dst");
        let single = dst.join("a.txt");
        std::fs::write(&single, b"x").unwrap();

        refuse_hard_linked_dst(&single, "a.txt").expect("one name is not a shared inode");
        refuse_hard_linked_dst(&dst.join("gone.txt"), "gone.txt")
            .expect("a missing destination has nothing to truncate");
    }

    /// The order of the guards is load-bearing, and this is where it shows: a
    /// destination that is a link into src is refused **before** the type
    /// cleanup would have unlinked it.
    ///
    /// The cleanup and the check are about the same path, and the cleanup
    /// resolves the conflict in the copy's favour by removing the link. The
    /// refusal comes first, so the link is still there after it — and the copy
    /// stops, rather than carrying on with the source it was about to reach.
    #[test]
    fn the_link_into_src_is_refused_before_the_type_cleanup_runs() {
        let dir = girpr_tempdir::TempDir::new("girsync_copy", "order");
        let src = dir.subdir("src");
        let dst = dir.subdir("dst");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub/keep.txt"), b"source").unwrap();
        let d = dst.join("a.txt");
        if let Err(e) = dir_link(&src.join("sub"), &d) {
            eprintln!("skip: this platform refused a directory link: {e}");
            return;
        }
        let src_canonical = canonical(&src);
        let mut buf = Vec::new();

        let err = copy_one(
            &src,
            &src_canonical,
            &dst,
            "a.txt",
            aset(&["md5"]),
            &mut buf,
            &TestHooks::default(),
        )
        .expect_err("the copy must refuse a link into src");
        assert!(
            err.to_string().contains("which is inside src"),
            "names the overlap: {err}"
        );
        assert!(
            crate::link::try_is_link(&d).unwrap(),
            "the type cleanup did not run before the refusal"
        );
        assert_eq!(
            std::fs::read(src.join("sub/keep.txt")).unwrap(),
            b"source",
            "the source is untouched"
        );
    }
}
