//! The query-value byte table both programs send verbatim.
//!
//! HYP and Sophon query *values* are escaped with one exact byte table in the
//! repair binary and the capture tool. A space is `%20` (never `+`) and every
//! byte outside the RFC3986 unreserved set becomes an uppercase `%XX`; bracketed
//! keys such as `game_ids[]` are appended literally by each caller and never
//! pass through this table. Keeping the table in this value leaf means an edit
//! to one program's request builder cannot silently re-spell the other's URLs.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC};

/// The recorded query-value spelling: every byte except the RFC3986 unreserved
/// set becomes an uppercase `%XX`. `Url`'s form serializer would send spaces as
/// `+` and brackets as `%5B%5D`, so the table's exact set is kept here rather
/// than delegating to its default escaping.
pub const QUERY_VALUE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[cfg(test)]
mod tests {
    use super::*;
    use percent_encoding::utf8_percent_encode;

    #[test]
    fn query_values_keep_the_recorded_spelling() {
        // `%20`, never `+`; reserved bytes are uppercase `%XX`; the unreserved
        // set passes through. Bracketed keys stay literal at the callers.
        assert_eq!(
            utf8_percent_encode("a b/c+d?e", QUERY_VALUE_SET).to_string(),
            "a%20b%2Fc%2Bd%3Fe"
        );
        assert_eq!(
            utf8_percent_encode("-._~", QUERY_VALUE_SET).to_string(),
            "-._~"
        );
    }
}
