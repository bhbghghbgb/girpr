//! Executing a sync plan against the filesystem.
//!
//! All shared state is hung off [`Applier`] so each phase reads as a short list
//! of steps. The phase *order* is a correctness invariant, so it lives in one
//! place ([`Applier::apply`]) rather than being spread across the coordinator:
//!
//! 1. fix-dirs, then mkdir (create the shapes src expects; a fix replaces the
//!    blocking file a nested directory needs),
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
use std::collections::HashMap;
use std::path::Path;
use tracing::info;

use super::plan::Plan;
use crate::cache::{CacheDb, FileRec};
use crate::config::CommonOpts;
use crate::effective::EffRec;
use crate::report::{Record, Report};
use crate::rw::RwRuntime;

mod copy;
mod phases;

/// Everything the apply phases share: both roots, the dst cache, the two
/// resolved side maps, and the resolved config.
pub(super) struct Applier<'a> {
    pub src: &'a Path,
    /// `src` as the input validation resolved it. The removal passes compare a
    /// delete target's directory against this, so a delete can never reach a
    /// file the run is copying even if a plan names one.
    pub src_canonical: &'a Path,
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
        // `fix_dirs` before `mkdirs`: a src directory can be nested under a dst
        // *file*, in which case the plan holds the file in `fix_dirs` and its
        // descendants in `mkdir`. `fix_dirs` removes the blocker and creates the
        // replacement directory with `create_dir_all`, so the nested `mkdir` only
        // succeeds after it. The other order aborted a valid plan with
        // `AlreadyExists` (os error 183) after the dry run had promised success.
        self.fix_dirs(plan)?;
        self.mkdirs(plan)?;
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
    /// Runs after [`Applier::fix_dirs`], so a directory nested under a path dst
    /// held as a file has its parent in place by the time this runs.
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
            self.guard_no_link_ancestor(r, "create")?;
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
            // A link *above* this path would resolve the removal and the mkdir
            // into its target. A link *at* it is the type conflict itself, and
            // is unlinked below.
            self.guard_no_link_ancestor(r, "replace")?;
            let p = self.dst.join(r);
            if crate::link::try_is_link(&p)
                .with_context(|| format!("check link {}", p.display()))?
            {
                crate::link::remove(&p).with_context(|| format!("unlink {}", p.display()))?;
            } else if p.is_file() {
                std::fs::remove_file(&p).with_context(|| format!("remove {}", p.display()))?;
            } else if p.is_dir() {
                std::fs::remove_dir_all(&p).with_context(|| format!("rmdir {}", p.display()))?;
            }
            std::fs::create_dir_all(&p).with_context(|| format!("mkdir {}", p.display()))?;
            self.report.emit(Record::path("FIX-DIR", r));
        }
        // The directory exists now, so it owes a row like `mkdirs` writes. The
        // old row was removed and committed above; leaving the new one out made
        // `compare-self` report the directory this run created as extra.
        let mut w = self.dst_db.begin_write()?;
        for r in &plan.fix_dirs {
            w.put(r, &FileRec::dir())?;
        }
        w.commit()?;
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
            // Removing a link removes everything the walk reported under it.
            // Those rows describe paths that will not exist after the unlink, so
            // they are dropped here, before the filesystem change, rather than
            // left for `compare-self` to report as missing.
            if self.dm.get(r).is_some_and(|rec| rec.is_link) {
                let prefix = format!("{r}/");
                for key in w.load_keys()? {
                    if key.starts_with(&prefix) {
                        w.remove(&key)?;
                    }
                }
            }
        }
        w.commit()?;
        Ok(())
    }

    /// Fail before touching a path whose ancestor is a `dst` link.
    ///
    /// The plan-level check (`ensure_link_safe`) runs before any apply phase;
    /// this is the per-phase belt and braces, so a path that changed since the
    /// plan (or one a future phase adds) cannot resolve a link as an
    /// intermediate component. The final component is deliberately not checked:
    /// a link *at* the path is what a type conflict replaces.
    fn guard_no_link_ancestor(&self, rel: &str, action: &str) -> Result<()> {
        if let Some(link) = crate::link::link_ancestor_above(self.dst, rel)? {
            bail!(
                "refusing to {action} `{rel}` through dst link `{link}`: \
                 remove the link, or exclude it with --exclude"
            );
        }
        Ok(())
    }
}
