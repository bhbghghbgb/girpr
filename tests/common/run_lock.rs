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
/// install mid-run leaves a key for a path that no longer exists. The parent is
/// still there, and canonicalizing it and re-joining the last component gives
/// the spelling the run used.
fn lock_key_path(game_dir: &Path) -> PathBuf {
    if game_dir.exists() {
        return game_dir.to_path_buf();
    }
    let (Some(parent), Some(name)) = (game_dir.parent(), game_dir.file_name()) else {
        return game_dir.to_path_buf();
    };
    match parent.canonicalize() {
        Ok(parent) => parent.join(name),
        Err(_) => game_dir.to_path_buf(),
    }
}

/// Remove the unlocked lock file a run against `game_dir` leaves behind.
///
/// The path is keyed by the canonical game dir, and [`Lock::acquire`] is the
/// public way to learn it. Acquire after the run has returned: a lock the run
/// failed to release makes this fail, leaving the file for the reader to see.
pub fn remove_run_lock(game_dir: &Path) {
    if let Ok(lock) = Lock::acquire(&lock_key_path(game_dir)) {
        let path = lock.path().to_path_buf();
        drop(lock);
        let _ = std::fs::remove_file(path);
    }
}

/// The `--game-path` value in a binary argument list, if it has one.
fn game_path_arg(args: &[impl AsRef<str>]) -> Option<&str> {
    args.windows(2)
        .find_map(|pair| (pair[0].as_ref() == "--game-path").then(|| pair[1].as_ref()))
}

/// Remove the lock file a binary run leaves, from its own `--game-path`.
pub fn remove_run_lock_for_args(args: &[impl AsRef<str>]) {
    if let Some(game) = game_path_arg(args) {
        remove_run_lock(Path::new(game));
    }
}
