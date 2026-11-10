//! Digests shared by test targets, both those that build fixtures through the
//! rest of `common` and fixture-free targets that include this module directly.

/// Lowercase hex MD5 of a byte slice.
pub fn md5hex(data: &[u8]) -> String {
    use md5::Digest as _;
    md5::Md5::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
