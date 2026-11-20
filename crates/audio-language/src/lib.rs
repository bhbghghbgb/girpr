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
//! validation. Those are each program's own policy and stay at the call site;
//! changing them can move bundle bytes or selection decisions. [`CaseFold`]
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
}
