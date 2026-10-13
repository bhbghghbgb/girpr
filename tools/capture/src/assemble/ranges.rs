//! Range algebra: offsets, lengths, and the gap/overlap checks.
//!
//! **M7.** Chunks are placed at absolute offsets in a target file, and a REUSE
//! reads them from an absolute offset in a *different* file, so a moved chunk is
//! two ranges that happen to have equal length. Representing a range by
//! `(start, end)` rather than `(start, length)` is what makes an overlap or a gap
//! a comparison instead of an arithmetic coincidence.