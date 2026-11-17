//! Test fixtures shared with `sled2redb`, now owned by `girpr-tempdir`.
//!
//! This used to be a 702-line verbatim copy of the same file in `sled2redb`.
//! The implementations now live once, in `girpr-tempdir::test_support`; this
//! module only re-exports them so every call site in this suite keeps its
//! existing `mod common;`, `common::helper(..)`, `use common::*`, and unqualified
//! `skip_if_*!` spellings.

pub use girpr_tempdir::test_support::*;
// `#[macro_export]` macros live at the defining crate's root, not inside the
// `test_support` module, so re-export them explicitly as well. Most test
// binaries in this suite use neither, hence the allow.
#[allow(unused_imports)]
pub use girpr_tempdir::{skip_if_case_insensitive, skip_if_no_link};
