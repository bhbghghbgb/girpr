//! Digests shared by test targets, both those that build fixtures through the
//! rest of `common` and fixture-free targets that include this module directly.

/// Lowercase hex MD5 of a byte slice.
///
/// The spelling comes from the same leaf crate the programs use, so a fixture
/// built here names exactly the digest the repair path compares against.
pub fn md5hex(data: &[u8]) -> String {
    girpr_digest::md5_hex(data)
}
