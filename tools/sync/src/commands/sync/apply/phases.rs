//! The four filesystem phases of `Applier`: delete-extras, copy-files,
//! remove-unknown-dirs, and prune-dst-cache. `copy_files` delegates the
//! per-file work to [`super::copy::copy_one`].

use anyhow::{Context, Result, bail};
use std::collections::HashSet;
use tracing::{debug, info, trace};

use super::super::plan::Plan;
use super::Applier;
use super::copy::copy_one;
use crate::report::Record;

impl<'a> Applier<'a> {
    /// Delete dst-only files, returning how many paths were removed.
    ///
    /// **Never reaches through a link.** `Plan::delete_files` names the topmost
    /// link rather than a path under it (see `build_plan`), and this pass
    /// re-checks disk anyway: a planned path whose ancestor is now a link is
    /// replaced by that link. That is the difference between removing the link
    /// and removing a file in its target - possibly outside `dst`.
    pub(crate) fn delete_extras(&self, plan: &Plan) -> Result<usize> {
        let mut deleted = 0usize;
        info!(files = plan.delete_files.len(), "apply deletes");
        for (i, r) in plan.delete_files.iter().enumerate() {
            // Belt and braces: the plan was computed before this run changed
            // anything, so ask the filesystem once more. `None` means no
            // component is a link (including when the path is already gone).
            let rel = crate::link::topmost_link_ancestor(self.dst, r)
                .with_context(|| format!("check link ancestors for {r}"))?
                .unwrap_or_else(|| r.clone());
            if rel != *r {
                tracing::warn!(
                    planned = %r,
                    removing = %rel,
                    "planned delete is under a link; removing the link instead"
                );
            }
            let p = self.dst.join(&rel);
            // the plan only holds files and links here; a genuine dir is handled
            // by the unknown-dirs pass
            if let Some(rec) = self.dm.get(&rel)
                && rec.kind == "dir"
                && !rec.is_link
            {
                continue;
            }
            if crate::link::try_is_link(&p)
                .with_context(|| format!("check link {}", p.display()))?
            {
                remove_link(&p)?;
                self.report.emit(Record::path("DELETE", &rel));
                deleted += 1;
            } else {
                match std::fs::symlink_metadata(&p) {
                    Ok(md) if md.is_file() => {
                        remove_file(&p)?;
                        self.report.emit(Record::path("DELETE", &rel));
                        deleted += 1;
                    }
                    Ok(md) if md.is_dir() => {
                        remove_dir_all(&p)?;
                        self.report.emit(Record::path("RMDIR", &rel));
                        deleted += 1;
                    }
                    Ok(_) => bail!("unsupported type {}", p.display()),
                    // Gone between the plan and now: nothing to remove, and the
                    // cache row was already pre-dropped.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e).with_context(|| format!("stat {}", p.display())),
                }
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
    /// Each copy holds **both** permits for the whole of `copy_one` — the stream
    /// copy and the verify re-hash of dst — because the file is not finished until
    /// it verifies. Under a shared counter that is one permit; under a split counter
    /// it is one per side, which is what admits `min(src, dst)` concurrent
    /// copies. The permit is released before the cache row is written, because the
    /// gate covers file content and not the cache's own writes.
    ///
    /// **Cache rows are written as the copies complete**, through the phase's
    /// batched handle, rather than collected and inserted after the pool joins.
    /// [`CommitLimits`](crate::cache::CommitLimits) is checked on each `put`; if the
    /// rows were hoisted to the end, every threshold would be evaluated after the
    /// last file was already durable on disk, so it would bound nothing during the
    /// copy phase. The handle is behind a mutex for the reason
    /// [`CacheWrite`](crate::cache::CacheWrite) is not `Sync` (it owns a live write
    /// transaction), not because the writes want to be serialised: the lock is held
    /// for one `put`, never across a copy.
    pub(crate) fn copy_files(&self, plan: &Plan) -> Result<usize> {
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
        // is held for exactly one emit, never across the copy or the cache write, so
        // it does not serialise the work it is protecting.
        //
        // **It cannot deadlock.** It is a leaf: nothing is called while it is held,
        // no other lock is ever held while waiting for it, and it is released before
        // the worker returns to the pool. The rw permits are released by then too
        // (they cover `copy_one` and not the bookkeeping after it), so a worker
        // waiting here holds nothing another worker needs and the permit wait-for
        // graph stays the `src -> dst` DAG it has to be.
        let emit = std::sync::Mutex::new(());
        // The phase's batched cache handle, written from each worker as that copy
        // verifies. `CacheWrite` owns a live `redb::WriteTransaction` and so is not
        // `Sync`; a mutex is all it needs, and cheaper than a channel here — a `put`
        // is one table insert, so the hand-off is per file rather than a queue, and
        // the interval since the last commit is measured when the batch is next
        // touched, which is now during the copies.
        //
        // It cannot deadlock either: held for one `put`, never while an rw permit is
        // held, and never nested with the emit lock above. A commit that trips
        // inside `put` only makes another worker's `put` wait; that worker holds no
        // permit and needs none.
        let cache = std::sync::Mutex::new(self.dst_db.begin_write()?);
        let results = self.rw.pool().install(|| {
            use rayon::prelude::*;
            plan.copy
                .par_iter()
                .map(|item| -> Result<()> {
                    // The permit covers the copy itself. Writing the row is cache
                    // work, which the rw gate deliberately does not cover, so it is
                    // done after the permit is released rather than stretching the
                    // permit over an unrelated fsync.
                    let r = {
                        let _rw = self.rw.acquire_copy();
                        debug!(rel = %item.rel, "copy start");
                        copy_one(self.src, self.dst, &item.rel, &self.common.algos)
                    };
                    let n = copy_done_cb.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    if n.is_multiple_of(25) || n == copy_total {
                        info!(done = n, total = copy_total, "copy progress");
                    } else {
                        trace!(done = n, total = copy_total, rel = %item.rel, "copy progress");
                    }
                    // A failed copy aborts the run before the summary, so no `COPY`
                    // line is announced and no row is written: a cache must never
                    // claim content the dst does not have.
                    let rec = r?;
                    // **As done.** Announced the moment the file is in place, not
                    // after every copy finishes — so with more than one rw operation
                    // permitted the `COPY` lines come out in completion order rather
                    // than plan order.
                    {
                        let _guard = emit.lock().expect("emit lock is never poisoned");
                        self.report.emit(
                            Record::path("COPY", &item.rel).put("because", item.because.as_str()),
                        );
                    }
                    // Only verified copies reach here, and the row is put now — while
                    // the copy phase is still running — so the batch handle's
                    // `CommitLimits` bound uncommitted work instead of checking three
                    // thresholds after the last file is already durable.
                    cache
                        .lock()
                        .expect("cache lock is never poisoned")
                        .put(&item.rel, &rec)?;
                    Ok(())
                })
                .collect::<Vec<_>>()
        });
        let mut copied = 0usize;
        // Plan order, so the failure reported is the lowest-index one rather than
        // whichever worker happened to lose. The `COPY` lines were already emitted
        // above, as each copy completed, and the rows were already written.
        for (item, r) in plan.copy.iter().zip(results) {
            r.with_context(|| format!("copy {}", item.rel))?;
            copied += 1;
        }
        info!(copied, total = copy_total, "copy done");
        // The final partial batch. Batches the thresholds committed during the copy
        // phase are already durable; a run that errored above never reaches this, so
        // its uncommitted rows are discarded with the transaction.
        cache
            .into_inner()
            .expect("cache lock is never poisoned")
            .commit()?;
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
    pub(crate) fn remove_unknown_dirs(&self, plan: &Plan) -> Result<usize> {
        let mut removed_dirs = 0usize;
        info!(unknown = plan.rmdir.len(), "rmdir pass");
        let mut w = self.dst_db.begin_write()?;
        // Cache prefixes this pass removed recursively. A link takes its whole
        // followed subtree with it, and `remove_dir_all` can take rows the plan
        // did not enumerate; those rows describe paths that are gone.
        let mut wiped: Vec<String> = Vec::new();
        for (i, r) in plan.rmdir.iter().enumerate() {
            // The plan excludes links and everything under one; re-check disk as
            // the belt-and-braces half, so a recursive removal can never run
            // through a link component.
            let rel = crate::link::topmost_link_ancestor(self.dst, r)
                .with_context(|| format!("check link ancestors for {r}"))?
                .unwrap_or_else(|| r.clone());
            let p = self.dst.join(&rel);
            if crate::link::try_is_link(&p)
                .with_context(|| format!("check link {}", p.display()))?
            {
                remove_link(&p)?;
                w.remove(r)?;
                if rel != *r {
                    w.remove(&rel)?;
                }
                wiped.push(format!("{rel}/"));
                self.report.emit(Record::path("RMDIR", &rel));
                removed_dirs += 1;
            } else {
                match std::fs::symlink_metadata(&p) {
                    Ok(md) if md.is_dir() => {
                        remove_dir_all(&p)?;
                        w.remove(r)?;
                        wiped.push(format!("{r}/"));
                        self.report.emit(Record::path("RMDIR", r));
                        removed_dirs += 1;
                    }
                    // The plan said directory. A file here means dst changed
                    // under the plan; refusing beats reporting a removal the
                    // run did not make.
                    Ok(_) => bail!("rmdir {}: not a directory", p.display()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e).with_context(|| format!("stat {}", p.display())),
                }
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
        if !wiped.is_empty() {
            for key in w.load_keys()? {
                if wiped.iter().any(|p| key.starts_with(p.as_str())) {
                    w.remove(&key)?;
                }
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
    /// **Subtracted** — what a copy or an unlink destroys. `rmdir` drops its own
    /// rows, and `delete_files` drops the row it names. The subtrees are the two
    /// that need this pass: `copy_one` clearing a blocking dst directory
    /// *recursively*, and a `delete_files` link, whose followed descendants were
    /// never paths of their own on disk. `Plan::rmdir` deliberately excludes the
    /// copy-wiped subtree from its own list — the directory is gone by the time
    /// the removal pass runs, so listing it would promise an `rmdir` the run
    /// cannot reach — which leaves this the place that can tell "dst held this
    /// before the run" from "dst holds it now".
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
    pub(crate) fn prune_dst_cache(&self, plan: &Plan) -> Result<()> {
        // dst paths this run's copies destroy, as `dir/` prefixes. Computed from
        // `dm` rather than the plan's own lists, so it names exactly the subtrees
        // `copy_one` will clear: a copy whose dst is a directory.
        let wiped: Vec<String> = plan
            .copy
            .iter()
            .filter(|item| self.dm.get(&item.rel).is_some_and(|rec| rec.kind == "dir"))
            .map(|item| format!("{}/", item.rel))
            .chain(
                plan.delete_files
                    .iter()
                    .filter(|r| self.dm.get(*r).is_some_and(|rec| rec.is_link))
                    .map(|r| format!("{r}/")),
            )
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

/// Remove a link itself, treating "already gone" as success.
///
/// A removal error used to be swallowed when the path no longer existed, which
/// also hid a failed unlink of a dangling link. `NotFound` is the one error that
/// means there is nothing left to remove; everything else aborts the run.
fn remove_link(p: &std::path::Path) -> Result<()> {
    match crate::link::remove(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("unlink {}", p.display())),
    }
}

/// Remove a regular file, treating "already gone" as success.
fn remove_file(p: &std::path::Path) -> Result<()> {
    match std::fs::remove_file(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("delete {}", p.display())),
    }
}

/// Remove a directory tree, treating "already gone" as success.
///
/// The caller has already established that `p` is a directory and not a link,
/// so this never resolves a link as the final component. An intermediate link
/// is the caller's guard, not this function's.
fn remove_dir_all(p: &std::path::Path) -> Result<()> {
    match std::fs::remove_dir_all(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("rmdir {}", p.display())),
    }
}
