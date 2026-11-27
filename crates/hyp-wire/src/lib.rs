//! Shared wire values for the HYP/Sophon JSON exchange.
//!
//! This leaf owns the pieces of the JSON API that are the same bytes in both
//! programs: the identity sent on every request, the envelope shape every
//! endpoint answers with, the number-or-string fields the API emits and the
//! retcode classification the repair client uses to stop retrying.
//!
//! It holds no HTTP client, retry loop, exit code or artifact format. Each
//! program keeps its own policy on top: girpr maps `-202` to `SophonNotFound`
//! and refuses a non-2xx before reading the body; gircap records the bytes
//! verbatim and turns a non-zero retcode into its own exit-2 shape error.
//! Neither policy crosses this boundary.

use serde::{Deserialize, Serialize};

/// The one `User-Agent` string sent on every request.
///
/// A generic Unity player string rather than one naming the tool: the API is a
/// third party, and a request should not identify the client that made it. It is
/// still a real header, so a captured `api/requests.ndjson` records exactly what
/// the CDN saw. Both producers send this same value; no published byte changes.
pub const USER_AGENT: &str = "UnityPlayer/2019.4.40f1 (UnityWebRequest/1.0, libcurl/7.80.0-DEV)";

/// The [`USER_AGENT`] string, owned where a [`String`] is wanted.
pub fn user_agent() -> String {
    USER_AGENT.to_string()
}

/// A number the API may send as an integer or as a decimal string.
///
/// Serialises back as whichever variant it was parsed from, so a typed value
/// round-trips to the same bytes the API sent. Callers choose the numeric view
/// they need with [`IntOrStr::as_u64`] / [`IntOrStr::as_i64`]; no coercion is
/// applied on the way in, because a capture must be able to reproduce the
/// original spelling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IntOrStr {
    /// An actual JSON number.
    Int(i64),
    /// A JSON string holding a decimal integer.
    Str(String),
}

impl IntOrStr {
    /// The value as `u64`, or `None` when it is negative or not a number.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            IntOrStr::Int(i) => u64::try_from(*i).ok(),
            IntOrStr::Str(s) => s.trim().parse().ok(),
        }
    }

    /// The value as `i64`, or `None` when it is not a number.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            IntOrStr::Int(i) => Some(*i),
            IntOrStr::Str(s) => s.trim().parse().ok(),
        }
    }
}

impl std::fmt::Display for IntOrStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntOrStr::Int(i) => write!(f, "{i}"),
            IntOrStr::Str(s) => f.write_str(s),
        }
    }
}

impl From<u64> for IntOrStr {
    fn from(v: u64) -> Self {
        match i64::try_from(v) {
            Ok(i) => IntOrStr::Int(i),
            Err(_) => IntOrStr::Str(v.to_string()),
        }
    }
}

impl From<i64> for IntOrStr {
    fn from(v: i64) -> Self {
        IntOrStr::Int(v)
    }
}

/// The API sends these as strings, and so does a caller writing a fixture — the
/// string form is the *faithful* one and gets no coercion on the way in.
impl From<&str> for IntOrStr {
    fn from(v: &str) -> Self {
        IntOrStr::Str(v.to_string())
    }
}

impl From<String> for IntOrStr {
    fn from(v: String) -> Self {
        IntOrStr::Str(v)
    }
}

/// A field decoder for a number that arrives as either a JSON number or a
/// decimal string, yielding the caller's chosen numeric type.
///
/// The target type configures the decoder: a field of type `i64` gets an `i64`,
/// a `u64` gets a `u64`, and an out-of-range or non-numeric spelling is refused
/// rather than wrapped. This is the coercing counterpart to [`IntOrStr`], which
/// preserves the wire spelling for a caller that needs to reproduce its bytes.
pub fn deserialize_number_from_string<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let number = IntOrStr::deserialize(deserializer)?;
    let text = match number {
        IntOrStr::Int(i) => i.to_string(),
        IntOrStr::Str(s) => s,
    };
    text.parse::<T>().map_err(serde::de::Error::custom)
}

/// `{"retcode":…,"message":…,"data":…}`, the envelope every endpoint returns.
///
/// The payload is a type parameter so both programs can share the shape without
/// agreeing on its contents: a caller that needs a required payload uses
/// `Envelope<T>`, and one that must accept a null payload uses
/// `Envelope<Option<T>>` (serde treats a missing `Option` field as `None`). What
/// a non-zero `retcode` *means* is deliberately left to the call site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope<T> {
    /// `0` on success. A non-zero value is interpreted by the caller.
    pub retcode: i64,
    /// `"OK"`, or the reason. Absent in some responses.
    #[serde(default)]
    pub message: Option<String>,
    /// The payload, required or optional by the caller's type parameter.
    pub data: T,
}

/// Just the envelope head, for reading `retcode` from a raw body whose payload
/// may not deserialize at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvelopeHead {
    /// The top-level `retcode`, as the API sent it.
    pub retcode: i64,
    /// `"OK"`, or the reason. Absent in some responses.
    #[serde(default)]
    pub message: Option<String>,
}

/// Whether a `retcode` means the *request* is wrong, so an identical retry will
/// be answered identically. This is girpr's classifier; gircap does not use it,
/// because it records every non-zero retcode as a shape error instead.
///
/// Measured against the live endpoints on 2026-10-16:
///
/// | retcode | request | what it means |
/// |---|---|---|
/// | `-202` | `getBuild?tag=<unknown>` | `not found` — surfaced as `SophonNotFound`, which the local-build fallback is built around |
/// | `-201` | `getBuild` with a bad `package_id` | `invalid params` |
/// | `-1001` | `getGameConfigs` / `getGameBranches` with a bad `game_ids[]` | `invalid params` |
///
/// Retrying any of these costs ~15 s of backoff to learn exactly what the first
/// response already said.
///
/// **What is deliberately not in this table: `retcode: 0` with an empty node.**
/// `getGameBranches` answers `{"retcode":0,…,"data":{"game_branches":[]}}` for a
/// game id it does not know. That is a success with nothing in it, not a
/// failure, and it is handled downstream by `game_branch`'s
/// `empty game_branches` — which is why this has to stay a retcode check and not
/// become a "did we get anything" check.
pub fn terminal_retcode(retcode: i64) -> bool {
    matches!(retcode, -201 | -202 | -1001)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identity_is_one_string_with_an_owned_form() {
        assert_eq!(user_agent(), USER_AGENT);
        assert!(USER_AGENT.starts_with("UnityPlayer/2019.4.40f1"));
    }

    // ------------------------------------------------ number decoder

    #[derive(Debug, Deserialize)]
    struct Sizes {
        #[serde(deserialize_with = "deserialize_number_from_string")]
        compressed: i64,
        #[serde(default, deserialize_with = "deserialize_number_from_string")]
        uncompressed: u64,
    }

    #[test]
    fn number_or_string_fields_keep_the_full_i64_range() {
        let a: Sizes = serde_json::from_str(r#"{"compressed":12,"uncompressed":"34"}"#).unwrap();
        assert_eq!((a.compressed, a.uncompressed), (12, 34));

        let max: Sizes = serde_json::from_str(
            r#"{"compressed":9223372036854775807,"uncompressed":"18446744073709551615"}"#,
        )
        .unwrap();
        assert_eq!(max.compressed, i64::MAX);
        assert_eq!(max.uncompressed, u64::MAX);
    }

    #[test]
    fn out_of_range_and_non_numeric_spellings_are_refused() {
        assert!(serde_json::from_str::<Sizes>(r#"{"compressed":9223372036854775808}"#).is_err());
        assert!(serde_json::from_str::<Sizes>(r#"{"compressed":"abc"}"#).is_err());
        assert!(serde_json::from_str::<Sizes>(r#"{"compressed":-1,"uncompressed":"-1"}"#).is_err());
    }

    #[test]
    fn a_negative_size_is_kept_for_the_caller_to_clamp() {
        let v: Sizes = serde_json::from_str(r#"{"compressed":-1}"#).unwrap();
        assert_eq!(v.compressed, -1);
        assert_eq!(v.uncompressed, 0);
    }

    // ------------------------------------------------ IntOrStr, the measured shape

    #[test]
    fn a_string_number_keeps_its_wire_form() {
        let v: IntOrStr = serde_json::from_str(r#""9007187""#).unwrap();
        assert_eq!(v, IntOrStr::Str("9007187".into()));
        assert_eq!(v.as_u64(), Some(9_007_187));
        assert_eq!(serde_json::to_string(&v).unwrap(), r#""9007187""#);
    }

    #[test]
    fn an_integer_number_keeps_its_wire_form() {
        let v: IntOrStr = serde_json::from_str("42").unwrap();
        assert_eq!(v, IntOrStr::Int(42));
        assert_eq!(v.as_i64(), Some(42));
        assert_eq!(serde_json::to_string(&v).unwrap(), "42");
    }

    #[test]
    fn a_u64_above_i64_max_is_kept_as_a_faithful_string() {
        let big = IntOrStr::from(u64::MAX);
        assert_eq!(big, IntOrStr::Str("18446744073709551615".into()));
        assert_eq!(big.as_u64(), Some(u64::MAX));
        assert_eq!(big.as_i64(), None);
    }

    #[test]
    fn neither_form_is_coerced_into_the_other_on_the_way_out() {
        assert_ne!(
            serde_json::to_string(&IntOrStr::Str("1".into())).unwrap(),
            serde_json::to_string(&IntOrStr::Int(1)).unwrap()
        );
    }

    #[test]
    fn a_negative_or_unparsable_value_has_no_unsigned_form() {
        assert_eq!(IntOrStr::Int(-1).as_u64(), None);
        assert_eq!(IntOrStr::Int(-1).as_i64(), Some(-1));
        assert_eq!(IntOrStr::Str("nope".into()).as_u64(), None);
        assert_eq!(IntOrStr::Str(" 42 ".into()).as_u64(), Some(42));
    }

    #[test]
    fn display_is_the_original_text_either_way() {
        assert_eq!(IntOrStr::Str("007".into()).to_string(), "007");
        assert_eq!(IntOrStr::Int(7).to_string(), "7");
    }

    // ------------------------------------------------ envelope + retcode

    #[test]
    fn the_envelope_shape_is_shared_with_an_optional_or_required_payload() {
        let body = r#"{"retcode":0,"message":"OK","data":{"n":1}}"#;
        let required: Envelope<serde_json::Value> = serde_json::from_str(body).unwrap();
        assert_eq!(required.retcode, 0);
        assert_eq!(required.message.as_deref(), Some("OK"));
        assert_eq!(required.data["n"], 1);

        let optional: Envelope<Option<serde_json::Value>> =
            serde_json::from_str(r#"{"retcode":0,"data":null}"#).unwrap();
        assert!(optional.data.is_none());
        assert_eq!(optional.message, None);
    }

    #[test]
    fn the_head_reads_a_retcode_from_a_body_whose_payload_cannot_parse() {
        let head: EnvelopeHead =
            serde_json::from_str(r#"{"data":"not an object","retcode":-202}"#).unwrap();
        assert_eq!(head.retcode, -202);
    }

    #[test]
    fn the_measured_terminal_retcodes_are_terminal_and_nothing_else_is() {
        for r in [-202, -201, -1001] {
            assert!(terminal_retcode(r), "{r} is invalid-params/not-found");
        }
        for r in [0, -1, -200, -2000] {
            assert!(!terminal_retcode(r), "{r} must not be treated as terminal");
        }
    }
}
