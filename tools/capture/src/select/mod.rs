//! Scoring, variant classes, and budget admission.
//!
//! **M5.** The ranking key is `total_cost` — old bytes *plus* new bytes — and
//! not `patch_bytes`, because the old file must be materialized in full for the
//! fixture to be real. That choice is what puts a 1.83 MB DLL ahead of its own
//! 274 KB patch size, and it is the difference between a cheap artifact and a
//! dishonest one.