//! The acyclic production leaves the girsync mirror is built on.
//!
//! Each module here is a leaf of girsync's dependency graph: it needs only the
//! shared crates below it and no girsync module above it. Keeping them in one
//! upstream crate lets girsync depend on them directly, so an edit to a
//! planner, command or other high-level module leaves all of this — and the
//! native dependencies behind the rw counters — cached instead of recompiling
//! them. The crate is girsync-only: no program's policy is shared through it.
//!
//! Nothing here changes a policy. The files moved verbatim from girsync's lib:
//! - [`rw`] — the read/write limiter and the worker pool. The gate's src-then-dst
//!   ordering is the deadlock-freedom invariant and stays exactly as it was.
//! - [`filter`] — girsync's include/exclude rules on top of `girpr-filter`'s
//!   one pinned glob version. The subtree rule is unchanged.
//! - [`hardlink`] — the copy guard's link count and handle-backed identity,
//!   read from `girpr-fs-identity`.
//! - [`link`] — what counts as a link and how to remove one without touching
//!   its target. The platform classification is unchanged.
//!
//! The tests that travelled with the files still live beside the code they
//! exercise. They use `unwrap`/`expect` deliberately: a panic in a dev tool is
//! a failed developer run, which is the lint policy girsync itself uses.
#![allow(clippy::unwrap_used, clippy::expect_used)]

pub mod filter;
pub mod hardlink;
pub mod link;
pub mod rw;
