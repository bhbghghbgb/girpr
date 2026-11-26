//! Small helpers the cache layer needs but that do not belong to any cache
//! subsystem.
//!
//! These used to live in girsync's `util` and `hardlink` modules. They are kept
//! here, inside the cache crate, so the crate has no dependency on girsync's
//! lib: that is what lets an edit elsewhere in girsync leave the whole cache
//! (and its native dependency tree) cached. Each helper is the same rule it was
//! before the move — the timestamp format, the file identity and the ASCII
//! fold are unchanged.

use std::fs::File;
use std::io;
use std::path::Path;

/// The identity of a file, for comparing a cache name claim with the file that
/// is actually at that name.
///
/// The leaf owns the platform pair and the held handle; this is the same
/// `girpr-fs-identity` type girsync's `hardlink` module re-exports, so a cache
/// claim and a plan-time identity cannot become two different notions of "the
/// same file".
pub use girpr_fs_identity::Identity as FileId;

/// UTC stamp in the shared ISO format, used to name backups and snapshots.
///
/// The format itself lives in `girpr_time::compact_stamp`; this shim keeps call
/// sites short while leaving one definition of the stamp.
pub fn ts_now() -> String {
    girpr_time::compact_stamp_now()
}

/// The identity of the file at `path`, following links.
///
/// This is the same leaf call girsync's `hardlink::info` uses for its `id`, so a
/// cache name claim and a plan-time identity cannot disagree about what "the
/// same file" means. A missing path is an error the caller turns into "no name
/// to protect".
pub fn identity_at(path: &Path) -> io::Result<FileId> {
    girpr_fs_identity::identity_at(path)
}

/// The identity of the file a handle names.
///
/// A handle names the file it was opened on even after the name has gone, so
/// this is what lets a claim ask "is the thing at this path still the file this
/// handle was opened on?". The leaf duplicates the `File` for the identity,
/// leaving the caller's own handle usable.
pub fn identity_of(file: &File) -> io::Result<FileId> {
    girpr_fs_identity::identity_of(file)
}

/// One ASCII-case comparison, used by the hash vocabulary's own tests to
/// check that its fold agrees with the rest of the tool's path logic.
#[cfg(test)]
#[allow(clippy::manual_is_ascii_check)]
const fn ascii_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' { b + 32 } else { b }
}

#[cfg(test)]
pub(crate) const fn ascii_case_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if ascii_lower(a[i]) != ascii_lower(b[i]) {
            return false;
        }
        i += 1;
    }
    true
}
