//! Thin shim over the library's run-lock cleanup helpers.
//!
//! Every binary reaches it through `tests/common`, whose `mod.rs` declares it
//! as a child module. The implementation lives in `girpr_test_fixtures` so the
//! lock policy is written once even though several binaries need only the
//! cleanup.

// Each including binary uses a different subset of the cleanup helpers.
#[allow(unused_imports)]
pub use girpr_test_fixtures::{
    args_take_lock, game_path_arg, lock_key_path, mode_takes_lock, remove_run_lock,
    remove_run_lock_for_args,
};
