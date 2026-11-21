//! Thin shim over `girpr-sync-test-support`.
//!
//! The option/cache/planner fixtures and the in-process CLI runner used to be
//! re-exported here with the runner glue inlined, which compiled that glue into
//! every girsync integration binary. The glue now lives in the support library
//! alongside the fixtures; this module keeps every call site's `mod common;`,
//! `common::helper(..)`, `use common::*` and `skip_if_*!` spelling unchanged.

pub use girpr_sync_test_support::*;
// `#[macro_export]` macros live at the defining crate's root, so re-export them
// explicitly as well. Most test binaries in this suite use neither, hence the
// allow. The fixtures crate re-exports them from `girpr-tempdir`, where they are
// still defined.
#[allow(unused_imports)]
pub use girpr_sync_test_support::{skip_if_case_insensitive, skip_if_no_link};
