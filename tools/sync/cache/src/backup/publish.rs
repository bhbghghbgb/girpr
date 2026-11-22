//! The filesystem mechanics of publishing a cache copy.
//!
//! This module owns the **partial + rename** half of the backup protocol: a
//! copy is written under a unique working name ([`WORK_PREFIX`]), flushed, and
//! only then published under a name of a bounded list — by a hard link where
//! the filesystem allows it, and by a claim plus rename where it does not.
//!
//! The cross-process half of the protocol — the record a published copy
//! carries, the process-wide registries that spare a run's own copies, and the
//! age-ordered retention — stays in [`super::super::open`] and is reached from
//! here through the narrow [`PublishHooks`] interface. Nothing in this module
//! touches those records or registries directly.
//!
//! The invariants the protocol depends on are spelled out in the `open` module
//! docs: a name in a copy list never holds half a copy, and a publish whose
//! claim is no longer this run's fails loudly rather than landing over another
//! run's copy. The two-call gaps those docs describe are preserved verbatim by
//! the code here — a publish is still a link, or a claim, an identity check and
//! a rename, with no new advisory lock added around them.

use anyhow::{Context, Result, bail};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use tracing::{info, trace, warn};

use crate::helpers::copy_dir_all;
use crate::helpers::{self, FileId};

use super::super::open::{CacheHooks, Claim, PublishHooks};

/// The prefix of a copy that is still being made.
///
/// A copy is written under this prefix and published under one of the two
/// published prefixes only once it is complete, so the bound — which ranges
/// over the published copies — cannot see a copy that is still being written,
/// let alone select it.
///
/// The name is in the tool's own namespace (`girpr-cache*`), so a working
/// copy is tool state and is never mirrored as content. A working name is a
/// [`tempfile`] name: unique by atomic creation, with the start stamp folded
/// into the prefix so two runs still leave distinguishable scratch names.
///
/// **No bound reclaims a working name.** The bound ranges over the published
/// copies, and a working name is not one of them: a working name is a copy
/// that is still being made, and a bound that could select it could take the
/// copy another run is making there — the one thing the publish exists to
/// prevent. So a working name left behind — a run killed mid-copy, or a
/// removal that failed after the copy was published — stays in the folder
/// until it is removed by hand.
pub(crate) const WORK_PREFIX: &str = "girpr-cache-partial-";

impl<H: PublishHooks> FsPublisher<H> {
    /// Copy `db_path` beside itself as `<prefix><ts>`. A directory (a legacy sled
    /// cache) is copied recursively; a redb file is copied in one call. The
    /// destination never replaces an existing sibling.
    ///
    /// **The copy is published, not written in place.** It is written under a
    /// working name ([`WORK_PREFIX`]) and published under its own name once it
    /// is complete, so:
    ///
    /// - a name of a bounded list never holds half a copy, and
    /// - a copy that is still being made is not a member of any list, so no
    ///   bound — however often it is applied — can select it.
    ///
    /// The publish is what claims the published name, so two runs cannot publish
    /// over each other.
    ///
    /// **The copy is stamped twice, and the two stamps say different things.** The
    /// working name says when the copy *started*; it is scratch, and nothing but
    /// the run making it ever reads it. The published name says when the copy
    /// became *whole*, because that is the moment it joins a bounded list, and
    /// the bound orders the copies by that name. A published name that said when
    /// the copy started would make the copy of a slow run look old the moment it
    /// landed, and the bound would take it away from the run that had just made
    /// it.
    fn copy_one(
        &self,
        db_path: &Path,
        prefix: &str,
        label: &str,
        stamp: &mut dyn FnMut() -> String,
    ) -> Result<Option<PathBuf>> {
        if !db_path.exists() {
            trace!(path = %db_path.display(), "{label} skip: db missing");
            return Ok(None);
        }
        let parent = db_path.parent().unwrap_or_else(|| Path::new("."));
        let is_dir = db_path.is_dir();
        let work = working_name(parent, is_dir, &stamp())?;
        info!(src = %db_path.display(), dst = %work.display(), "{label} start");
        let copied: Result<()> = if is_dir {
            copy_dir_all(db_path, &work)
        } else {
            std::fs::copy(db_path, &work)
                .with_context(|| format!("copy {} -> {}", db_path.display(), work.display()))
                .map(|_| ())
        };
        if let Err(e) = copied {
            // The working copy is ours and the copy did not land. A half-written
            // file is not a backup, and leaving it would be a file a later run
            // reads as one — so it goes, and the failure is reported either way.
            let _ = remove_cache_path(&work);
            return Err(e);
        }
        // The published name is only ever a whole copy, so the bytes behind it
        // have to be on disk before the name appears: the name can reach the disk
        // before the data it names does.
        if let Err(e) = flush_copy(&work, is_dir) {
            let _ = remove_cache_path(&work);
            return Err(e);
        }
        match self.publish_copy(&work, parent, prefix, &stamp(), is_dir) {
            Ok(dest) => {
                self.hooks.remember(&dest);
                info!(src = %db_path.display(), dst = %dest.display(), "{label} done");
                Ok(Some(dest))
            }
            Err(e) => {
                // Nothing was published, so there is no backup to report: the
                // run fails rather than say it made one.
                let _ = remove_cache_path(&work);
                Err(e)
            }
        }
    }
}

/// Flush a finished copy before its name is published.
///
/// The publish puts the name on the copy, and the name can be durable before
/// the data it names is, so the bytes are flushed first. A legacy sled
/// directory is copied file by file and is left as it is: the live cache is a
/// file, and the directory case is a migration path rather than the one that
/// runs.
fn flush_copy(work: &Path, is_dir: bool) -> Result<()> {
    if is_dir {
        return Ok(());
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(work)
        .with_context(|| format!("open {}", work.display()))?;
    file.sync_all()
        .with_context(|| format!("flush {}", work.display()))
}

/// Publish a finished copy under a name of the bounded lists.
///
/// A file copy is published by **linking** it into place, so the name is
/// created with the copy's bytes in it: the name appears whole, and it
/// appears *with* the copy rather than a moment before it. The link is also
/// the claim — it fails when the name is taken — so two runs cannot publish
/// onto one name.
///
/// A directory cannot be linked, so its name is claimed with an exclusive
/// create and the copy is renamed onto the claim: a POSIX rename onto an
/// existing empty directory succeeds, so the claim is what keeps two runs
/// from publishing onto one name.
///
/// **The record of the copy is taken before the name is claimed.** The record
/// says the copy is in use by the run that is about to publish it, and a
/// bound that met the name without the record would have nothing to read and
/// would take the copy away from the run publishing it. Taking the record
/// first also settles the name: a run whose record for a name is already held
/// is looking at a name another run is publishing under, so it moves on to the
/// next one rather than racing for it.
///
/// A filesystem that cannot lock a file has no records to speak, so the copy
/// is published without one — as it was published before a record existed —
/// rather than not published at all. The record the attempt made goes with it:
/// a record nobody holds and nobody can lock reads as "a live run holds it"
/// to a bound, which would spare the copy for good.
///
/// **A claim that is taken away fails the publish.** A bound cannot tell a
/// claim from a published copy, and on a filesystem that cannot lock a file
/// there is no record beside the name to say the name is being claimed, so
/// the claim can go. The publish checks that the name still holds *its own*
/// claim before the copy lands on it — the same object, and not merely
/// something at the name — and fails loudly when it does not: landing the
/// copy on a name this run no longer holds would publish it over whatever
/// holds it now, which is the one thing the claim exists to prevent.
///
/// The check and the rename are two calls, and no portable rename is
/// conditional on the name holding this run's claim, so the gap between them
/// is one syscall wide: a claim taken away in it is still landed on. Only a
/// filesystem that can neither link nor lock a file gets there — with a link
/// the publish is atomic, and with a lock the record beside the name keeps a
/// bound off the claim until the run opens the folder's cache.
impl<H: PublishHooks> FsPublisher<H> {
    fn publish_copy(
        &self,
        work: &Path,
        parent: &Path,
        prefix: &str,
        stamp: &str,
        is_dir: bool,
    ) -> Result<PathBuf> {
        for n in 1..=MAX_CLAIM_ATTEMPTS {
            let name = candidate_name(prefix, stamp, n);
            let cand = parent.join(&name);
            // Ask first whether the name is taken: `create_new` on an existing
            // *directory* reports `Access is denied` rather than `AlreadyExists`,
            // so the error kind alone cannot distinguish "occupied" from "cannot
            // write here".
            match std::fs::symlink_metadata(&cand) {
                Ok(_) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).with_context(|| format!("stat {}", cand.display())),
            }
            // The record comes before the claim, so a published name is never
            // without one.
            let hold = match self.hooks.take_record(parent, &name)? {
                Claim::Ours(hold) => Some(hold),
                // A live run holds this name's record: the name is that run's,
                // and the next one is ours to try.
                Claim::Taken => continue,
                // The filesystem cannot lock a file, so there is no record to
                // keep: the copy is published as it was before a record existed,
                // and the run's own bound still spares it through the published
                // registry.
                Claim::Unusable => None,
            };
            let published = if is_dir {
                publish_dir(work, &cand)
            } else {
                publish_file(work, &cand)
            };
            match published {
                Ok(()) => {
                    self.hooks.keep_record(hold);
                    return Ok(cand);
                }
                // The name is another run's: the next one is ours to try.
                Err(PublishError::Taken) => {
                    self.hooks.discard_record(hold);
                    continue;
                }
                Err(PublishError::Io(e)) => {
                    self.hooks.discard_record(hold);
                    return Err(e).with_context(|| format!("publish {}", cand.display()));
                }
            }
        }
        bail!(
            "no free backup name in {} after {MAX_CLAIM_ATTEMPTS} tries",
            parent.display()
        )
    }
}

/// Why a copy did not land on the name it was published under.
pub(crate) enum PublishError {
    /// The name is taken: another run has it, and the next name is ours.
    Taken,
    /// The publish itself failed.
    Io(std::io::Error),
}

/// Publish a finished file copy under its published name.
///
/// The publish is a **link**: the name is created with the copy's bytes in
/// it, so there is no moment at which the name is there and empty. A
/// rename cannot do that on its own — the rename has to be preceded by a
/// claim of the name, and the claim leaves the name empty for as long as
/// the rename takes.
///
/// The link is also the claim: it fails when the name is taken, so two runs
/// cannot publish onto one name.
///
/// A filesystem that cannot link a file has no such publish. The copy is
/// then renamed onto a name it claims first, and the name is briefly empty
/// there — which is how a copy was published before links were used. The
/// claim is checked before the copy lands on it, so a claim that was taken
/// away fails the publish rather than landing the copy on a name this run no
/// longer holds.
///
/// The check and the rename are two calls, and no portable rename is
/// conditional on the name holding this run's claim, so the gap between them
/// is one syscall wide: a claim taken away in it is still landed on.
///
/// The rename has to be able to replace the claim for the copy to land at
/// all, and a filesystem whose rename cannot is one where the copy can never
/// be published. The claim is dropped and the publish fails there, rather
/// than dropped and tried again: a rename of a file replaces whatever is at
/// the name, so a second try could put the copy over another run's published
/// copy — and the only platform whose rename cannot replace anything at all
/// is the one where the copy never lands, so there is nowhere a second try
/// would be worth making.
///
/// **A rename that does not land drops the claim.** The copy is not
/// published, so the publish fails and the run reports the failure rather
/// than a backup that is not on disk. The claim is dropped with it, so the
/// name is left free for the next run to take — and it is dropped only while
/// the name still holds it: a name another run has taken over holds that
/// run's copy, and that copy is not this run's to remove.
///
/// The drop is best-effort — its error is ignored — so a removal that fails
/// leaves the claim behind, and the name is then not free after all. The
/// leftover is an empty file under a published prefix, which the bound reads
/// as a published copy.
fn publish_file(work: &Path, cand: &Path) -> Result<(), PublishError> {
    match link_file(work, cand) {
        Ok(()) => {
            // The name exists with the copy's bytes in it. The working name
            // is scratch, so it goes: the copy now lives under its
            // published name alone.
            #[cfg(test)]
            pause_publish();
            if let Err(e) = remove_working_copy(work) {
                warn!(
                    path = %work.display(),
                    error = %e,
                    "the working copy of a published copy could not be removed; the working name is left behind"
                );
            }
            Ok(())
        }
        // The name is taken: it is another run's.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(PublishError::Taken),
        Err(e) => {
            // The filesystem cannot link a file, so the copy is published by
            // renaming it onto a name it claims first.
            trace!(
                path = %cand.display(),
                error = %e,
                "the copy cannot be linked into place; it is renamed onto a claimed name"
            );
            // The name is claimed, and the claim is what the rename replaces.
            let file = match claim_name(cand) {
                Ok(file) => file,
                Err(e) if name_is_taken(&e) => return Err(PublishError::Taken),
                Err(e) => return Err(PublishError::Io(e)),
            };
            // The identity is read from the handle and not from the name: the
            // handle names the claim this run created even if the name has
            // been taken away since, so the answer cannot be another run's
            // file.
            let claim = match helpers::identity_of(&file) {
                Ok(id) => NameClaim {
                    id,
                    is_dir: false,
                    _file: Some(file),
                },
                Err(e) => return Err(PublishError::Io(e)),
            };
            // The publish is held still for a moment, so a test can look at
            // the folder in the middle of one: the claim is there, empty,
            // under a published name.
            #[cfg(test)]
            pause_publish();
            // The claim has to still be ours when the copy lands on it.
            if !claim_is_in_place(cand, &claim) {
                return Err(PublishError::Io(claim_taken_away()));
            }
            match rename_onto(work, cand) {
                Ok(()) => Ok(()),
                Err(e) => {
                    // The copy did not land on the claim: the claim is
                    // dropped, so the name is left free for the next run to
                    // take. It is dropped only while the name still holds
                    // it — a name another run has taken over holds that run's
                    // copy, and that copy is not this run's to remove. The
                    // drop is best-effort, so a removal that fails leaves the
                    // claim behind.
                    drop_the_claim(cand, &claim);
                    Err(PublishError::Io(e))
                }
            }
        }
    }
}

/// Link a file under a second name, if the filesystem can.
///
/// A filesystem that cannot link a file is not something a test can make, so
/// the failure is injected here: with the switch on, the link fails the way
/// it fails on such a filesystem.
fn link_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if LINKING_UNAVAILABLE.with(|f| f.get()) {
        return Err(std::io::Error::other("this filesystem cannot link a file"));
    }
    std::fs::hard_link(src, dst)
}

/// Publish a finished directory copy under its published name.
///
/// A directory cannot be linked, so the name is **claimed** with an
/// exclusive create and the copy is renamed onto the claim. The claim is
/// what decides: a POSIX rename onto an existing **empty** directory
/// succeeds, so a rename on its own would let a second run publish onto a
/// name the first run has, and replace the copy that is there.
///
/// The rename replaces the claim where the platform allows it, so the name
/// is briefly an empty directory. Where a rename refuses to replace a
/// directory, the claim is dropped and the rename tried again: a name taken
/// in the meantime makes that rename fail — a rename that cannot replace a
/// directory cannot replace the claim another run holds either — so the
/// publish moves on to the next name.
///
/// The drop and the rename are two calls, and where the removal succeeded the
/// name is free in between, so another run can take it in that window. The
/// gap is one syscall wide and cannot be closed; it is reachable only where
/// the filesystem cannot lock a file, since the record the publishing run
/// holds keeps another run off the name until the run opens the folder's
/// cache.
///
/// The claim is checked before the copy lands on it, so a claim that was
/// taken away — a bound cannot tell a claim from a published copy — fails
/// the publish rather than landing the copy on a name this run no longer
/// holds. It is checked again before the claim is dropped, because dropping
/// it takes whatever is at the name with it: a name another run has taken
/// over holds that run's copy, and that copy is not this run's to remove.
pub(crate) fn publish_dir(work: &Path, cand: &Path) -> Result<(), PublishError> {
    match std::fs::create_dir(cand) {
        Ok(()) => {}
        // The name is taken: it is another run's.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Err(PublishError::Taken),
        Err(e) => return Err(PublishError::Io(e)),
    }
    // The identity of the claim, read from the name: a directory has no
    // portable handle, and the name is what the copy is renamed onto. The
    // claim is gone if the name cannot be read at all.
    let Some(id) = identity_at(cand) else {
        return Err(PublishError::Io(claim_taken_away()));
    };
    let claim = NameClaim {
        id,
        is_dir: true,
        _file: std::fs::File::open(cand).ok(),
    };
    // The publish is held still for a moment, so a test can look at the
    // folder in the middle of one: the claim is there, empty, under a
    // published name.
    #[cfg(test)]
    pause_publish();
    // The claim has to still be ours when the copy lands on it.
    if !claim_is_in_place(cand, &claim) {
        return Err(PublishError::Io(claim_taken_away()));
    }
    match rename_onto(work, cand) {
        Ok(()) => Ok(()),
        Err(_) => publish_onto_the_claimed_name(work, cand, &claim),
    }
}

/// Publish the copy onto a name whose claim is in the way of the rename.
///
/// A rename cannot replace a directory on every platform, so the claim is
/// dropped and the rename is tried again.
///
/// The drop is best-effort, as it is everywhere else: its error is ignored,
/// so a removal that fails leaves the claim behind, and the rename then
/// fails with it — the name is still taken — and the publish moves on to the
/// next name.
///
/// **The drop and the rename are two calls.** Where the removal succeeded the
/// name is free between them, so another run can take it in that window — and
/// the rename fails then too, since a platform that cannot replace a
/// directory cannot replace the claim another run holds either: the name is
/// taken, and the publish moves on to the next name. The gap is one syscall
/// wide and cannot be closed; it is reachable only where the filesystem
/// cannot lock a file, since the record the publishing run holds keeps
/// another run off the name until the run opens the folder's cache.
///
/// **The claim is dropped only while the name still holds it.** A name that
/// holds something else — another run's claim, or another run's published
/// copy — is not ours to drop: dropping it would take that run's copy away
/// from under the run that is about to report it, and the copy is not landed
/// on it either. Only an empty directory goes, so a copy another run has
/// published with content in it is never dropped either.
///
/// The ask and the removal are two calls, and there is no portable call that
/// removes a claim by what it is rather than by what it is called, so a
/// claim taken away in the gap is still removed. The gap is one syscall wide
/// and cannot be closed; it is reachable only where the filesystem cannot
/// lock a file, since a record a live run holds keeps a bound off the name
/// until the run opens the folder's cache.
///
/// A claim that was taken away fails the publish, like every other claim
/// that is no longer this run's: the copy is not landed on a name this run
/// no longer holds.
pub(crate) fn publish_onto_the_claimed_name(
    work: &Path,
    cand: &Path,
    claim: &NameClaim,
) -> Result<(), PublishError> {
    if !drop_the_claim(cand, claim) {
        return Err(PublishError::Io(claim_taken_away()));
    }
    match rename_onto(work, cand) {
        Ok(()) => Ok(()),
        Err(e) => {
            // The name is there, so another run has it.
            if cand.exists() {
                Err(PublishError::Taken)
            } else {
                Err(PublishError::Io(e))
            }
        }
    }
}

/// Rename the copy onto the name it has claimed.
///
/// The rename is what lands the copy, and it is the call a platform refuses
/// when it cannot replace what is at the name — so it is also the call the
/// publish cannot do without, on a filesystem that cannot link a file.
fn rename_onto(work: &Path, cand: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if RENAME_UNAVAILABLE.with(|f| f.get()) {
        return Err(std::io::Error::other(
            "this filesystem cannot rename onto a claimed name",
        ));
    }
    std::fs::rename(work, cand)
}

// The test-only switch that makes the rename onto a claimed name fail.
//
// A filesystem that cannot rename onto a name it has claimed is not something
// a test can make, so the failure is injected here: with the switch on, the
// rename fails the way it fails on such a filesystem.
#[cfg(test)]
std::thread_local! {
    pub(crate) static RENAME_UNAVAILABLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

// The test-only switch that makes linking a file fail.
#[cfg(test)]
std::thread_local! {
    pub(crate) static LINKING_UNAVAILABLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

// The test-only switch that makes removing the working copy fail.
//
// A removal that fails is not something a test can make the same way on
// every platform, so the failure is injected here: with the switch on, the
// removal fails the way it fails when the file will not go.
#[cfg(test)]
std::thread_local! {
    pub(crate) static REMOVAL_UNAVAILABLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

// The test-only gate that pauses a publish once the name is there.
//
// A publish is over in microseconds, so a test that wants to look at the
// folder in the middle of one needs the publish to wait. The publishing
// thread installs a one-shot receiver here; `pause_publish` blocks on it
// until the main test has observed the claim and released its sender. There
// is no wall-clock sleep, so the race window is exact and costs no dead time.
#[cfg(test)]
std::thread_local! {
    static PUBLISH_GATE: std::cell::RefCell<Option<std::sync::mpsc::Receiver<()>>> =
        const { std::cell::RefCell::new(None) };
}

/// The send half of a one-shot publish gate, held by the test.
///
/// The main test releases it once it has observed the claim. If the test
/// panics or returns before releasing, dropping this sender closes the
/// channel and unblocks the paused publisher anyway, so a failure cannot
/// turn into a hang.
#[cfg(test)]
pub(crate) struct PublishGate {
    tx: Option<std::sync::mpsc::Sender<()>>,
}

#[cfg(test)]
impl PublishGate {
    /// Let the paused publisher continue.
    pub(crate) fn release(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Create a one-shot gate. The receiver arms the publishing thread; the
/// returned sender is kept by the test to release it once the claim is seen.
#[cfg(test)]
pub(crate) fn publish_gate() -> (std::sync::mpsc::Receiver<()>, PublishGate) {
    let (tx, rx) = std::sync::mpsc::channel();
    (rx, PublishGate { tx: Some(tx) })
}

/// Install the receive half of a publish gate in this (publishing) thread.
#[cfg(test)]
pub(crate) fn arm_publish_gate(rx: std::sync::mpsc::Receiver<()>) {
    PUBLISH_GATE.with(|f| *f.borrow_mut() = Some(rx));
}

/// Hold a publish still until the test releases its gate, so a test can look
/// at the folder deterministically in the middle of one.
#[cfg(test)]
fn pause_publish() {
    let rx = PUBLISH_GATE.with(|f| f.borrow_mut().take());
    if let Some(rx) = rx {
        // A message releases us; a dropped sender (a test that panicked
        // before releasing) makes `recv` return Err and releases us too.
        // Either way there is no fixed sleep and no way to hang here.
        let _ = rx.recv();
    }
}

/// Take a name for a published copy, so that no two runs publish onto one.
///
/// The handle is returned rather than dropped: it names the claim this run
/// created, and the publish asks the name whether it still names that claim
/// before the copy lands on it.
fn claim_name(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// The claim this run holds on a published name.
///
/// A claim is the name the run has taken for itself: an empty file for a file
/// copy, an empty directory for a directory copy. The copy is renamed onto it,
/// so the claim has to be **this run's** when the copy lands on it.
///
/// The identity is what says so. The name has to name the object this run
/// created — not merely hold *something*, which is a weaker question with a
/// weaker answer: a bound cannot tell a claim from a published copy, so on a
/// filesystem that cannot lock a file the claim can be taken away, and
/// another run can take the name and publish there.
pub(crate) struct NameClaim {
    /// The identity of the object the claim created.
    pub(crate) id: FileId,
    /// Whether the claim is a directory.
    ///
    /// A file copy is claimed with an empty file and a directory copy with
    /// an empty directory, and the two are dropped with different calls.
    pub(crate) is_dir: bool,
    /// The handle on the claim, kept open while the publish is in flight.
    ///
    /// It names the object even after the name has gone, so the identity
    /// cannot be freed and reused for another file while the publish decides
    /// whether the claim is still in place. A directory claim has no portable
    /// handle — `File::open` refuses a directory on Windows — and is named by
    /// the path alone there.
    pub(crate) _file: Option<File>,
}

/// The identity of the object at `path`, if the platform can name it.
pub(crate) fn identity_at(path: &Path) -> Option<FileId> {
    helpers::identity_at(path).ok()
}

/// Whether the claim on a published name is still this run's.
///
/// The claim is ours if the name still names the object this run created. A
/// name that holds something else — another run's published copy, or another
/// run's claim — is not ours, and the copy must not land on it: publishing
/// over another run's copy would leave that run reporting a backup that is
/// not on disk, which is the one thing the claim exists to prevent.
///
/// The answer is read at one moment, and the call that follows it is another
/// call: a claim taken away in between is still in reach. Nothing portable
/// makes the call that follows conditional on this answer.
fn claim_is_in_place(cand: &Path, claim: &NameClaim) -> bool {
    helpers::identity_at(cand).is_ok_and(|id| id == claim.id)
}

/// Drop the claim at a published name, and say whether the name still held
/// it.
///
/// The name is asked about **again** before the claim goes: a name another
/// run has taken over holds that run's copy, and that copy is not this run's
/// to remove — the copy is the one its run is about to report.
///
/// **The removal is best-effort.** The answer is about the name and not
/// about the removal: the error of the removal is ignored, and the claim is
/// reported as dropped either way. A removal that fails therefore leaves
/// the claim behind — an empty file under a published prefix for a file
/// claim, an empty directory for a directory one — and the bound reads that
/// name as a published copy: it counts towards the bound, and it may be the
/// one the bound keeps in place of a whole copy.
///
/// The ask and the removal are two calls, so a takeover in between is still
/// in reach: a claim has no portable handle to be removed by identity, and a
/// directory cannot be removed by what it is rather than by what it is
/// called. The gap is therefore one syscall wide, and it cannot be closed.
/// It is reachable only where the filesystem cannot lock a file: a record a
/// live run holds keeps a bound off the name until the run opens the
/// folder's cache.
pub(crate) fn drop_the_claim(cand: &Path, claim: &NameClaim) -> bool {
    if !claim_is_in_place(cand, claim) {
        return false;
    }
    let _ = if claim.is_dir {
        std::fs::remove_dir(cand)
    } else {
        std::fs::remove_file(cand)
    };
    true
}

/// The error a publish fails with when its claim was taken away.
fn claim_taken_away() -> std::io::Error {
    std::io::Error::other("the claimed name was taken away before the copy landed on it")
}

/// Remove the working copy of a published copy.
///
/// The working name is scratch: the copy now lives under its published name
/// alone, so the working name goes. A removal that fails is not a failure of
/// the publish — the copy is published and whole — so the caller warns about
/// the leftover rather than failing the run over it.
fn remove_working_copy(work: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if REMOVAL_UNAVAILABLE.with(|f| f.get()) {
        return Err(std::io::Error::other("this file cannot be removed"));
    }
    std::fs::remove_file(work)
}

/// Whether an error says the name was taken rather than that the copy failed.
fn name_is_taken(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::AlreadyExists
            | std::io::ErrorKind::DirectoryNotEmpty
            | std::io::ErrorKind::NotADirectory
    )
}

/// How many candidate names a claim tries before it gives up.
///
/// The `_2`, `_3` … spellings mean a candidate is only ever taken by a run
/// that collided on the same microsecond, so the bound is generous on
/// purpose: it is there so a pathological directory cannot spin the loop
/// forever, not because the number is expected to be reached.
pub(crate) const MAX_CLAIM_ATTEMPTS: u32 = 1000;

/// Claim a free `<parent>/<prefix><ts>` sibling and return it.
///
/// The name is **claimed**, not chosen: `create_new` for a file and
/// `create_dir` for a legacy sled directory are what take it, so two runs
/// against one root cannot pick the same sibling. Probing for a free name
/// and then creating it has a window between the two, and that window is
/// exactly where two concurrent runs would collide.
///
/// The fallback names are the `_2`, `_3` … spellings the tool has always
/// used, so a run that collides on the timestamp still leaves a
/// recognisable sibling.
///
/// Production working names now come from [`tempfile`] (see [`working_name`]),
/// which folds the same start stamp into its prefix but gets uniqueness from
/// a random suffix instead of this `_2`, `_3` loop. The primitive is kept for
/// its tests, which pin the name family the tool has always published under;
/// the published name itself is still claimed by [`publish_copy`], once the
/// copy is complete.
#[cfg(test)]
pub(crate) fn claim_sibling(
    parent: &Path,
    prefix: &str,
    stamp: &str,
    is_dir: bool,
) -> Result<PathBuf> {
    for n in 1..=MAX_CLAIM_ATTEMPTS {
        let cand = parent.join(candidate_name(prefix, stamp, n));
        // Ask first whether the name is taken: `create_new` on an existing
        // *directory* reports `Access is denied` rather than `AlreadyExists`,
        // so the error kind alone cannot distinguish "occupied" from "cannot
        // write here".
        match std::fs::symlink_metadata(&cand) {
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("stat {}", cand.display())),
        }
        let claimed = if is_dir {
            std::fs::create_dir(&cand).map(drop)
        } else {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&cand)
                .map(drop)
        };
        match claimed {
            Ok(()) => return Ok(cand),
            Err(e) => {
                // A racing creator, or a candidate that appeared between the
                // stat and the claim: the name is ours only if we created it.
                match std::fs::symlink_metadata(&cand) {
                    Ok(_) => continue,
                    Err(_) => {
                        return Err(e).with_context(|| format!("claim {}", cand.display()));
                    }
                }
            }
        }
    }
    bail!(
        "no free backup name in {} after {MAX_CLAIM_ATTEMPTS} tries",
        parent.display()
    )
}

/// The `n`th candidate name for a prefix and a timestamp: the plain one, then
/// `_2`, `_3`, and so on.
pub(crate) fn candidate_name(prefix: &str, stamp: &str, n: u32) -> String {
    if n == 1 {
        format!("{prefix}{stamp}")
    } else {
        format!("{prefix}{stamp}_{n}")
    }
}

/// The filesystem half of publishing a copy.
///
/// The cross-process record protocol is supplied by the caller through
/// [`PublishHooks`]; this trait is only the partial-write and publish
/// mechanics that can be exercised against a plain filesystem.
pub(crate) trait Publisher {
    /// Copy `db_path` beside itself under `prefix`, returning the published
    /// path, or `None` when there was nothing to copy.
    fn copy_aside(
        &self,
        db_path: &Path,
        prefix: &str,
        label: &str,
        stamp: &mut dyn FnMut() -> String,
    ) -> Result<Option<PathBuf>>;
}

/// The default [`Publisher`]: real filesystem operations, with the
/// cross-process record protocol reached through [`PublishHooks`].
pub(crate) struct FsPublisher<H: PublishHooks> {
    hooks: H,
}

impl<H: PublishHooks> FsPublisher<H> {
    pub(crate) fn new(hooks: H) -> Self {
        Self { hooks }
    }
}

impl<H: PublishHooks> Publisher for FsPublisher<H> {
    fn copy_aside(
        &self,
        db_path: &Path,
        prefix: &str,
        label: &str,
        stamp: &mut dyn FnMut() -> String,
    ) -> Result<Option<PathBuf>> {
        self.copy_one(db_path, prefix, label, stamp)
    }
}

/// Remove a path that is either a cache file or a legacy sled directory.
///
/// A path that is already gone is not a failure: two runs bounding one
/// folder can pick the same sibling, and the second one to reach it finds
/// nothing there. This is the copy mechanic's own copy of the helper the
/// cache layer re-exports, so the publish mechanics do not depend on the
/// registries' module.
pub(crate) fn remove_cache_path(db_path: &Path) -> Result<()> {
    if db_path.is_dir() {
        match std::fs::remove_dir_all(db_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("remove {}", db_path.display())),
        }
    } else if db_path.is_file() {
        match std::fs::remove_file(db_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("remove {}", db_path.display())),
        }
    }
    Ok(())
}

/// Create a unique working name under `parent` for a copy that starts now.
///
/// The start `stamp` is folded into the prefix so the name still says when the
/// copy began, but uniqueness and atomic creation come from [`tempfile`]
/// rather than from a hand-rolled `create_new` loop: the library retries a
/// collision with a fresh random suffix, so two runs cannot share a working
/// name. The returned path is kept — its removal is the publish's business,
/// and the working-copy failure seam has to stay under the publisher's
/// control — so no temporary object is dropped here.
fn working_name(parent: &Path, is_dir: bool, stamp: &str) -> Result<PathBuf> {
    let prefix = format!("{WORK_PREFIX}{stamp}");
    let mut builder = tempfile::Builder::new();
    builder.prefix(&prefix);
    if is_dir {
        builder
            .tempdir_in(parent)
            .map(|dir| dir.keep())
            .with_context(|| format!("create working directory in {}", parent.display()))
    } else {
        builder
            .tempfile_in(parent)
            .and_then(|file| {
                let (_handle, path) = file.into_parts();
                path.keep().map_err(Into::into)
            })
            .with_context(|| format!("create working file in {}", parent.display()))
    }
}

/// The production entry point for a backup or snapshot copy.
///
/// The publisher is parameterised by the record hooks so the mechanics stay
/// testable against a plain filesystem; production supplies the cache layer's
/// real hooks. The `stamp` closure is called once for the working name and
/// once, later, for the published name — the two stamps say different things.
pub(crate) fn copy_aside(
    db_path: &Path,
    prefix: &str,
    label: &str,
    mut stamp: impl FnMut() -> String,
) -> Result<Option<PathBuf>> {
    FsPublisher::new(CacheHooks).copy_aside(db_path, prefix, label, &mut stamp)
}
