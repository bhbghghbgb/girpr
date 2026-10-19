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
//! [`STALE_AFTER`]; a live one refuses the new run, by name and by pid.

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
        let path = game_dir.join(LOCK_FILE);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
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
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if is_stale(&path) {
                    // Its run is gone: take it over.
                    std::fs::remove_file(&path).ok();
                    return Lock::acquire(game_dir);
                }
                Err(anyhow::anyhow!(
                    "another girpr run holds {} (pid {}); refusing to run",
                    path.display(),
                    holder_pid(&path)
                ))
            }
            Err(e) => Err(e).with_context(|| format!("create {}", path.display())),
        }
    }

    /// The lock file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }
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
        let dir = crate::testtemp::TempDir::new("girpr_test_lock", "fresh");
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
        let dir = crate::testtemp::TempDir::new("girpr_test_lock", "stale");
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

    #[test]
    fn a_live_lock_is_not_taken_over() {
        let dir = crate::testtemp::TempDir::new("girpr_test_lock", "live");
        let path = dir.join(LOCK_FILE);
        std::fs::write(&path, "pid=1234\n").unwrap();
        // Fresh: not stale, so the lock stands.
        assert!(!is_stale(&path));
        assert!(Lock::acquire(&dir).is_err());
        assert_eq!(holder_pid(&path), "1234");
    }

    #[test]
    fn a_lock_with_an_unreadable_body_still_excludes() {
        let dir = crate::testtemp::TempDir::new("girpr_test_lock", "body");
        let path = dir.join(LOCK_FILE);
        std::fs::write(&path, "not a pid at all").unwrap();
        assert_eq!(holder_pid(&path), "?");
        assert!(Lock::acquire(&dir).is_err());
    }
}
