//! The three retention sets, and how a chunk is assigned to them.
//!
//! **M6.** `patch_fetch` is `materialize_new` minus every chunk whose
//! `(uncompressed_md5, uncompressed_size)` appears in the old file — same-file
//! only, which is the simplification under measurement, and `reuse_pool.json`
//! is what bounds it.