//! Request-URL grammar shared by the repair binary and the capture tool.
//!
//! This leaf owns the *mechanics* that must spell the same bytes in both
//! programs: percent-encoding a query value with the shared byte table,
//! appending `key=value` pairs with the recorded separator, trimming a base's
//! trailing slash, replacing one query value in an existing URL, and appending
//! path segments to a CDN base with `Url`.
//!
//! It chooses no endpoint, no base, no language, no channel tuple and no
//! prefix: those are program policy and stay at each call site. Keys are
//! appended and matched **literally** — `game_ids[]` keeps its brackets, which
//! is the spelling in recorded request URLs. The query functions deliberately
//! work on strings rather than a `Url` query serializer, which would escape
//! that key.

use girpr_model::query::QUERY_VALUE_SET;
use percent_encoding::utf8_percent_encode;

/// Percent-encode one query **value** with the recorded byte table.
///
/// Keys never pass through here: the bracketed `game_ids[]` key is appended
/// literally by [`push_pair`] and matched literally by the lookup/rewrite
/// helpers. Spaces become `%20`, never `+`; reserved bytes become uppercase
/// `%XX`.
pub fn encode_value(value: &str) -> String {
    utf8_percent_encode(value, QUERY_VALUE_SET).to_string()
}

/// Append `&key=encoded_value` to `url`, using `?` when it has no query yet.
///
/// The key is written literally; only the value is encoded.
pub fn push_pair(url: &mut String, key: &str, value: &str) {
    url.push(if url.contains('?') { '&' } else { '?' });
    url.push_str(key);
    url.push('=');
    url.push_str(&encode_value(value));
}

/// Build `base/path?k=v&...`, trimming trailing slashes off the base and
/// appending the pairs in the given order.
///
/// The order is load-bearing: recorded URLs diff the branch/package/password/
/// tag pairs byte for byte. A program decides *which* pairs to send; this leaf
/// only decides how they are joined.
pub fn build_query_url(base: &str, path: &str, pairs: &[(&str, &str)]) -> String {
    let mut url = format!("{}/{}", base.trim_end_matches('/'), path);
    for &(key, value) in pairs {
        push_pair(&mut url, key, value);
    }
    url
}

/// Replace every literal `key=` value in `url`'s query with an encoded `value`.
///
/// The key is matched and kept literally, brackets included. A URL with no
/// query is returned unchanged rather than gaining an empty query.
pub fn replace_query_value(url: &str, key: &str, value: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_owned();
    };
    let prefix = format!("{key}=");
    let encoded = encode_value(value);
    let parts: Vec<String> = query
        .split('&')
        .map(|part| match part.strip_prefix(&prefix) {
            Some(_) => format!("{prefix}{encoded}"),
            None => part.to_owned(),
        })
        .collect();
    format!("{base}?{}", parts.join("&"))
}

/// The raw value of the first literal `key=` pair in a bare query string.
///
/// No decoding: the value is returned exactly as it appeared, so a `%XX` escape
/// survives. Callers that need decoding use [`decoded_query_value`].
pub fn literal_query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    query.split('&').find_map(|part| part.strip_prefix(&prefix))
}

/// The form-decoded value of the first literal `key` in a bare query string.
///
/// The standard form parser handles `%XX`, `+` and lossy UTF-8. Keys are
/// compared after decoding, so a bracketed key spelled literally still matches.
pub fn decoded_query_value(query: &str, key: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find_map(|(k, v)| (k == key).then(|| v.into_owned()))
}

/// Why a CDN base could not accept appended path segments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendPathError {
    /// The base is not an absolute URL.
    Parse(url::ParseError),
    /// The base has an opaque path, which cannot carry segments.
    OpaquePath,
}

impl std::fmt::Display for AppendPathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "URL is not absolute: {e}"),
            Self::OpaquePath => f.write_str("URL has an opaque path"),
        }
    }
}

impl std::error::Error for AppendPathError {}

/// Append path segments to an absolute URL with `Url`'s path-segment API.
///
/// Each segment is split on `/` and empty parts are skipped, so a caller can
/// pass a backslash-folded path name or a leading `/` without producing a
/// double slash. A trailing empty segment on the base is popped first; `Url`
/// owns separators and escaping from there. The caller keeps its own error
/// type by mapping [`AppendPathError`] at the call site.
pub fn append_path_segments<'a>(
    base: &str,
    segments: impl IntoIterator<Item = &'a str>,
) -> Result<String, AppendPathError> {
    let mut url = url::Url::parse(base).map_err(AppendPathError::Parse)?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| AppendPathError::OpaquePath)?;
        path.pop_if_empty();
        for segment in segments {
            for part in segment.split('/') {
                if !part.is_empty() {
                    path.push(part);
                }
            }
        }
    }
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_values_keep_the_recorded_spelling() {
        assert_eq!(encode_value("a b/c+d?e"), "a%20b%2Fc%2Bd%3Fe");
        assert_eq!(encode_value("-._~"), "-._~");
        assert_eq!(encode_value("gopR6Cufr3"), "gopR6Cufr3");
    }

    #[test]
    fn push_pair_keeps_bracketed_keys_literal_and_picks_the_separator() {
        let mut url = String::from("https://api.test/getGameConfigs");
        push_pair(&mut url, "launcher_id", "VYTpXlbWo8");
        push_pair(&mut url, "game_ids[]", "gop R6");
        assert_eq!(
            url,
            "https://api.test/getGameConfigs?launcher_id=VYTpXlbWo8&game_ids[]=gop%20R6"
        );
    }

    #[test]
    fn build_query_url_trims_the_base_and_keeps_pair_order() {
        assert_eq!(
            build_query_url(
                "https://api.test/api/",
                "getBuild",
                &[
                    ("branch", "main"),
                    ("package_id", "pkg id"),
                    ("password", "p@ss/word"),
                    ("tag", "5.0.0"),
                ]
            ),
            "https://api.test/api/getBuild?branch=main&package_id=pkg%20id&password=p%40ss%2Fword&tag=5.0.0"
        );
    }

    #[test]
    fn replace_query_value_matches_the_literal_key_and_encodes() {
        let url =
            "https://api.test/getGameConfigs?launcher_id=VYTpXlbWo8&game_ids[]=one&game_ids[]=two";
        assert_eq!(
            replace_query_value(url, "game_ids[]", "a&b"),
            "https://api.test/getGameConfigs?launcher_id=VYTpXlbWo8&game_ids[]=a%26b&game_ids[]=a%26b"
        );
        assert_eq!(
            replace_query_value(url, "launcher", "nope"),
            url,
            "a key with no `=` match must leave the query alone"
        );
        assert_eq!(
            replace_query_value("https://api.test/y", "k", "v"),
            "https://api.test/y"
        );
    }

    #[test]
    fn lookups_agree_on_literal_keys_but_not_on_decoding() {
        let query = "game_ids[]=a%20b&tag=5.0.0";
        assert_eq!(literal_query_value(query, "game_ids[]"), Some("a%20b"));
        assert_eq!(
            decoded_query_value(query, "game_ids[]").as_deref(),
            Some("a b")
        );
        assert_eq!(literal_query_value(query, "tag"), Some("5.0.0"));
        assert_eq!(literal_query_value(query, "missing"), None);
        assert_eq!(decoded_query_value(query, "missing"), None);
    }

    #[test]
    fn path_segments_skip_empty_parts_and_a_trailing_slash() {
        assert_eq!(
            append_path_segments("https://cdn.test/base/", ["a/b", "", "/c"]).unwrap(),
            "https://cdn.test/base/a/b/c"
        );
        assert!(matches!(
            append_path_segments("not a url", ["a"]),
            Err(AppendPathError::Parse(_))
        ));
    }
}
