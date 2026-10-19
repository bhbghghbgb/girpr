//! The single-writer lock (docs/07 §4.1, P5).
//!
//! Two `girpr` runs against the same install are harmless under the
//! temp-then-promote path — they share one `*_tmp`, its MD5 fails, and
//! nothing is promoted. They are **not** harmless under the in-place path:
//! two runs with different `--audio` / `--include` plans write conflicting
//! bytes into one file, and a `discard` deletes a file the other run has
//! open (docs/07 §6, D1).
//!
//! So a run takes a lock before it writes anything: `{game_dir}/girpr.lock`,
//! created with `create_new` so the creation itself is the mutual
//! exclusion. The file holds the pid and the time the run started, and a
//! lock older than a day is taken over — a run that was killed must not
//! block the next one forever.
//!
//! The lock is only taken when the path scope **admits** it — the same
//! rule `config.ini` lives under. A run confined to a subtree does not
//! leave a lock file at the root, and the files-cleanup keeps the lock
//! either way.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::repair::scope::PathScope;

/// The lock file's name, under the game dir.
pub const LOCK_FILE: &str = "girpr.lock";

/// How old a lock may be before it is taken over: 24 hours.
///
/// Long enough that a long repair is never interrupted by a second run,
/// short enough that a killed run does not block the next one for a day.
pub const STALE_AFTER_SECS: i64 = 24 * 60 * 60;

/// A held lock. Dropping it releases the lock.
#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
}

impl Drop for Lock {
    fn drop(&mut self) {
        // Best effort: a lock that outlives its run is stale by definition,
        // and the next run takes it over.
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Why the lock could not be taken.
#[derive(Debug)]
pub enum LockError {
    /// Another run holds it.
    Held {
        path: PathBuf,
        /// The pid that holds it, when the lock file says so.
        pid: Option<u32>,
    },
    /// The lock file could not be created.
    Io(anyhow::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Held { path, pid } => write!(
                f,
                "another girpr run holds the lock at {} (pid {}); wait for it to finish, \
                 or delete that file if the run is gone",
                path.display(),
                pid.map(|p| p.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            ),
            LockError::Io(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for LockError {}

/// Take the lock for `game_dir`.
///
/// `Ok(None)` means **no lock was needed**: the path scope does not admit
/// the lock file, so the run is confined to a subtree and must not leave a
/// lock at the root.
pub fn acquire(game_dir: &Path, scope: &PathScope) -> Result<Option<Lock>, LockError> {
    let path = game_dir.join(LOCK_FILE);
    if !scope.is_visible(LOCK_FILE) {
        tracing::debug!(
            "the path scope does not admit {LOCK_FILE} -> no lock is taken"
        );
        return Ok(None);
    }
    match create(&path) {
        Ok(()) => Ok(Some(Lock { path })),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // The lock exists. Is it live?
            let info = read_info(&path);
            if info.is_stale(&path) {
                tracing::warn!(
                    path = %path.display(),
                    "the lock is stale -> taking it over"
                );
                let _ = std::fs::remove_file(&path);
                match create(&path) {
                    Ok(()) => Ok(Some(Lock { path })),
                    Err(e) => Err(LockError::Io(anyhow::Error::new(e).context(format!(
                        "take over the stale lock {}",
                        path.display()
                    )))),
                }
            } else {
                Err(LockError::Held {
                    path,
                    pid: info.pid,
                })
            }
        }
        Err(e) => Err(LockError::Io(
            anyhow::Error::new(e).context(format!("create the lock {}", path.display())),
        )),
    }
}

/// Create the lock file, exclusively.
fn create(path: &Path) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let pid = std::process::id();
    let now = chrono::Utc::now().timestamp();
    // The content is a courtesy to whoever finds the file: the *existence*
    // of the file is the lock, and the mtime is the fallback for a file
    // whose content was truncated.
    let _ = writeln!(f, "pid={pid}");
    let _ = writeln!(f, "started={now}");
    let _ = f.flush();
    Ok(())
}

/// What a lock file says about its holder.
#[derive(Debug, Default)]
struct LockInfo {
    pid: Option<u32>,
    started: Option<i64>,
}

impl LockInfo {
    /// Is this lock old enough to take over?
    fn is_stale(&self, path: &Path) -> bool {
        let now = chrono::Utc::now().timestamp();
        if let Some(started) = self.started {
            return now.saturating_sub(started) > STALE_AFTER_SECS;
        }
        // No timestamp in the file: fall back to when it was last touched.
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| now.saturating_sub(d.as_secs() as i64) > STALE_AFTER_SECS)
            .unwrap_or(false)
    }
}

/// Read a lock file's pid and start time.
fn read_info(path: &Path) -> LockInfo {
    let Ok(text) = std::fs::read_to_string(path) else {
        return LockInfo::default();
    };
    let mut info = LockInfo::default();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("pid=") {
            info.pid = v.trim().parse().ok();
        } else if let Some(v) = line.strip_prefix("started=") {
            info.started = v.trim().parse().ok();
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn scope(include: &[&str], exclude: &[&str]) -> PathScope {
        let inc: Vec<String> = include.iter().map(|s| s.to_string()).collect();
        let exc: Vec<String> = exclude.iter().map(|s| s.to_string()).collect();
        PathScope::new(&inc, &exc).unwrap()
    }

    #[test]
    fn a_fresh_lock_is_taken_and_a_second_one_is_refused() {
        let dir = crate::testtemp::TempDir::new("girpr_test_lock", "fresh");
        let path = dir.join(LOCK_FILE);
        let lock = acquire(&dir, &PathScope::default())
            .expect("the first lock is taken")
            .expect("a lock is taken");
        assert!(path.exists());
        let info = read_info(&path);
        assert_eq!(info.pid, Some(std::process::id()));
        assert!(info.started.is_some());

        // A second run is refused, and says whose lock it is.
        match acquire(&dir, &PathScope::default()) {
            Err(LockError::Held { pid, .. }) => {
                assert_eq!(pid, Some(std::process::id()));
            }
            other => panic!("a live lock must be refused, got {other:?}"),
        }

        // Releasing it lets the next run in.
        drop(lock);
        assert!(!path.exists());
        let _again = acquire(&dir, &PathScope::default())
            .expect("the lock is free again")
            .expect("a lock is taken");
    }

    #[test]
    fn a_stale_lock_is_taken_over() {
        let dir = crate::testtemp::TempDir::new("girpr_test_lock", "stale");
        let path = dir.join(LOCK_FILE);
        // A lock that was taken two days ago.
        let old = chrono::Utc::now().timestamp() - 2 * 24 * 60 * 60;
        std::fs::write(&path, format!("pid=999999\nstarted={old}\n")).unwrap();
        let lock = acquire(&dir, &PathScope::default())
            .expect("a stale lock is taken over")
            .expect("a lock is taken");
        let info = read_info(&path);
        assert_eq!(
            info.pid,
            Some(std::process::id()),
            "the stale holder is gone; the new run holds it"
        );
        drop(lock);
    }

    #[test]
    fn a_lock_the_scope_does_not_admit_is_not_taken() {
        let dir = crate::testtemp::TempDir::new("girpr_test_lock", "scoped");
        // A scope that admits only a subtree does not admit the lock file
        // at the root: the run must not leave one there.
        let s = scope(&["game/foo.dat"], &[]);
        assert!(!s.is_visible(LOCK_FILE));
        let lock = acquire(&dir, &s).expect("no lock is needed");
        assert!(lock.is_none());
        assert!(!dir.join(LOCK_FILE).exists());

        // An open scope admits everything, so the lock is taken.
        let lock = acquire(&dir, &PathScope::default())
            .expect("the lock is taken")
            .expect("a lock is taken");
        assert!(dir.join(LOCK_FILE).exists());
        drop(lock);
    }

    #[test]
    fn an_excluded_lock_path_is_not_admitted() {
        let s = scope(&[], &[LOCK_FILE]);
        assert!(!s.is_visible(LOCK_FILE));
        let _ = HashSet::<String>::new();
    }
}
