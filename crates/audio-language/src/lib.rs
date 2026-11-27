//! The shared audio-language mechanics, parameterised by each caller's policy.
//!
//! The repair binary and the capture tool start from the same two things: a
//! user-supplied language token that must be lower-cased with `_` accepted for
//! `-`, and the four Sophon `matching_field` codes with the display names the
//! game writes for them, in the game's canonical order. Those are mechanics and
//! a constant table, not program behaviour, so they live here.
//!
//! What is deliberately *not* here: the alias map that turns a display name
//! into a code, the `none` game-only sentinel, and capture's path-component
//! validation. This leaf owns no alias map: the reverse lookup takes the
//! caller's aliases as an explicit parameter, so those remain each program's
//! own policy at the call site; changing them can move bundle bytes or
//! selection decisions. [`CaseFold`]
//! lets each caller keep its exact lower-casing flavour, and trimming stays
//! with the caller that does it, so neither tool's byte contract changes.

/// Which lower-casing flavour a caller uses before `_` becomes `-`.
///
/// The repair binary folds Unicode (its historical `str::to_lowercase`); the
/// capture tool folds ASCII only. The difference is unobservable for the four
/// measured codes, but it is passed in explicitly rather than assumed so each
/// caller keeps its exact contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseFold {
    Unicode,
    Ascii,
}

/// Lower-case `token` and rewrite `_` to `-` — the normalisation shared by
/// both audio-language call sites.
///
/// Trimming is the caller's business: a caller that accepts surrounding
/// whitespace trims before calling; the other passes its token unchanged. The
/// returned string keeps every other byte, so an alias map, a sentinel check
/// or a path-component validation at the call site sees precisely what it saw
/// before this primitive was shared.
pub fn normalize_audio_token(token: &str, fold: CaseFold) -> String {
    let lowered = match fold {
        CaseFold::Unicode => token.to_lowercase(),
        CaseFold::Ascii => token.to_ascii_lowercase(),
    };
    lowered.replace('_', "-")
}

/// The four Sophon `matching_field` codes and the display names the game
/// writes, in the game's canonical (not alphabetical) order.
///
/// Shared verbatim by the repair binary's scan-file writer and the capture
/// tool's fixture emitter: two tools that write the same selection must write
/// the same bytes. This is a constant table only — no flag, no error message
/// and no validation policy crosses with it.
pub const AUDIO_LANGUAGES: [(&str, &str); 4] = [
    ("zh-cn", "Chinese"),
    ("en-us", "English(US)"),
    ("ja-jp", "Japanese"),
    ("ko-kr", "Korean"),
];

/// The display name the game writes for a canonical `code`, if there is one.
pub fn display_name(code: &str) -> Option<&'static str> {
    AUDIO_LANGUAGES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, name)| *name)
}

/// The canonical-order position of a display `name`, if it is one of the four.
///
/// A caller uses this to render a selection in the game's order regardless of
/// the order the codes arrived in. An unknown name has no position and the
/// caller sorts it last, as before.
pub fn display_name_position(name: &str) -> Option<usize> {
    AUDIO_LANGUAGES.iter().position(|(_, n)| *n == name)
}

/// The canonical code whose display name is exactly `name`, if there is one.
pub fn code_from_display_name(name: &str) -> Option<&'static str> {
    AUDIO_LANGUAGES
        .iter()
        .find(|(_, n)| *n == name)
        .map(|(code, _)| *code)
}

/// Reverse lookup using the caller's own alias spellings.
///
/// Canonical names are matched exactly first; an alias is matched ASCII
/// case-insensitively. The leaf owns **no** alias map — `aliases` is supplied
/// by the caller, because which spellings are accepted is program policy that
/// can change what a selection means. The repair scan-file reader passes its
/// `English(US)` / `English` aliases here; a caller that accepts only the four
/// canonical names passes an empty iterator.
pub fn code_from_display_name_with_aliases<'a>(
    name: &str,
    aliases: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Option<&'a str> {
    if let Some(code) = code_from_display_name(name) {
        return Some(code);
    }
    aliases.into_iter().find_map(|(spelling, code)| {
        if name.eq_ignore_ascii_case(spelling) {
            Some(code)
        } else {
            None
        }
    })
}

/// Render selected codes as display names, one per line, in the game's
/// canonical order.
///
/// This is the shared canonical projection and join: duplicates are written
/// once and codes that have no display name here are ignored, because whether
/// an unknown code is an error is the caller's validation policy.
///
/// `trailing_newline` is the caller's byte contract and is deliberately a
/// parameter, not a convergence: the capture fixture ends **every** line with
/// `\n`, while the repair binary's scan file writes no final newline. Both
/// spellings are produced by this one implementation.
pub fn render_scan_file<'a>(
    codes: impl IntoIterator<Item = &'a str>,
    trailing_newline: bool,
) -> String {
    let mut selected: Vec<&str> = codes.into_iter().filter_map(display_name).collect();
    selected.sort_by_key(|name| display_name_position(name).unwrap_or(usize::MAX));
    selected.dedup();
    let mut out = selected.join("\n");
    if trailing_newline && !out.is_empty() {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_normalisation_preserves_each_callers_flavour() {
        assert_eq!(normalize_audio_token("EN_US", CaseFold::Unicode), "en-us");
        assert_eq!(normalize_audio_token("EN_US", CaseFold::Ascii), "en-us");
        // The folds can differ outside the measured set; the caller's choice wins.
        assert_eq!(normalize_audio_token("İ", CaseFold::Unicode), "i\u{307}");
        assert_eq!(normalize_audio_token("İ", CaseFold::Ascii), "İ");
    }

    #[test]
    fn the_table_is_in_the_games_canonical_order() {
        assert_eq!(
            AUDIO_LANGUAGES.map(|(_, name)| name),
            ["Chinese", "English(US)", "Japanese", "Korean"]
        );
        assert_eq!(display_name("zh-cn"), Some("Chinese"));
        assert_eq!(display_name("fr-fr"), None);
        assert_eq!(display_name_position("Korean"), Some(3));
        assert_eq!(display_name_position("nope"), None);
    }

    #[test]
    fn reverse_lookup_takes_the_callers_aliases_explicitly() {
        assert_eq!(code_from_display_name("Chinese"), Some("zh-cn"));
        assert_eq!(code_from_display_name("chinese"), None);
        assert_eq!(
            code_from_display_name_with_aliases("Chinese", []),
            Some("zh-cn")
        );
        assert_eq!(
            code_from_display_name_with_aliases("chinese", []),
            None,
            "without an alias the canonical name is exact"
        );
        let aliases = [("English(US)", "en-us"), ("English", "en-us")];
        assert_eq!(
            code_from_display_name_with_aliases("ENGLISH", aliases),
            Some("en-us")
        );
        assert_eq!(
            code_from_display_name_with_aliases("english(us)", aliases),
            Some("en-us")
        );
        assert_eq!(
            code_from_display_name_with_aliases("Klingon", aliases),
            None
        );
    }

    #[test]
    fn render_scan_file_joins_in_canonical_order_with_the_callers_newline() {
        let codes = ["ko-kr", "zh-cn", "en-us", "ko-kr", "fr-fr"];
        assert_eq!(
            render_scan_file(codes, false),
            "Chinese\nEnglish(US)\nKorean",
            "the repair scan file has no trailing newline"
        );
        assert_eq!(
            render_scan_file(codes, true),
            "Chinese\nEnglish(US)\nKorean\n",
            "the capture fixture ends every line with a newline"
        );
        let empty: [&str; 0] = [];
        assert_eq!(render_scan_file(empty, true), "");
    }
}
