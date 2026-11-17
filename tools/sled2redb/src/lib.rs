//! One-shot `sled` -> `redb` cache converter, extracted from `girsync`.
//!
//! The converter used to live in `girsync` behind a `convert` feature, which
//! put `sled` in girsync's dependency graph. It is now its own tool crate so
//! that the `girsync` package and binary stay completely free of `sled`.
//!
//! The conversion itself is unchanged. It reads a legacy `<root>/girpr-cache`
//! sled directory (JSON values, hex digests under the `\0meta` reservation)
//! and writes a new redb file through girsync's **public** cache API; hex
//! digests are decoded to raw bytes and unknown algorithms pass through
//! untouched. The only thing that moved is the crate boundary: `crate::cache`,
//! `crate::hash` and `crate::util` paths became `girsync::…` paths.

pub mod convert;
