//! Test-side cleanup of the single-writer lock file a run leaves behind.
//!
//! A run deliberately keeps its unlocked lock file in the shared temp directory
//! so an opener cannot be unlinked; a test is not another run, and every case
//! uses a unique install, so the file would otherwise accumulate one per case.
//!
//! The module is shared by several test binaries through `#[path]`, so it must
//! not pull in the mock server or the fixture builders: a binary that only
//! needs the cleanup should not compile them.

// The module is reached from several test binaries and each uses a different
// part of it, so some helpers are dead code in any one of them.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use girpr::test_support::Lock;

/// The spelling whose canonical form names the run's lock key.
///
/// A run canonicalizes the install before locking, so a case that removes the
/// install mid-run leaves a key for a path that no longer exists. The lock is
/// keyed by the canonical install, so this reconstructs that spelling by
/// canonicalizing the longest surviving ancestor and re-joining the components
/// that vanished.
///
/// This handles a relative spelling and a junction above the install. It cannot
/// recover an install root that was itself a reparse point and was removed:
/// once the link is gone, its target is not reachable from the spelling. Tests
/// that model a vanished install use an ordinary directory, so the invariant is
/// that the removed part carries no reparse point of its own.
fn lock_key_path(game_dir: &Path) -> PathBuf {
    if game_dir.exists() {
        return game_dir.to_path_buf();
    }
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor = game_dir;
    loop {
        if cursor.as_os_str().is_empty() {
            // A relative path ends at the current directory, not at an empty
            // path that cannot be canonicalized.
            cursor = Path::new(".");
        }
        match cursor.canonicalize() {
            Ok(mut base) => {
                for part in missing.iter().rev() {
                    base.push(part);
                }
                return base;
            }
            Err(_) => {
                let Some(parent) = cursor.parent() else {
                    return game_dir.to_path_buf();
                };
                // A component such as `..` has no file name: only real
                // components are re-joined, and the parent's canonicalization
                // carries the rest.
                if let Some(name) = cursor.file_name() {
                    missing.push(name.to_os_string());
                }
                cursor = parent;
            }
        }
    }
}

/// Remove the unlocked lock file a run against `game_dir` leaves behind.
///
/// The path is keyed by the canonical game dir, and [`Lock::acquire`] is the
/// public way to learn it. Acquire after the run has returned: a lock the run
/// failed to release makes this fail, leaving the file for the reader to see.
///
/// Prefer [`remove_run_lock_for_args`] when the run's arguments are available:
/// a read-only run never took a lock, and acquiring to find the path would
/// briefly create the very file this removes.
pub fn remove_run_lock(game_dir: &Path) {
    if let Ok(lock) = Lock::acquire(&lock_key_path(game_dir)) {
        let path = lock.path().to_path_buf();
        drop(lock);
        let _ = std::fs::remove_file(path);
    }
}

/// Whether a run with these read-only flags takes the lock.
///
/// This mirrors the run's own rule: `--dry-run` and `--check-only` are the
/// read-only modes (`RunCtx::readonly`), and `--print-plan` returns before the
/// execution phase. `--purge-only` mutates and does take the lock.
pub fn mode_takes_lock(readonly: bool, check_only: bool, print_plan: bool) -> bool {
    !readonly && !check_only && !print_plan
}

/// Whether a binary argument list names a mode that takes the lock.
fn args_take_lock(args: &[impl AsRef<str>]) -> bool {
    !args
        .iter()
        .any(|arg| matches!(arg.as_ref(), "--dry-run" | "--check-only" | "--print-plan"))
}

/// The `--game-path` value in a binary argument list, if it has one.
fn game_path_arg(args: &[impl AsRef<str>]) -> Option<&str> {
    args.windows(2)
        .find_map(|pair| (pair[0].as_ref() == "--game-path").then(|| pair[1].as_ref()))
}

/// Remove the lock file a binary run leaves, from its own `--game-path`.
///
/// A read-only run (or a printed plan) never took a lock, so this returns
/// without acquiring: acquiring would create a lock file just to delete it.
pub fn remove_run_lock_for_args(args: &[impl AsRef<str>]) {
    if !args_take_lock(args) {
        return;
    }
    if let Some(game) = game_path_arg(args) {
        remove_run_lock(Path::new(game));
    }
}
