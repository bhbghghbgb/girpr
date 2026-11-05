//! Utility primitives: the scan/MD5 primitive, server-controlled path
//! containment, atomic publishes, and the launcher's config.ini files.
//!
//! - [`md5`] — one read, every answer: `scan_file`, `scan_reader`,
//!   `read_range`, `md5_file`, `md5_file_slice`, plus `Scan`, `Md5` and
//!   `EMPTY_MD5`.
//! - [`path`] — `contained_rel`, `clean_rel`, `contained_join`, `PathRoot`,
//!   `file_len`, `FileLen`, `rel_from_base`, `rel_pair_from_base`, `join_rel`:
//!   lexical + filesystem checks and `/`-form rel spellings.
//! - [`atomic`] — `write_atomic`: temp-sibling-then-rename publishing for every
//!   artifact girpr writes.
//! - [`ini`] — `read_game_version`, `read_ignore_categories`,
//!   `read_blacklist`, `write_config_ini`: the launcher's config.ini files.
//!
//! The re-exports below keep the shared names at `crate::util`. The MD5 file
//! wrappers stayed in [`md5`] rather than moving to the root: `md5_file` and
//! `md5_file_slice` are reached as [`md5::md5_file`] and
//! [`md5::md5_file_slice`].

pub mod atomic;
pub mod ini;
pub mod md5;
pub mod path;

pub use self::ini::{read_blacklist, read_game_version, read_ignore_categories, write_config_ini};
pub use self::md5::{EMPTY_MD5, Md5, Scan, ScanRange, read_range, scan_file, scan_reader};
pub use self::path::{FileLen, PathRoot, clean_rel, contained_join, file_len};
