//! Utility primitives: the scan/MD5 primitive, server-controlled path
//! containment, and the launcher's config.ini files.
//!
//! - [`md5`] — one read, every answer: `scan_file`, `scan_reader`,
//!   `read_range`, `md5_file`, `md5_file_slice`, plus `Scan`, `Md5` and
//!   `EMPTY_MD5`.
//! - [`path`] — `contained_rel`, `clean_rel`, `contained_join`, `PathRoot`,
//!   `file_len`, `FileLen`: lexical + filesystem checks for server-controlled
//!   rel paths.
//! - [`ini`] — `read_game_version`, `read_ignore_categories`,
//!   `read_blacklist`, `write_config_ini`: the launcher's config.ini files.
//!
//! Every public name that lived in `crate::util` before this split still
//! resolves here; this module only moves code into focused files.

pub mod ini;
pub mod md5;
pub mod path;

pub use self::ini::{read_blacklist, read_game_version, read_ignore_categories, write_config_ini};
pub use self::md5::{
    EMPTY_MD5, Md5, Scan, ScanRange, md5_file, md5_file_slice, md5_reader_slice, read_range,
    scan_file, scan_reader,
};
pub use self::path::{FileLen, PathRoot, clean_rel, contained_join, contained_rel, file_len};
