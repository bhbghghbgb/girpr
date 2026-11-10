//! Tests for the shared run-lock cleanup helpers.
//!
//! `tests/common/run_lock.rs` reaches the test binaries in two ways:
//! `tests/artifact_bridge.rs` and this file include it directly with `#[path]`,
//! while the other binaries that need it reach it through `tests/common`, whose
//! `mod.rs` declares it as a plain child module. Its tests live here rather
//! than in a `#[cfg(test)]` module in that file: such a module would run once
//! per including binary.

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "common/run_lock.rs"]
mod run_lock;

use clap::{CommandFactory, Parser};
use girpr::test_support::{Lock, lock_path, takes_lock};
use girpr::{Args, RunCtx};
use girpr_tempdir::TempDir;
use run_lock::{
    args_take_lock, game_path_arg, lock_key_path, mode_takes_lock, remove_run_lock,
    remove_run_lock_for_args,
};

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

/// clap's own reading of an argument list: the game path when the CLI accepts
/// the list, and `None` when it refuses it.
fn clap_game_path(args: &[&str]) -> Option<String> {
    let mut argv = vec!["girpr"];
    argv.extend_from_slice(args);
    Args::try_parse_from(argv)
        .ok()
        .map(|parsed| parsed.game_path.to_string_lossy().into_owned())
}

mod run_lock_tests {
    use super::*;

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
        assert_eq!(game_path_arg(&["--game-path"]), None);
        assert_eq!(game_path_arg(&["--other", "/games/a"]), None);
        // The CLI rejects an empty value, so cleanup has no path to target.
        assert_eq!(game_path_arg(&["--game-path="]), None);
        assert_eq!(game_path_arg(&["--game-path", ""]), None);
    }

    #[test]
    fn game_path_arg_refuses_a_repeated_flag() {
        // A repeated `--game-path` is a usage error: the run never starts and
        // takes no lock, so cleanup must not choose one of the two paths.
        for args in [
            vec!["--game-path", "/games/a", "--game-path=/games/b"],
            vec!["--game-path=/games/a", "--game-path", "/games/b"],
            vec!["--game-path", "/games/a", "--game-path", "/games/a"],
        ] {
            assert_eq!(game_path_arg(&args), None, "{args:?}");
        }
        assert!(
            Args::try_parse_from([
                "girpr",
                "--game-path",
                "/games/a",
                "--game-path",
                "/games/b",
                "--biz",
                "hk4e_global",
            ])
            .is_err()
        );
    }

    #[test]
    fn game_path_arg_refuses_a_trailing_or_flag_value() {
        // clap rejects a `--game-path` with no value or with another flag as
        // its value, so the run never starts and takes no lock. The same holds
        // when such an occurrence follows a valid one.
        for args in [
            vec!["--game-path"],
            vec!["--game-path", "--dry-run"],
            vec!["--game-path", "--biz", "hk4e_global"],
            vec!["--game-path", "/games/a", "--game-path"],
            vec!["--game-path", "/games/a", "--game-path", "--dry-run"],
            vec!["--game-path=/games/a", "--game-path"],
        ] {
            assert_eq!(game_path_arg(&args), None, "{args:?}");
        }
        // A lone `-` is a value, not a flag.
        assert_eq!(game_path_arg(&["--game-path", "-"]), Some("-"));
    }

    #[test]
    fn game_path_arg_skips_another_flags_value() {
        // The token after a value-taking flag is that flag's value, so a
        // `--game-path` there is not a game path. clap refuses such a spelling
        // (a value may not look like a flag), so the run never takes a lock.
        for args in [
            vec!["--include", "--game-path", "/games/a"],
            vec!["--include", "--game-path=/games/a"],
            vec!["--max-files", "--game-path", "/games/a"],
            vec!["--jobs", "--game-path", "/games/a"],
            vec!["--log-file", "--game-path", "/games/a"],
            vec!["--include", "--dry-run", "--game-path", "/games/a"],
        ] {
            assert_eq!(game_path_arg(&args), None, "{args:?}");
        }

        // The same flags with a value of their own leave the path visible.
        for args in [
            vec!["--include", "*.blk", "--game-path", "/games/a"],
            vec!["--include=*.blk", "--game-path", "/games/a"],
            vec!["--jobs", "4", "--game-path", "/games/a"],
            vec!["--io-threads", "4", "--game-path", "/games/a"],
            vec!["--include", "-", "--game-path", "/games/a"],
        ] {
            assert_eq!(game_path_arg(&args), Some("/games/a"), "{args:?}");
        }
    }

    #[test]
    fn every_value_flag_hides_a_following_game_path() {
        // Read the value-taking flags from the CLI itself, so a new flag cannot
        // be added without the cleanup learning that it owns the next token.
        let command = Args::command();
        let mut checked = 0;
        for arg in command.get_arguments() {
            if !arg.get_action().takes_values() {
                continue;
            }
            let long = arg
                .get_long()
                .expect("every value-taking flag has a long name");
            for name in std::iter::once(long).chain(arg.get_aliases().unwrap_or_default()) {
                checked += 1;
                let flag = format!("--{name}");
                assert_eq!(
                    game_path_arg(&[flag.as_str(), "--game-path", "/games/a"]),
                    None,
                    "{flag} takes a value"
                );
                // A second `--game-path` is a repeat rather than a visible
                // path, so only the other flags get this shape.
                if flag != "--game-path" {
                    assert_eq!(
                        game_path_arg(&[flag.as_str(), "value", "--game-path", "/games/a"]),
                        Some("/games/a"),
                        "{flag} takes a value"
                    );
                }
            }
        }
        assert!(checked >= 20, "the CLI exposes its value-taking flags");
    }

    #[test]
    fn game_path_mirror_matches_clap_option_arity() {
        // Each list is either accepted by the CLI, and then the mirror must
        // name the same path, or refused, and then the mirror must name none
        // because no run took a lock.
        for args in [
            vec!["--game-path", "/games/a", "--biz", "hk4e_global"],
            vec!["--game-path=/games/a", "--biz", "hk4e_global"],
            vec!["--game-path", "-", "--biz", "hk4e_global"],
            vec!["--game-path=-", "--biz", "hk4e_global"],
            vec!["--biz", "hk4e_global", "--game-path", "/games/a"],
            vec![
                "--audio",
                "en-us",
                "--audio",
                "ja-jp",
                "--game-path",
                "/games/a",
                "--biz",
                "hk4e_global",
            ],
            vec![
                "--include",
                "*.blk",
                "--exclude",
                "*_tmp",
                "--game-path",
                "/games/a",
                "--biz",
                "hk4e_global",
            ],
            vec![
                "--include=--game-path",
                "--game-path",
                "/games/a",
                "--biz",
                "hk4e_global",
            ],
            vec![
                "--include",
                "--game-path",
                "/games/a",
                "--biz",
                "hk4e_global",
            ],
            vec!["--include", "--game-path=/games/a", "--biz", "hk4e_global"],
            vec![
                "--max-files",
                "--game-path",
                "/games/a",
                "--biz",
                "hk4e_global",
            ],
            vec!["--game-path", "--include", "*.blk", "--biz", "hk4e_global"],
            vec![
                "--game-path",
                "/games/a",
                "--game-path",
                "/games/b",
                "--biz",
                "hk4e_global",
            ],
            vec!["--game-path", "", "--biz", "hk4e_global"],
            vec!["--game-path=", "--biz", "hk4e_global"],
            vec![
                "--include",
                "-",
                "--game-path",
                "/games/a",
                "--biz",
                "hk4e_global",
            ],
        ] {
            assert_eq!(
                game_path_arg(&args),
                clap_game_path(&args).as_deref(),
                "{args:?}"
            );
        }
    }

    #[test]
    fn the_cli_refuses_a_missing_or_flag_game_path_value() {
        for argv in [
            vec!["girpr", "--game-path"],
            vec!["girpr", "--game-path", "--dry-run", "--biz", "hk4e_global"],
            vec!["girpr", "--game-path", "--biz", "hk4e_global"],
            vec![
                "girpr",
                "--game-path",
                "/games/a",
                "--game-path",
                "--biz",
                "hk4e_global",
            ],
        ] {
            assert!(Args::try_parse_from(argv.clone()).is_err(), "{argv:?}");
        }
        // An explicit `=` value and a lone `-` are still values, and the
        // parsed path is exactly that one-character spelling.
        let explicit = Args::try_parse_from(["girpr", "--game-path=-", "--biz", "hk4e_global"])
            .expect("an explicit `=` value parses");
        assert_eq!(explicit.game_path.to_str(), Some("-"));
        let lone = Args::try_parse_from(["girpr", "--game-path", "-", "--biz", "hk4e_global"])
            .expect("a lone `-` is a value");
        assert_eq!(lone.game_path.to_str(), Some("-"));
    }

    #[test]
    fn the_cli_refuses_an_empty_game_path() {
        // This is why `lock_key_path` does not mirror production's key for
        // `""`: clap rejects the empty value before any run can take a lock.
        assert!(
            Args::try_parse_from(["girpr", "--game-path", "", "--biz", "hk4e_global"]).is_err()
        );
        assert!(Args::try_parse_from(["girpr", "--game-path=", "--biz", "hk4e_global"]).is_err());
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
