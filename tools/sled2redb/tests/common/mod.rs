//! Test fixtures shared with `girsync`, now owned by `girsync-test-support`.
//!
//! This used to be a 702-line verbatim copy of the same file in `girsync`.
//! The implementations now live once, in `girsync_test_support`; this module
//! only re-exports them so `convert.rs` keeps its existing `mod common;`,
//! `common::helper(..)` and `use common::{..}` spellings.

pub use girsync_test_support::*;
// `#[macro_export]` macros live at the defining crate's root. `convert.rs` uses
// neither, but re-exporting keeps the helper surface identical to what this
// module used to provide. The fixtures crate re-exports them from
// `girpr-tempdir`, where they are still defined.
#[allow(unused_imports)]
pub use girsync_test_support::{skip_if_case_insensitive, skip_if_no_link};
