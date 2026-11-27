//! Hash primitives: the one-read file scan and its MD5 helpers.
//!
//! - [`md5`] — one read, every answer: `scan_file`, `scan_reader`,
//!   `read_range`, `md5_file`, `md5_file_slice`, plus `Scan`, `Md5` and
//!   `EMPTY_MD5`;
//! - `read_up_to` / `reserve_exact` — the reserved-capacity bounded read the
//!   file writers compose for their capture staging.
//!
//! The re-exports keep the shared names at the crate root. The MD5 file
//! wrappers stayed in [`md5`] rather than moving to the root: `md5_file` and
//! `md5_file_slice` are reached as [`md5::md5_file`] and
//! `md5::md5_file_slice` (the latter is a test-only helper).
//!
//! Nothing here names a product data model: the crate holds only mechanics.

pub mod md5;

pub use self::md5::{
    EMPTY_MD5, Md5, Scan, ScanRange, read_range, read_up_to, reserve_exact, scan_file, scan_reader,
};
