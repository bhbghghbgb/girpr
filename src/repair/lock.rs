//! The single-writer lock.
//!
//! Two `girpr` runs against one install can write conflicting bytes into the
//! same file — different `--audio` or `--include` plans, or a `discard` that
//! deletes a file the other run has open — and neither run can know about the
//! other.
//!
//! The lock is an **OS advisory lock** on a file outside the install:
//! `{temp}/girpr-{key:016x}.lock`, where `key` is an FNV-1a hash of the
//! canonical game directory (ASCII case folded on Windows, because the
//! filesystem is). Every run that mutates the install takes it, regardless of
//! the path scope, so a scoped run cannot write beside an unscoped one.
//!
//! The open handle holds the lock: the OS releases it when the process dies,
//! so a killed run never leaves a wall-clock wait behind. The file body names
//! the pid and the time it was taken, so a refusal can name the run to look
//! at. A leftover file with no holder is harmless: the next `try_lock`
//! succeeds and overwrites its body. `Drop` only releases the handle; it never
//! removes the file, because deleting it after unlocking would let a second and
//! a third run hold two different files for the same key at once.
//!
//! Read-only runs (`--check-only`, `--dry-run`, `--print-plan`) take no lock.
//!
//! Older runs wrote `{game_dir}/girpr.lock`; this one never creates it. If
//! one is left over it is left alone (and kept by the cleanup) rather than
//! deleted behind the user's back.

use std::fs::{File, TryLockError};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The in-install lock file older runs created. Kept only so the cleanup
/// recognizes and preserves it; this module never creates it.
pub const LOCK_FILE: &str = "girpr.lock";

/// FNV-1a: small, stable, and specified — unlike `DefaultHasher`, whose
/// algorithm is not promised across releases.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// A held lock. The open handle is what excludes; dropping it unlocks.
#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
    file: File,
}

impl Lock {
    /// Take the lock, or say why it could not be taken.
    ///
    /// The refusal names the file and the holder from its body, because the
    /// person hitting it wants to know *which* run to look at. An exclusive
    /// lock can only be held by a live process, so there is no staleness
    /// rule: the OS releases a dead owner's handle.
    pub fn acquire(game_dir: &Path) -> Result<Lock> {
        Self::acquire_at(lock_path(game_dir))
    }

    /// The implementation, with the path injected so tests can take two
    /// locks for one key without depending on the global temp directory.
    fn acquire_at(path: PathBuf) -> Result<Lock> {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("open lock file {}", path.display()))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(held(&path)),
            Err(TryLockError::Error(e)) => {
                return Err(e).with_context(|| format!("lock {}", path.display()));
            }
        }
        // The contents are for a human: which process holds it, and since
        // when. A lock whose body cannot be written is still a lock — the
        // open handle and its OS lock are what exclude.
        let body = format!(
            "pid={}\nstarted={}\n",
            std::process::id(),
            chrono::Utc::now().to_rfc3339()
        );
        if file.set_len(0).is_ok() && file.seek(SeekFrom::Start(0)).is_ok() {
            let _ = file.write_all(body.as_bytes());
        }
        Ok(Lock { path, file })
    }

    /// The lock file's path (in the temp directory, keyed by the game dir).
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Does a run with these read-only flags mutate the install and therefore
/// need the lock?
///
/// `check_only` is already part of `readonly`, but stating it keeps the
/// contract explicit. `--purge-only` is not read-only and does take the lock.
pub(crate) fn takes_lock(readonly: bool, check_only: bool, print_plan: bool) -> bool {
    !readonly && !check_only && !print_plan
}

/// Where the lock for `game_dir` lives: the temp directory, keyed by the
/// canonical path so every spelling of the same install agrees.
fn lock_path(game_dir: &Path) -> PathBuf {
    let canonical = game_dir
        .canonicalize()
        .unwrap_or_else(|_| game_dir.to_path_buf());
    std::env::temp_dir().join(format!("girpr-{:016x}.lock", key_for(&canonical)))
}

/// A stable key for a canonical path.
fn key_for(path: &Path) -> u64 {
    let mut key = FNV_OFFSET;
    for byte in path.as_os_str().to_string_lossy().bytes() {
        #[cfg(windows)]
        let byte = byte.to_ascii_lowercase();
        key ^= u64::from(byte);
        key = key.wrapping_mul(FNV_PRIME);
    }
    key
}

impl Drop for Lock {
    fn drop(&mut self) {
        // The OS handle is the exclusion; releasing it lets the next run in.
        // The file is deliberately left behind. Removing it after unlocking
        // would race a run that opens the same path between our unlock and our
        // delete: it holds the old file while a third run creates a new one,
        // and both then think they are the single writer. A leftover unlocked
        // file is harmless because the next `try_lock` succeeds.
        let _ = self.file.unlock();
    }
}

/// The refusal for a lock that a live process holds.
fn held(path: &Path) -> anyhow::Error {
    match std::fs::read_to_string(path) {
        Ok(body) => {
            let pid = field(&body, "pid").unwrap_or("?");
            match field(&body, "started") {
                Some(started) => anyhow::anyhow!(
                    "another girpr run holds {} (pid {}, started {}); refusing to run",
                    path.display(),
                    pid,
                    started
                ),
                None => anyhow::anyhow!(
                    "another girpr run holds {} (pid {}); refusing to run",
                    path.display(),
                    pid
                ),
            }
        }
        Err(_) => anyhow::anyhow!(
            "another girpr run holds {} (its body is unreadable); refusing to run",
            path.display()
        ),
    }
}

/// One `name=value` field from a lock body.
fn field<'a>(body: &'a str, name: &str) -> Option<&'a str> {
    body.lines().find_map(|line| {
        line.strip_prefix(name)
            .and_then(|rest| rest.strip_prefix('='))
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_lock_excludes_a_second_run() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lock", "fresh");
        let lock = Lock::acquire(&dir).expect("the first run takes the lock");
        assert!(lock.path().exists());
        // The lock lives outside the install, keyed by its canonical path.
        assert_eq!(lock.path().parent(), Some(std::env::temp_dir().as_path()));
        let name = lock.path().file_name().unwrap().to_string_lossy();
        assert!(
            name.starts_with("girpr-") && name.ends_with(".lock"),
            "{name}"
        );
        // The second run is refused, and the refusal names the file.
        let err = Lock::acquire(&dir).expect_err("the second run is refused");
        let msg = format!("{err:#}");
        assert!(
            msg.contains(lock.path().to_string_lossy().as_ref()),
            "{msg}"
        );
        // An exclusive OS lock can hide the body from another process (on
        // Windows it does); the refusal says so rather than inventing a pid.
        assert!(
            msg.contains(&std::process::id().to_string()) || msg.contains("body is unreadable"),
            "{msg}"
        );
        // The lock is released when it is dropped, and the file is left for
        // the next run to reuse rather than unlinked under a racing opener.
        let path = lock.path().to_path_buf();
        drop(lock);
        assert!(
            path.exists(),
            "the unlocked file is left in place, not removed"
        );
        // And the next run takes it.
        let lock = Lock::acquire(&dir).expect("the lock is free again");
        assert_eq!(lock.path(), path);
    }

    #[test]
    fn a_junk_body_names_an_unknown_holder() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lock", "body");
        let path = lock_path(&dir);
        std::fs::write(&path, "not a pid at all").unwrap();
        let msg = format!("{:#}", held(&path));
        assert!(msg.contains("pid ?"), "an unknown holder is named: {msg}");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn an_unreadable_body_says_so() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lock", "unreadable");
        let path = lock_path(&dir);
        let msg = format!("{:#}", held(&path));
        assert!(msg.contains("body is unreadable"), "{msg}");
    }

    #[test]
    fn different_game_dirs_do_not_conflict() {
        let one = girpr_tempdir::TempDir::new("girpr_test_lock", "dir_one");
        let two = girpr_tempdir::TempDir::new("girpr_test_lock", "dir_two");
        let first = Lock::acquire(&one).expect("the first install is locked");
        let second = Lock::acquire(&two).expect("another install is independent");
        assert_ne!(first.path(), second.path());
    }

    #[test]
    fn dropping_releases_the_lock_and_keeps_the_file() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lock", "release");
        let path = {
            let lock = Lock::acquire(&dir).expect("the lock is taken");
            lock.path().to_path_buf()
        };
        assert!(
            path.exists(),
            "the file is left behind so an opener cannot be unlinked"
        );
        let _again = Lock::acquire(&dir).expect("the OS released the lock");
    }

    #[test]
    fn a_file_left_by_a_dead_run_does_not_block() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lock", "dead");
        // A killed run leaves its body behind, but the OS has released the
        // handle: there is no mtime rule and no wall-clock wait.
        let path = lock_path(&dir);
        std::fs::write(&path, "pid=1\nstarted=2000-01-01T00:00:00Z\n").unwrap();
        let lock = Lock::acquire(&dir).expect("an unlocked leftover is harmless");
        assert_eq!(lock.path(), path);
    }

    #[test]
    fn lock_policy_covers_every_run_mode() {
        // A mutating run, including `--purge-only`: takes the lock.
        assert!(takes_lock(false, false, false));
        // `--dry-run` (readonly) and `--check-only`: no lock.
        assert!(!takes_lock(true, false, false));
        assert!(!takes_lock(false, true, false));
        assert!(!takes_lock(true, true, false));
        // `--print-plan`: no lock.
        assert!(!takes_lock(false, false, true));
    }

    #[cfg(windows)]
    #[test]
    fn the_key_folds_ascii_case_on_windows() {
        assert_eq!(
            key_for(Path::new(r"C:\Game")),
            key_for(Path::new(r"c:\game"))
        );
        assert_ne!(
            key_for(Path::new(r"C:\Game")),
            key_for(Path::new(r"D:\Game"))
        );
    }
}
