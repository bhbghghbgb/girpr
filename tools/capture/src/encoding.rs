//! Shared percent-encoding: the text escaping used on the stdout data plane
//! and the uppercase URL encoding used in query values.
//!
//! Both encoders agree on one spelling for non-ASCII text: UTF-8 bytes,
//! `%XX`-encoded, uppercase. `percent_encode` escapes only the characters that
//! would break `k=v` tokenisation; `percent` escapes every byte. They delegate
//! to a single hex renderer so they cannot drift.

use crate::hash::hex_bytes_upper;

/// Characters the text form must encode, everything else stays as typed.
const TEXT_ESCAPES: &[char] = &[' ', '\t', '\n', '\r', '%'];

/// Percent-encode exactly the bytes that would break `k=v` tokenisation.
///
/// The set is deliberately the minimum: everything else stays as typed, so the
/// common line reads as a human wrote it.
pub(crate) fn percent_encode(s: &str) -> String {
    if !s.contains(TEXT_ESCAPES) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 8);
    let mut buf = [0u8; 4];
    for c in s.chars() {
        if TEXT_ESCAPES.contains(&c) {
            // Non-ASCII is encoded too, as UTF-8 bytes, so the encoding is
            // reversible on any host and not just one with the right codepage.
            out.push_str(&percent(c.encode_utf8(&mut buf).as_bytes()));
        } else {
            out.push(c);
        }
    }
    out
}

/// The inverse of `percent_encode`. Exposed so a consumer — and a test — can
/// read a value back without re-implementing the rule.
pub fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &s[i + 1..i + 3];
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `%XX` for each byte, uppercase — the spelling `application/x-www-form-
/// urlencoded` requires.
///
/// Public because `report` percent-encodes the same way for the `SUMMARY`
/// line; two encoders that disagree would make a value unquotable.
pub fn percent(bytes: &[u8]) -> String {
    let hex = hex_bytes_upper(bytes);
    let mut out = String::with_capacity(bytes.len() * 3);
    for pair in hex.as_bytes().chunks(2) {
        out.push('%');
        out.push(pair[0] as char);
        out.push(pair[1] as char);
    }
    out
}
