//! Thin shim over the gircap test-support library.
//!
//! The mock, world builders and shared-bundle publisher used to be inlined here
//! by `mod common;` into every integration binary. They now live once in the
//! `gircap-test-support` library, reached through this shim so nextest's
//! one-process-per-test model compiles them once. The mock still constructs
//! every byte locally and borrows nothing from the repair binary (§2 of
//! `docs/00-spec.md`); it only depends on gircap's own public seams.

#![allow(dead_code, unused_imports)]

pub use gircap_test_support::*;
