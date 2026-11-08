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
fn lock_key_path(game_dir: &Path) -> PathBuf {
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
pub fn mode_takes_lock(readonly: bool, check_only: bool, print_plan: bool) -> bool {
    !readonly && !check_only && !print_plan
}

/// Whether a binary argument list names a mode that takes the lock.
fn args_take_lock(args: &[impl AsRef<str>]) -> bool {
    !args
        .iter()
        .any(|arg| matches!(arg.as_ref(), "--dry-run" | "--check-only" | "--print-plan"))
}

/// The `--game-path` value in a binary argument list, in either spelling.
fn game_path_arg(args: &[impl AsRef<str>]) -> Option<&str> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let arg = arg.as_ref();
        if arg == "--game-path" {
            return args.next().map(|value| value.as_ref());
        }
        if let Some(value) = arg.strip_prefix("--game-path=") {
            return Some(value);
        }
    }
    None
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    use clap::Parser;
    use girpr::test_support::takes_lock;
    use girpr::{Args, RunCtx};
    use girpr_tempdir::TempDir;

    const MODE_FLAGS: [&str; 4] = ["--dry-run", "--check-only", "--print-plan", "--purge-only"];

    /// Every subset of the mode flags, so the tests pin every combination a
    /// run can select rather than a few hand-picked booleans.
    fn mode_sets() -> Vec<Vec<&'static str>> {
        (0..1 << MODE_FLAGS.len())
            .map(|mask| {
                MODE_FLAGS
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, flag)| *flag)
                    .collect()
            })
            .collect()
    }

    fn ctx_for(flags: &[&str]) -> RunCtx {
        let mut argv = vec!["girpr", "--game-path", ".", "--biz", "hk4e_global"];
        argv.extend_from_slice(flags);
        Args::parse_from(argv)
            .into_ctx()
            .expect("test flags must parse")
    }

    #[test]
    fn mode_mirror_matches_every_run_mode() {
        for flags in mode_sets() {
            let ctx = ctx_for(&flags);
            assert_eq!(
                mode_takes_lock(ctx.readonly(), ctx.check_only, ctx.print_plan),
                takes_lock(ctx.readonly(), ctx.check_only, ctx.print_plan),
                "flags {flags:?}"
            );
        }

        // `--purge-only` is the mutating inverse of check-only: it is not one
        // of the read-only modes, so it must take the lock.
        let purge_only = ctx_for(&["--purge-only"]);
        assert!(purge_only.purge_only);
        assert!(!purge_only.readonly());
        assert!(takes_lock(
            purge_only.readonly(),
            purge_only.check_only,
            purge_only.print_plan
        ));
    }

    #[test]
    fn args_mirror_accepts_purge_only_and_rejects_read_only_flags() {
        for flags in mode_sets() {
            let ctx = ctx_for(&flags);
            let mut argv = vec!["--game-path", "."];
            argv.extend_from_slice(&flags);
            assert_eq!(
                args_take_lock(&argv),
                takes_lock(ctx.readonly(), ctx.check_only, ctx.print_plan),
                "flags {flags:?}"
            );
        }

        for flag in ["--dry-run", "--check-only", "--print-plan"] {
            assert!(
                !args_take_lock(&["--game-path", ".", flag]),
                "{flag} is read-only"
            );
        }
        assert!(args_take_lock(&["--game-path", ".", "--purge-only"]));
    }

    #[test]
    fn game_path_is_read_in_both_spellings() {
        assert_eq!(
            game_path_arg(&["--game-path", "/games/a"]),
            Some("/games/a")
        );
        assert_eq!(game_path_arg(&["--game-path=/games/a"]), Some("/games/a"));
        assert_eq!(
            game_path_arg(&["--dry-run", "--game-path", "/games/a"]),
            Some("/games/a")
        );
        // A value may itself contain `=`.
        assert_eq!(
            game_path_arg(&["--game-path=/games/a=b"]),
            Some("/games/a=b")
        );
        // The first spelling wins, exactly as clap's last-wins rule would not
        // be relevant to a cleanup that only needs one path.
        assert_eq!(
            game_path_arg(&["--game-path", "/games/a", "--game-path=/games/b"]),
            Some("/games/a")
        );
        assert_eq!(game_path_arg(&["--game-path"]), None);
        assert_eq!(game_path_arg(&["--other", "/games/a"]), None);
    }

    #[test]
    fn a_vanished_install_keys_by_its_canonical_parent() {
        let dir = TempDir::new("girpr_it", "run_lock_vanished");
        let install = dir.join("install");
        let expected = dir.canonicalize().unwrap().join("install");
        assert_eq!(lock_key_path(&install), expected);
    }

    #[test]
    fn a_vanished_suffix_is_replayed_lexically() {
        let dir = TempDir::new("girpr_it", "run_lock_dotdot");
        std::fs::create_dir(dir.join("sub")).unwrap();

        let through_sub = dir.join("sub").join("..").join("install");
        let through_current = dir.join(".").join("install");
        let expected = dir.canonicalize().unwrap().join("install");
        assert_eq!(lock_key_path(&through_sub), expected);
        assert_eq!(lock_key_path(&through_current), expected);
    }

    #[test]
    fn cleanup_does_not_create_a_lock_for_an_absent_run() {
        let dir = TempDir::new("girpr_it", "run_lock_absent");
        let game = dir.join("never-installed");
        let path = lock_path(&lock_key_path(&game));
        assert!(!path.exists(), "the setup has no lock file");

        remove_run_lock(&game);
        assert!(
            !path.exists(),
            "cleanup must not create the file it did not find"
        );
    }

    #[test]
    fn cleanup_removes_a_lock_the_run_left() {
        let dir = TempDir::new("girpr_it", "run_lock_present");
        let game = dir.to_path_buf();
        let lock = Lock::acquire(&game).expect("the lock is taken");
        let path = lock.path().to_path_buf();
        drop(lock);
        assert!(path.exists(), "the run left the lock file");

        remove_run_lock(&game);
        assert!(!path.exists(), "cleanup removes the run's lock file");
    }

    #[test]
    fn args_cleanup_removes_the_lock_in_both_spellings() {
        let dir = TempDir::new("girpr_it", "run_lock_args");
        let game = dir.to_string_lossy().into_owned();
        for args in [
            vec!["--game-path".to_string(), game.clone()],
            vec![format!("--game-path={game}")],
        ] {
            let lock = Lock::acquire(dir.as_ref()).expect("the lock is taken");
            let path = lock.path().to_path_buf();
            drop(lock);
            assert!(path.exists(), "the run left the lock file");

            remove_run_lock_for_args(&args);
            assert!(!path.exists(), "cleanup removes the lock for {args:?}");
        }
    }
}
