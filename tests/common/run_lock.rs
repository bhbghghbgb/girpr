//! Test-side cleanup of the single-writer lock file a run leaves behind.
//!
//! A run deliberately keeps its unlocked lock file in the shared temp directory
//! so an opener cannot be unlinked; a test is not another run, and every case
//! uses a unique install, so the file would otherwise accumulate one per case.
//!
//! The module is shared by several test binaries through `#[path]`, so it must
//! not pull in the mock server or the fixture builders: a binary that only
//! needs the cleanup should not compile them.

use std::path::{Component, Path, PathBuf};

use girpr::test_support::{Lock, lock_path};

/// The spelling whose canonical form names the run's lock key.
///
/// A run canonicalizes the install before locking, so a case that removes the
/// install mid-run leaves a key for a path that no longer exists. The lock is
/// keyed by the canonical install, so this reconstructs that spelling by
/// canonicalizing the longest surviving ancestor and re-joining the components
/// that vanished. The suffix is replayed lexically, so `.` and `..` in a
/// vanished part normalize exactly as they did while the path existed; a
/// spelling such as `sub/../install` therefore keys like `install` on a
/// case-sensitive host too. The spelling of a genuinely vanished component
/// cannot be recovered, but on a case-sensitive host the run could only have
/// used the one on disk, so replaying the caller's spelling matches it.
///
/// This handles a relative spelling and a junction above the install. It cannot
/// recover an install root that was itself a reparse point and was removed:
/// once the link is gone, its target is not reachable from the spelling. Tests
/// that model a vanished install use an ordinary directory, so the invariant is
/// that the removed part carries no reparse point of its own.
pub(crate) fn lock_key_path(game_dir: &Path) -> PathBuf {
    if let Ok(canonical) = game_dir.canonicalize() {
        return canonical;
    }

    let components: Vec<Component<'_>> = game_dir.components().collect();
    for keep in (0..=components.len()).rev() {
        let mut prefix = PathBuf::new();
        for component in &components[..keep] {
            prefix.push(component.as_os_str());
        }
        if prefix.as_os_str().is_empty() {
            // A relative path is anchored at the current directory, which is
            // what `.` canonicalizes.
            prefix.push(".");
        }
        let Ok(mut base) = prefix.canonicalize() else {
            continue;
        };
        let missing = &components[keep..];
        // A root or drive prefix can only be at the start of the path. If one
        // is in the suffix, the empty-prefix case is not its anchor.
        if missing
            .iter()
            .any(|component| matches!(component, Component::RootDir | Component::Prefix(_)))
        {
            continue;
        }

        let mut parts: Vec<std::ffi::OsString> = Vec::new();
        for component in missing {
            match component {
                Component::Normal(name) => parts.push(name.to_os_string()),
                Component::CurDir => {}
                Component::ParentDir => {
                    if parts.pop().is_none() {
                        // The `..` climbs out of the survived prefix itself.
                        base.pop();
                    }
                }
                // Excluded by the check above.
                Component::RootDir | Component::Prefix(_) => {}
            }
        }
        for part in parts {
            base.push(part);
        }
        return base;
    }
    game_dir.to_path_buf()
}

/// Remove the unlocked lock file a run against `game_dir` leaves behind.
///
/// The path is keyed by the canonical game dir, and [`lock_path`] is the
/// public way to learn it without creating it. Acquire only after the run has
/// returned, and only when the file is already there: a run that failed before
/// execution never took the lock, and acquiring to find the path would briefly
/// create the very file this removes. A lock the run failed to release makes
/// the acquire fail, leaving the file for the reader to see.
///
/// Prefer [`remove_run_lock_for_args`] when the run's arguments are available:
/// a read-only run never took a lock, and acquiring to find the path would
/// briefly create the very file this removes.
pub fn remove_run_lock(game_dir: &Path) {
    let key_path = lock_key_path(game_dir);
    if !lock_path(&key_path).exists() {
        return;
    }
    if let Ok(lock) = Lock::acquire(&key_path) {
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
///
/// The bridge binary includes this module only for the argument cleanup, so
/// the mode mirror is dead there while its other callers use it.
#[allow(dead_code)]
pub fn mode_takes_lock(readonly: bool, check_only: bool, print_plan: bool) -> bool {
    !readonly && !check_only && !print_plan
}

/// Whether a binary argument list names a mode that takes the lock.
pub(crate) fn args_take_lock(args: &[impl AsRef<str>]) -> bool {
    !args
        .iter()
        .any(|arg| matches!(arg.as_ref(), "--dry-run" | "--check-only" | "--print-plan"))
}

/// The `--game-path` value in a binary argument list, in either spelling.
pub(crate) fn game_path_arg(args: &[impl AsRef<str>]) -> Option<&str> {
    let mut found = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let arg = arg.as_ref();
        if arg == "--game-path" {
            found = args.next().map(|value| value.as_ref());
        } else if let Some(value) = arg.strip_prefix("--game-path=") {
            found = Some(value);
        }
    }
    found
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
