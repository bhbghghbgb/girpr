//! Test fixtures shared with `girsync`, now owned by `girpr-tempdir`.
//!
//! This used to be a 702-line verbatim copy of the same file in `girsync`.
//! The implementations now live once, in `girpr-tempdir::test_support`; this
//! module only re-exports them so `convert.rs` keeps its existing
//! `mod common;`, `common::helper(..)` and `use common::{..}` spellings.

pub use girpr_tempdir::test_support::*;
// `#[macro_export]` macros live at the defining crate's root, not inside the
// `test_support` module. `convert.rs` uses neither, but re-exporting keeps the
// helper surface identical to what this module used to provide.
#[allow(unused_imports)]
pub use girpr_tempdir::{skip_if_case_insensitive, skip_if_no_link};
