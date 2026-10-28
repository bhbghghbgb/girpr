//! The single-writer lock.
//!
//! Two `girpr` runs against one install can write conflicting bytes into the
//! same file — different `--audio` or `--include` plans, or a `discard` that
//! deletes a file the other run has open — and neither run can know about the
//! other.
//!
//! The lock is one file — `{game_dir}/girpr.lock` — created with
//! `create_new(true)`, so the *creation* is the mutual exclusion: there is no
//! window between "check" and "take". It holds the pid and the time it was
//! taken, so a refusal can name the run to look at.
//!
//! Two rules keep it from being a nuisance:
//!
//! - it is only taken by a run that **mutates** the install, so a
//!   `--check-only`, a `--dry-run` and a `--print-plan` never block
//!   anything;
//! - it is only taken when the path scope **admits** it, by the same rule
//!   `config.ini` lives under: a path outside the scope does not exist to
//!   the run, and the lock file is at the game dir's root.
//!
//! A lock that outlives its run (the run was killed) is taken over after
//! [`STALE_AFTER`]; a live one refuses the new run, by name and by pid. A stale
//! lock that cannot be removed is also refused — naming the file and the I/O
//! error — rather than retried, because no retry changes the state.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};

/// The lock file's name, under the game dir.
pub const LOCK_FILE: &str = "girpr.lock";

/// How long a lock may live before another run takes it over.
///
/// Long enough that a real run is never overtaken, short enough that a dead
/// one is not a wall.
pub const STALE_AFTER: Duration = Duration::from_secs(24 * 3600);

/// A held lock. The file is removed when the lock is dropped.
#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
}

impl Lock {
    /// Take the lock, or say why it could not be taken.
    ///
    /// The refusal names the file and the pid that holds it, because the
    /// person hitting it wants to know *which* run to look at.
    pub fn acquire(game_dir: &Path) -> Result<Lock> {
        Self::acquire_with(game_dir, |path| std::fs::remove_file(path))
    }

    /// The implementation, with the stale-lock removal injected so the
    /// undeletable arm is testable without a platform-specific filesystem
    /// fixture.
    ///
    /// At most **one** takeover is attempted. A successful removal cannot
    /// recur: the next `create_new` either wins or finds a lock created after
    /// the removal, which is fresh by definition. A failed removal is the
    /// run's refusal, not a reason to retry — a retry with no state change is
    /// an unbounded loop.
    fn acquire_with(
        game_dir: &Path,
        remove: impl Fn(&Path) -> std::io::Result<()>,
    ) -> Result<Lock> {
        let path = game_dir.join(LOCK_FILE);
        let mut f = match create_lock(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if !is_stale(&path) {
                    return Err(held(&path));
                }
                // Its run is gone: take it over. An undeletable lock (an open
                // handle without `FILE_SHARE_DELETE`, a read-only file, a
                // delete-denying ACL) fails here with the path and the I/O
                // error instead of recursing until the process aborts.
                remove(&path).with_context(|| {
                    format!(
                        "take over the stale lock {} (it cannot be removed)",
                        path.display()
                    )
                })?;
                match create_lock(&path) {
                    Ok(f) => f,
                    // Another run took the slot between the removal and the
                    // create. It is fresh, so refuse it by name; retrying
                    // would only race again.
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        return Err(held(&path));
                    }
                    Err(e) => {
                        return Err(e).with_context(|| format!("create {}", path.display()));
                    }
                }
            }
            Err(e) => return Err(e).with_context(|| format!("create {}", path.display())),
        };
        // The contents are for a human: which process holds it, and
        // since when. A lock whose body cannot be written is still a
        // lock — the file's existence is what excludes.
        let body = format!(
            "pid={}\nstarted={}\n",
            std::process::id(),
            chrono::Utc::now().to_rfc3339()
        );
        let _ = f.write_all(body.as_bytes());
        Ok(Lock { path })
    }

    /// The lock file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Create the lock file with `create_new`, so creation is the mutual exclusion.
///
/// Split out because the post-takeover attempt must use exactly the same
/// operation as the first one.
fn create_lock(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// The refusal for a live lock (or one a raced run just created).
fn held(path: &Path) -> anyhow::Error {
    anyhow::anyhow!(
        "another girpr run holds {} (pid {}); refusing to run",
        path.display(),
        holder_pid(path)
    )
}

impl Drop for Lock {
    fn drop(&mut self) {
        // The lock is the run's own file: removing it is how the next run
        // starts. A failure here is not worth failing the run over.
        std::fs::remove_file(&self.path).ok();
    }
}

/// Is the lock at `path` too old to be live?
///
/// The file's **modification time** is the test, not the timestamp in its
/// body: a body that cannot be parsed must not make the lock immortal, and
/// a truncated body (a run killed mid-write) is exactly the case the mtime
/// handles.
fn is_stale(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        // It vanished between the open and the stat: not stale, just gone.
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .map(|age| age > STALE_AFTER)
        .unwrap_or(false)
}

/// The pid recorded in a lock file, or `?` when it cannot be read.
///
/// The body is advisory: the lock's *existence* is what excludes, and the pid
/// is only there to name the run to look at.
fn holder_pid(path: &Path) -> String {
    let Ok(text) = std::fs::read_to_string(path) else {
        return "?".to_string();
    };
    for line in text.lines() {
        if let Some(pid) = line.strip_prefix("pid=") {
            return pid.trim().to_string();
        }
    }
    "?".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_lock_excludes_a_second_run() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lock", "fresh");
        let lock = Lock::acquire(&dir).expect("the first run takes the lock");
        assert!(lock.path().exists());
        // The second run is refused, and the refusal names the file.
        let err = Lock::acquire(&dir).expect_err("the second run is refused");
        let msg = format!("{err:#}");
        assert!(msg.contains(LOCK_FILE), "{msg}");
        assert!(msg.contains(&std::process::id().to_string()), "{msg}");
        // The lock is released when it is dropped.
        drop(lock);
        assert!(!dir.join(LOCK_FILE).exists());
        // And the next run takes it.
        let lock = Lock::acquire(&dir).expect("the lock is free again");
        assert!(lock.path().exists());
    }

    #[test]
    fn a_stale_lock_is_taken_over() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lock", "stale");
        // A lock file from a run that was killed: old enough to be stale.
        let path = dir.join(LOCK_FILE);
        let f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("make the stale lock");
        let old = SystemTime::now() - STALE_AFTER - Duration::from_secs(3600);
        f.set_times(std::fs::FileTimes::new().set_modified(old))
            .expect("age the lock");
        drop(f);

        let lock = Lock::acquire(&dir).expect("a stale lock is taken over");
        assert!(lock.path().exists());
    }

    /// U-2: a stale lock whose removal fails (an open handle without
    /// `FILE_SHARE_DELETE`, a read-only file, a delete-denying ACL) is refused
    /// once, with the path and the I/O error. The old code discarded the error
    /// and called `acquire` again with no state change, so this shape overflowed
    /// the stack (`exit -1073741571`) instead of exiting `1`.
    ///
    /// The removal is injected: that pins the arm on every host without needing
    /// a platform-specific undeletable file.
    #[test]
    fn an_undeletable_stale_lock_is_refused_once_not_retried() {
        use std::cell::Cell;

        let dir = girpr_tempdir::TempDir::new("girpr_test_lock", "undeletable");
        let path = dir.join(LOCK_FILE);
        let f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("make the stale lock");
        let old = SystemTime::now() - STALE_AFTER - Duration::from_secs(3600);
        f.set_times(std::fs::FileTimes::new().set_modified(old))
            .expect("age the lock");
        drop(f);

        let removals = Cell::new(0);
        let err = Lock::acquire_with(&dir, |p| {
            removals.set(removals.get() + 1);
            assert_eq!(p, path.as_path());
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "held open without share-delete",
            ))
        })
        .expect_err("an undeletable stale lock is a refusal");
        assert_eq!(removals.get(), 1, "one takeover attempt, not a retry loop");
        let msg = format!("{err:#}");
        assert!(msg.contains(LOCK_FILE), "the refusal names the lock: {msg}");
        assert!(
            msg.contains("held open without share-delete"),
            "the refusal keeps the I/O error: {msg}"
        );
        assert!(path.exists(), "the stale lock is left where it was");
    }

    #[test]
    fn a_live_lock_is_not_taken_over() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lock", "live");
        let path = dir.join(LOCK_FILE);
        std::fs::write(&path, "pid=1234\n").unwrap();
        // Fresh: not stale, so the lock stands.
        assert!(!is_stale(&path));
        assert!(Lock::acquire(&dir).is_err());
        assert_eq!(holder_pid(&path), "1234");
    }

    #[test]
    fn a_lock_with_an_unreadable_body_still_excludes() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lock", "body");
        let path = dir.join(LOCK_FILE);
        std::fs::write(&path, "not a pid at all").unwrap();
        assert_eq!(holder_pid(&path), "?");
        assert!(Lock::acquire(&dir).is_err());
    }
}
