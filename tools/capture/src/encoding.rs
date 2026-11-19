//! Shared percent-encoding: the text escaping used on the stdout data plane
//! and the uppercase URL encoding used in query values.
//!
//! Both spellings come from the `percent-encoding` crate's byte table, so the
//! uppercase hex form cannot drift between them. They differ in what they
//! escape: `percent_encode` touches only the characters that would break `k=v`
//! tokenisation (and leaves non-ASCII text as typed), while `percent` escapes
//! every byte. `percent_decode` is their shared inverse.

use percent_encoding::{percent_decode_str, percent_encode_byte};

/// Characters the text form must encode, everything else stays as typed.
const TEXT_ESCAPES: &[char] = &[' ', '\t', '\n', '\r', '%'];

/// Percent-encode exactly the characters that would break `k=v` tokenisation.
///
/// The set is deliberately the minimum: everything else stays as typed, so the
/// common line reads as a human wrote it. Non-ASCII characters are copied
/// verbatim: the crate's `percent_encode` escapes every non-ASCII byte (an
/// `AsciiSet` cannot express bytes ≥ 0x80), which would rewrite UTF-8 names as
/// `%XX` triples and change the bytes the text form carries.
pub(crate) fn percent_encode(s: &str) -> String {
    if !s.contains(TEXT_ESCAPES) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if TEXT_ESCAPES.contains(&c) {
            // Every entry in TEXT_ESCAPES is ASCII, so the cast is exact and
            // the crate emits the same uppercase `%XX` as the URL form.
            out.push_str(percent_encode_byte(c as u8));
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
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

/// `%XX` for each byte, uppercase — the spelling `application/x-www-form-
/// urlencoded` requires.
///
/// Public because `report` percent-encodes the same way for the `SUMMARY`
/// line; two encoders that disagree would make a value unquotable.
pub fn percent(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for &b in bytes {
        out.push_str(percent_encode_byte(b));
    }
    out
}
