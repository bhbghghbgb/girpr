//! Executing a sync plan against the filesystem.
//!
//! All shared state is hung off [`Applier`] so each phase reads as a short list
//! of steps. The phase *order* is a correctness invariant, so it lives in one
//! place ([`Applier::apply`]) rather than being spread across the coordinator:
//!
//! 1. mkdir + fix-dirs (create the shapes src expects),
//! 2. pre-drop the cache entries for every path about to change,
//! 3. delete extras, copy src files, remove unknown dirs,
//! 4. prune leftover cache rows.
//!
//! Steps 1 and 2 must precede any content change: cache entries are always
//! dropped *and committed* before the corresponding filesystem change, so a
//! crash mid-run re-copies rather than trusting a half-written file. The
//! commit is the durability point — there is no separate flush, and there is
//! no resume beyond re-running.
//!
//! Every action a phase takes is announced through [`Applier::report`], and only
//! there: `Report::emit` prints the record and logs the matching event in one
//! place, so a phase cannot report an action the log never sees, nor narrate an
//! action twice by hand. `--dry-run` reports the same records for the same plan,
//! which is why the plan printer and these phases agree field for field.

use anyhow::{Context, Result, bail};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::Path;
use tracing::{debug, error, info, trace};

use super::plan::Plan;
use crate::cache::{CacheDb, FileRec};
use crate::config::CommonOpts;
use crate::effective::EffRec;
use crate::hash::{AlgoSet, CHUNK, Digests, HashAlg, hash_file};
use crate::report::{Record, Report};
use crate::rw::RwRuntime;
use crate::util::mtime_ns_of;

/// Everything the apply phases share: both roots, the dst cache, the two
/// resolved side maps, and the resolved config.
pub(super) struct Applier<'a> {
    pub src: &'a Path,
    pub dst: &'a Path,
    pub dst_db: &'a CacheDb,
    /// dst effective map, after the rename pass. Consulted for what dst actually
    /// holds at a path; src's map is not consulted, because the planner already
    /// decided which directories to remove (`Plan::rmdir`).
    pub dm: &'a HashMap<String, EffRec>,
    pub common: &'a CommonOpts,
    /// The run's rw limits and the pool they size.
    ///
    /// A reference rather than a count, because a count is only half of what the
    /// copy phase needs: the pool sizes the workers and the limiter caps how many
    /// may be *doing I/O*, which under a split limit is not the same number as the
    /// worker count. `&'a RwRuntime` rather than an owned one so a phase cannot
    /// quietly build a second pool from the same flags.
    pub rw: &'a RwRuntime,
    /// Where the plan's records go, and in what format.
    ///
    /// The apply phases announce every action they take, and `--dry-run` reports
    /// what it *would* take; both go through this one writer, so the two are the
    /// same document rather than two printers that must agree.
    pub report: &'a Report,
}

/// What a run actually did, for the `SUMMARY` record.
///
/// Field order matches the phase order in [`Applier::apply`].
#[derive(Debug, Default)]
pub(super) struct Applied {
    pub deleted: usize,
    pub copied: usize,
    pub removed_dirs: usize,
}

impl Applier<'_> {
    /// Run every apply phase in the order documented on this module.
    pub(super) fn apply(&self, plan: &Plan) -> Result<Applied> {
        self.mkdirs(plan)?;
        self.fix_dirs(plan)?;
        self.predrop_cache_entries(plan)?;
        // Field order is the phase order: deletes, then copies, then rmdirs.
        let applied = Applied {
            deleted: self.delete_extras(plan)?,
            copied: self.copy_files(plan)?,
            removed_dirs: self.remove_unknown_dirs(plan)?,
        };
        self.prune_dst_cache(plan)?;
        Ok(applied)
    }

    /// Create every src directory that dst lacks.
    ///
    /// Each one is reported, like every other action in the plan. The apply
    /// phases record what they do and `--dry-run` records what it would do, and
    /// the two documents are supposed to be the same — a phase that works
    /// silently is the one place a dry run cannot mirror, and a directory the
    /// user did not know was being created is worth a record.
    fn mkdirs(&self, plan: &Plan) -> Result<()> {
        info!(dirs = plan.mkdir.len(), "apply mkdirs");
        let mut w = self.dst_db.begin_write()?;
        for (i, r) in plan.mkdir.iter().enumerate() {
            std::fs::create_dir_all(self.dst.join(r))
                .with_context(|| format!("mkdir {}", self.dst.join(r).display()))?;
            w.put(r, &FileRec::dir())?;
            self.report.emit(Record::path("MKDIR", r));
            if (i + 1) % 100 == 0 {
                info!(done = i + 1, total = plan.mkdir.len(), "mkdir progress");
            }
        }
        w.commit()?;
        Ok(())
    }

    /// Resolve type-conflicts toward src: dst has a file where src has a dir.
    ///
    /// Cache rows are dropped and committed *before* any filesystem change,
    /// so a crash here re-resolves rather than trusting the half-done state.
    fn fix_dirs(&self, plan: &Plan) -> Result<()> {
        info!(count = plan.fix_dirs.len(), "apply fix-dirs");
        if plan.fix_dirs.is_empty() {
            return Ok(());
        }
        {
            let mut w = self.dst_db.begin_write()?;
            for r in &plan.fix_dirs {
                w.remove(r)?; // delete entries before change
            }
            w.commit()?;
        }
        for r in &plan.fix_dirs {
            let p = self.dst.join(r);
            if p.is_file() || p.is_symlink() {
                std::fs::remove_file(&p).with_context(|| format!("remove {}", p.display()))?;
            } else if p.is_dir() {
                std::fs::remove_dir_all(&p).with_context(|| format!("rmdir {}", p.display()))?;
            }
            std::fs::create_dir_all(&p).with_context(|| format!("mkdir {}", p.display()))?;
            self.report.emit(Record::path("FIX-DIR", r));
        }
        Ok(())
    }

    /// Drop the dst cache rows for every path this run will change, and
    /// commit before touching the filesystem.
    ///
    /// This is belt-and-braces, and deliberately so: the pre-drop is *not* the
    /// mechanism that makes an interrupted run safe. Every way `copy_one` can
    /// die is already caught without it, because
    ///
    /// - `File::create` truncates, so a partial write is strictly smaller than
    ///   src and fails the size comparison;
    /// - mtime is restored only after a complete write, so "copied but not yet
    ///   stamped" fails the mtime comparison;
    /// - a verify failure leaves the file complete with matching stat, but its
    ///   content then differs from the digest already on the row, so the
    ///   comparison still fires;
    /// - a run that got all the way through has nothing left to do.
    ///
    /// It stays because it costs one batched write and it does not depend on
    /// any of those properties holding. It becomes load-bearing the moment
    /// `copy_one` stops truncating, stops deferring mtime, or reuses a digest
    /// across a content change — so do not "optimize it away" without checking
    /// those three. `copy_one` digesting src during the copy is the near-miss
    /// worth knowing about: src's digest comes from the copy's own read, but
    /// dst's is still a separate read of the file on disk, which is the third
    /// property speaking. A single-pass copy — digesting dst from the same chunks
    /// going out — would trip it, and at that point "the comparison" is a value
    /// against itself and the pre-drop is the only thing left standing. Note also that
    /// it is not a *signal*: a later scan that
    /// records stat without a digest for a size/mtime-differing file looks
    /// exactly like this, so nothing downstream may depend on the row's absence
    /// to detect a crash.
    fn predrop_cache_entries(&self, plan: &Plan) -> Result<()> {
        let mut w = self.dst_db.begin_write()?;
        // Two loops rather than one chained iterator: `copy` holds `CopyItem`s and
        // `delete_files` holds paths, so the only thing they have in common is the
        // path, and a chain would have to borrow it from two different types.
        for item in &plan.copy {
            w.remove(&item.rel)?;
        }
        for r in &plan.delete_files {
            w.remove(r)?;
        }
        w.commit()?;
        Ok(())
    }

    /// Delete dst-only files, returning how many paths were removed.
    fn delete_extras(&self, plan: &Plan) -> Result<usize> {
        let mut deleted = 0usize;
        info!(files = plan.delete_files.len(), "apply deletes");
        for (i, r) in plan.delete_files.iter().enumerate() {
            let p = self.dst.join(r);
            // the plan only holds files here; dirs are handled by the
            // unknown-dirs pass
            if let Some(rec) = self.dm.get(r)
                && rec.kind == "dir"
            {
                continue;
            }
            if p.is_file() || p.is_symlink() {
                std::fs::remove_file(&p).with_context(|| format!("delete {}", p.display()))?;
                self.report.emit(Record::path("DELETE", r));
                deleted += 1;
            } else if p.is_dir() {
                std::fs::remove_dir_all(&p).with_context(|| format!("rmdir {}", p.display()))?;
                self.report.emit(Record::path("RMDIR", r));
                deleted += 1;
            } else if p.exists() {
                bail!("unsupported type {}", p.display());
            }
            if (i + 1) % 100 == 0 {
                info!(
                    done = i + 1,
                    total = plan.delete_files.len(),
                    deleted,
                    "delete progress"
                );
            }
        }
        info!(deleted, "deletes done");
        Ok(deleted)
    }

    /// Copy every planned src file across, in parallel, returning the count.
    ///
    /// The pool is the run's, not one built here: two pools sized from the same
    /// flags would mean twice the threads the user asked for, and a limit that
    /// bounds I/O while the pool decides how many workers exist is two knobs
    /// pretending to be one.
    ///
    /// Each copy holds **both** permits for its whole life — the stream copy and
    /// the verify re-hash of dst — because the file is not finished until it
    /// verifies. Under a shared counter that is one permit; under a split counter
    /// it is one per side, which is what admits `min(src, dst)` concurrent
    /// copies.
    fn copy_files(&self, plan: &Plan) -> Result<usize> {
        info!(
            files = plan.copy.len(),
            rw = %self.rw.describe(),
            threads = self.rw.pool().current_num_threads(),
            "copy start"
        );
        let copy_total = plan.copy.len();
        let copy_done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let copy_done_cb = copy_done.clone();
        // Serialises the *emit*, not the copy.
        //
        // `Report::emit` prints a line and logs a matching event as one unit, and
        // without this two workers could interleave between the two — putting a log
        // event about one file between the line and the event for another. The lock
        // is held for exactly one emit, never across the copy, so it does not
        // serialise the work it is protecting.
        //
        // **It cannot deadlock.** It is a leaf: nothing is called while it is held,
        // no other lock is ever held while waiting for it, and it is released before
        // the worker returns to the pool. It is also strictly *inside* the rw
        // permit, which is held across the whole copy — so a copy waiting for the
        // emit lock is never also holding a permit some other copy needs, and the
        // permit wait-for graph stays the `src -> dst` DAG it has to be.
        let emit = std::sync::Mutex::new(());
        let results = self.rw.pool().install(|| {
            use rayon::prelude::*;
            plan.copy
                .par_iter()
                .map(|item| {
                    let _rw = self.rw.acquire_copy();
                    debug!(rel = %item.rel, "copy start");
                    let r = copy_one(self.src, self.dst, &item.rel, &self.common.algos);
                    let n = copy_done_cb.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    if n.is_multiple_of(25) || n == copy_total {
                        info!(done = n, total = copy_total, "copy progress");
                    } else {
                        trace!(done = n, total = copy_total, rel = %item.rel, "copy progress");
                    }
                    // **As done.** Announced the moment the file is in place, not
                    // after every copy finishes — so with more than one rw operation
                    // permitted the `COPY` lines come out in completion order rather
                    // than plan order.
                    //
                    // Only on success: a failed copy aborts the run before the
                    // summary, so announcing one would be a claim about work that
                    // did not happen.
                    if r.is_ok() {
                        let _guard = emit.lock().expect("emit lock is never poisoned");
                        self.report.emit(
                            Record::path("COPY", &item.rel).put("because", item.because.as_str()),
                        );
                    }
                    r
                })
                .collect::<Vec<_>>()
        });
        let mut copied = 0usize;
        let mut new_recs: Vec<(String, FileRec)> = Vec::new();
        // Records only — the `COPY` line was already emitted above, as each copy
        // completed. The results come back in plan order because `collect` preserves
        // input order, so the cache rows below are written deterministically even
        // though the lines are not.
        for (item, r) in plan.copy.iter().zip(results) {
            let rec = r.with_context(|| format!("copy {}", item.rel))?;
            new_recs.push((item.rel.clone(), rec));
            copied += 1;
        }
        info!(copied, total = copy_total, "copy done");
        // Only record files that actually verified, so a partial failure never
        // leaves a cache claiming content the dst does not have.
        let mut w = self.dst_db.begin_write()?;
        for (rel, rec) in &new_recs {
            w.put(rel, rec)?;
        }
        w.commit()?;
        Ok(copied)
    }

    /// Remove dst directories that src does not have, deepest first.
    ///
    /// Runs after the copies so directories emptied by them are collected too,
    /// and reads the list [`Plan::rmdir`] rather than deriving one here. That is
    /// deliberate: the dry run must be able to state the same number before
    /// anything is written, so the decision belongs to the planner and this pass
    /// is left with nothing to decide. It also drops a full walk of dst, and
    /// makes the set the one the `--dry-run` summary already promised.
    ///
    /// The list excludes any directory a planned copy was going to clear out of
    /// the way, so nothing here can name a path that is already gone; the
    /// `is_dir` check stays as a cheap guard rather than as the logic.
    fn remove_unknown_dirs(&self, plan: &Plan) -> Result<usize> {
        let mut removed_dirs = 0usize;
        info!(unknown = plan.rmdir.len(), "rmdir pass");
        let mut w = self.dst_db.begin_write()?;
        for (i, r) in plan.rmdir.iter().enumerate() {
            let p = self.dst.join(r);
            if p.is_dir() {
                if let Err(e) = std::fs::remove_dir_all(&p)
                    && p.exists()
                {
                    bail!("rmdir {}: {:#}", p.display(), e);
                }
                w.remove(r)?;
                self.report.emit(Record::path("RMDIR", r));
                removed_dirs += 1;
            }
            if (i + 1) % 100 == 0 {
                info!(
                    done = i + 1,
                    total = plan.rmdir.len(),
                    removed = removed_dirs,
                    "rmdir progress"
                );
            }
        }
        w.commit()?;
        Ok(removed_dirs)
    }

    /// Drop dst cache rows for paths that no longer exist on disk.
    ///
    /// The present set is derived rather than walked. `dm` is the post-rename dst
    /// map — the same filtered, depth-capped set of paths a live walk would report,
    /// which is why `Plan::rmdir` reads it instead of deriving a second one — so the
    /// only thing it cannot know is what *this run* has done to the disk since.
    ///
    /// Both directions matter, and getting only one right is the failure mode:
    /// an additive-only set keeps rows for a subtree a copy wiped, and a
    /// subtractive-only set drops rows for a path the run created.
    ///
    /// **Added** — what the apply phases create. `mkdir` for new directories, `copy`
    /// for copied files, `fix_dirs` for the directories that replaced a file. All
    /// three put a cache row as they go, so all three count as present; leaving one
    /// out deletes a row for a path that is on disk, and the damage does not show up
    /// here — it surfaces later as a `compare-self` reporting drift for a file that
    /// never moved.
    ///
    /// **Subtracted** — what a copy destroys. `delete_files` and `rmdir` drop their
    /// own rows, so those need nothing. The one that does is `copy_one` clearing a
    /// blocking dst directory *recursively*, which takes a whole subtree with it.
    /// `Plan::rmdir` deliberately excludes that subtree from its own list — the
    /// directory is gone by the time the removal pass runs, so listing it would
    /// promise an `rmdir` the run cannot reach — which leaves this the only place
    /// that can tell "dst held this before the run" from "dst holds it now".
    ///
    /// The subtraction is by path prefix, so it covers the subtree at any depth
    /// rather than only the entries the plan happened to enumerate.
    ///
    /// **One case this cannot reach**, deliberately: `--case-sensitive` on a
    /// case-insensitive filesystem, where a directory this run `mkdir`s can be the
    /// same directory a later `rmdir` removes. Deriving cannot see that without
    /// case-folding the prune against the plan, in the one mode that deliberately
    /// does not case-fold. Such rows survive until the next run's phase A prunes
    /// them, and `compare-self` builds a cold cache so it cannot be misled.
    ///
    /// Cost: one pass over the map and three in-memory lists, against a full
    /// traversal of dst with a `stat` per entry. On a large tree that traversal is
    /// a real fraction of the run, and it would be the only one left in a real
    /// `sync`.
    fn prune_dst_cache(&self, plan: &Plan) -> Result<()> {
        // dst paths this run's copies destroy, as `dir/` prefixes. Computed from
        // `dm` rather than the plan's own lists, so it names exactly the subtrees
        // `copy_one` will clear: a copy whose dst is a directory.
        let wiped: Vec<String> = plan
            .copy
            .iter()
            .filter(|item| self.dm.get(&item.rel).is_some_and(|rec| rec.kind == "dir"))
            .map(|item| format!("{}/", item.rel))
            .collect();
        let mut present: HashSet<&str> = self.dm.keys().map(String::as_str).collect();
        for r in &plan.mkdir {
            present.insert(r);
        }
        for r in &plan.fix_dirs {
            present.insert(r);
        }
        for item in &plan.copy {
            present.insert(&item.rel);
        }
        let mut w = self.dst_db.begin_write()?;
        for rel in w.load_keys()? {
            // Strictly longer than the prefix, so the copied path itself keeps its
            // row: only what is *under* a wiped directory is gone.
            let wiped_subtree = wiped
                .iter()
                .any(|p| rel.len() > p.len() && rel.starts_with(p.as_str()));
            if !present.contains(rel.as_str()) || wiped_subtree {
                w.remove(&rel)?;
            }
        }
        w.commit()?;
        Ok(())
    }
}

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
) -> Result<HashMap<String, Vec<u8>>> {
    if algos.is_empty() {
        std::io::copy(src, dst)?;
        return Ok(HashMap::new());
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
fn verify_digests(
    path: &Path,
    algos: &[HashAlg],
    src: &HashMap<String, Vec<u8>>,
    dst: &HashMap<String, Vec<u8>>,
) -> Result<()> {
    for a in algos {
        if src.get(a.as_str()) != dst.get(a.as_str()) {
            error!(algo = %a, path = %path.display(), "verify hash mismatch");
            bail!("verify hash({}) {}", a, path.display());
        }
    }
    Ok(())
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
/// `sync_all()`, it also hashes the bytes that actually reached the disk, which
/// is the only way this catches silent corruption on the way down.
///
/// **What the src-side digest is not.** It describes the bytes src held *during*
/// the copy, not after it. So a src file rewritten mid-copy is not caught here:
/// dst is a faithful copy of a src that no longer exists, and the pair looks
/// clean. That is a race the tool cannot win anyway — phase A's stat sees the new
/// mtime on the next run and re-copies.
#[tracing::instrument(skip_all, fields(rel, bytes = tracing::field::Empty))]
fn copy_one(src_root: &Path, dst_root: &Path, rel: &str, algos: &[HashAlg]) -> Result<FileRec> {
    trace!("copy_one start");
    let s = src_root.join(rel);
    let d = dst_root.join(rel);
    // dst dir that is in the way of a file copy: remove first
    if d.is_dir() && !d.is_symlink() {
        std::fs::remove_dir_all(&d).with_context(|| format!("rmdir {}", d.display()))?;
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
    fout.sync_all().ok();
    drop(fout);
    // preserve mtime
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
            hashes: HashMap::new(),
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
        let dir = crate::testtemp::TempDir::new("girsync_copy", "digest");
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
        let dir = crate::testtemp::TempDir::new("girsync_copy", "noalgo");
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
        let dir = crate::testtemp::TempDir::new("girsync_copy", "empty");
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
    fn digests(algos: &[&str], byte: u8) -> HashMap<String, Vec<u8>> {
        algos.iter().map(|a| (a.to_string(), vec![byte])).collect()
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
        dst.insert("md5".into(), vec![2]);

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
        let dst = HashMap::new();

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
        dst.insert("sha256".into(), vec![2]);

        assert!(
            verify_digests(Path::new("dst/file.bin"), &algos(&["md5"]), &src, &dst).is_ok(),
            "sha256 was not requested, so its disagreement is not this copy's problem"
        );
    }
}
