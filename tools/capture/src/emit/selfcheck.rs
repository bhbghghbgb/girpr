//! The A1–A17 assertion log.
//!
//! **M8.** One row per assertion *instance*, not per assertion kind — 71 chunks
//! produce 71 A6 rows — and a failing row is never dropped. A capture that failed
//! an assertion leaves the failure in place to be read, because the useful
//! artifact in that case is the one that says exactly which byte disagreed.