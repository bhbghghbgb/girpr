//! Test-only helpers shared by the repair unit tests.
//!
//! A log line is part of the operator contract for the best-effort paths — a
//! refused audio move, a directory the sweep could not remove — so the tests
//! read it. `with_default` is thread-local, so a captured subscriber does not
//! race the other tests; nothing in the library installs a global subscriber
//! (only `main` does).
//!
//! The implementation lives once, in [`girpr_log::test_support`], as a public
//! library surface: nextest runs one process per test and skips doctests, so a
//! helper hidden in one test module cannot be reached by the other test
//! binaries. This module is the thin shim that keeps the existing
//! `crate::repair::test_support::logs_of` call sites unchanged.

pub(crate) use girpr_log::test_support::logs_of;
