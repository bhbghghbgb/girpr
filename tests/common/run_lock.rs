//! Test-side cleanup of the single-writer lock file a run leaves behind.
//!
//! A run deliberately keeps its unlocked lock file in the shared temp directory
//! so an opener cannot be unlinked; a test is not another run, and every case
//! uses a unique install, so the file would otherwise accumulate one per case.
//!
//! The module is shared by several test binaries, directly through `#[path]`
//! and through `tests/common`, so it must not pull in the mock server or the
//! fixture builders: a binary that only needs the cleanup should not compile
//! them.

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
    // An empty path is not reachable: the CLI rejects it ("a value is
    // required") and `game_path_arg` reports none, so production's fallback key
    // for `""` is deliberately not mirrored here.
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
///
/// Unlike [`game_path_arg`], this needs no option-arity mirror: a value may not
/// start with `-` (the CLI refuses it), and an inline `--flag=value` token is
/// not a bare mode flag, so a mode flag is never another flag's value.
pub(crate) fn args_take_lock(args: &[impl AsRef<str>]) -> bool {
    !args
        .iter()
        .any(|arg| matches!(arg.as_ref(), "--dry-run" | "--check-only" | "--print-plan"))
}

/// Long flags whose next token is their value rather than another flag.
///
/// `--io-threads` is the deprecated alias of `--jobs`. A value attached with
/// `=` carries its own token, and the CLI declares no short flags.
const VALUE_FLAGS: &[&str] = &[
    "--audio",
    "--biz",
    "--exclude",
    "--game-path",
    "--hyp-base",
    "--include",
    "--io-threads",
    "--jobs",
    "--log-file",
    "--log-file-level",
    "--log-level",
    "--max-delete-bytes",
    "--max-download-bytes",
    "--max-files",
    "--max-ram-bytes",
    "--max-verified-download-bytes",
    "--max-verified-write-bytes",
    "--max-write-bytes",
    "--output",
    "--plan-out",
    "--repair-mode",
    "--sophon-base",
];

/// The one `--game-path` value in a binary argument list, in either spelling.
///
/// `None` when the list names no game path, when the flag has no following
/// value, when the following token is another flag, when the value is empty, or
/// when the flag is repeated. The token after any other value-taking flag is
/// that flag's value, so `--include --game-path /x` names an `--include` value
/// and no game path. The CLI refuses each of those spellings before a run can
/// take a lock, so cleanup must not pick a directory that never ran.
pub(crate) fn game_path_arg(args: &[impl AsRef<str>]) -> Option<&str> {
    let mut found = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let arg = arg.as_ref();
        // `--flag=value` carries its value; only a bare flag consumes the next
        // token.
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (arg, None),
        };
        if name != "--game-path" && !VALUE_FLAGS.contains(&name) {
            continue;
        }
        let value = match inline {
            Some(value) => value,
            None => match args.next().map(|value| value.as_ref()) {
                // clap reads a token that starts with `-` as a flag rather
                // than the option's value; a lone `-` is still a value.
                Some(value) if value == "-" || !value.starts_with('-') => value,
                // A missing value or another flag is a usage error, so the run
                // never starts and takes no lock.
                _ => return None,
            },
        };
        if name != "--game-path" {
            continue;
        }
        if value.is_empty() || found.replace(value).is_some() {
            return None;
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
