//! Shared Windows path-name mechanics, parameterised by the caller's policy.
//!
//! The repair binary, the capture tool and the sync tool ask the same low-level
//! questions about a name — is it a Windows device name, does it end in a dot
//! or a space, does it fold to the same key on a case-insensitive host, how is
//! a backslash spelled — but they do not ask them under the same rule. Which
//! rule is right is a separate question (and changing one can move bundle bytes
//! or pairing decisions), so this crate deliberately does **not** choose. It
//! owns only the mechanism; each call site passes its own policy as an explicit
//! argument. In particular this crate never reads `cfg!(windows)`: a caller
//! that wants a host-dependent answer says so in the value it passes.
//!
//! The device-name check splits off the extension, optionally trims, compares
//! case-insensitively against a caller-supplied set, and applies the
//! `COM1`–`COM9` / `LPT1`–`LPT9` suffix pattern. The trailing-dot/space check is
//! host-independent and needs no policy argument. The case-fold and
//! forward-slash checks take a boolean for the host's rule. None of the four
//! holds a program's policy.

/// Is one path component a reserved Windows device name under the supplied rule?
///
/// `reserved` is the caller's set of literal names (for example `CON`, `NUL`,
/// `CONIN$`). The `COM1`–`COM9` and `LPT1`–`LPT9` names are part of the
/// mechanism rather than the set, because both callers share that pattern.
/// `trim_trailing_dot_or_space` selects whether trailing `.` and ` ` are
/// removed from the stem before comparing — the one rule on which the two
/// callers differ.
///
/// Only the part before the first `.` is considered: `NUL.txt` is still `NUL`.
/// Comparison is ASCII-case-insensitive on both paths, matching how Windows
/// resolves these names. The check is allocation-free.
pub fn is_reserved_device_name(
    component: &str,
    reserved: &[&str],
    trim_trailing_dot_or_space: bool,
) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    let stem = if trim_trailing_dot_or_space {
        stem.trim_end_matches([' ', '.'])
    } else {
        stem
    };

    if reserved.iter().any(|name| stem.eq_ignore_ascii_case(name)) {
        return true;
    }

    // Compare bytes, so a multi-byte UTF-8 stem can never be sliced through a
    // character boundary. `COM0` / `LPT0` are not reserved, so only 1–9 count.
    let bytes = stem.as_bytes();
    bytes.len() == 4
        && (bytes[..3].eq_ignore_ascii_case(b"COM") || bytes[..3].eq_ignore_ascii_case(b"LPT"))
        && matches!(bytes[3], b'1'..=b'9')
}

/// Does one name end in the `.` or ` ` that Windows strips when resolving it?
///
/// The pure, host-independent mechanic: Windows aliases `config.` and
/// `config ` to `config`, so a spelling with either trailing character names
/// the same file as the stripped one. Both the repair and the capture
/// predicates apply this on every host — the install is a Windows tree even
/// when the tool runs elsewhere — so there is no policy to parameterise here.
/// Callers own the question of *which* names to run it over (the `Path`'s
/// components, or the components of a `/`-separated bundle path).
pub fn has_trailing_dot_or_space(name: &str) -> bool {
    name.ends_with('.') || name.ends_with(' ')
}

/// Fold ASCII case in `value` when `fold_case` is set, or copy it unchanged.
///
/// `fold_case` is the caller's explicit host policy: a case-sensitive host
/// leaves every byte as it is, a case-insensitive one folds ASCII letters and
/// compares every other byte exactly. The fold is ASCII-only on purpose —
/// `str::to_lowercase` is a Unicode mapping that merges spellings the
/// filesystem keeps apart (the Kelvin sign and `k`, for example) — so a
/// non-ASCII case variant is deliberately a distinct key.
///
/// This crate never asks `cfg!(windows)`. The repair binary passes whether it
/// is on Windows; the sync tool passes `true` because case-insensitive pairing
/// is its user's option, not the host's rule. The two `path_key` behaviours
/// stay separate; only this fold mechanic is shared.
pub fn fold_ascii_case(value: &str, fold_case: bool) -> String {
    if fold_case {
        value.to_ascii_lowercase()
    } else {
        value.to_owned()
    }
}

/// Spell `value` with `/` separators, rewriting `\` when asked.
///
/// `rewrite_backslash` is the caller's explicit host policy: on a host whose
/// separator is `\` the spelling must be rewritten, while on POSIX a `\` is a
/// literal filename character and rewriting it would turn `\lead.txt` into an
/// absolute path or `a\b.txt` into a different file. The caller passes the
/// answer; this crate never reads `cfg!(windows)`.
pub fn forward_slash(value: &str, rewrite_backslash: bool) -> String {
    if rewrite_backslash {
        value.replace('\\', "/")
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        fold_ascii_case, forward_slash, has_trailing_dot_or_space, is_reserved_device_name,
    };

    // girpr's rule: the full set, including CLOCK$, with trailing trim.
    const TRIMMING_SET: &[&str] = &["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "CLOCK$"];
    // gircap's rule: the set without CLOCK$, compared on the raw stem.
    const RAW_SET: &[&str] = &["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];

    #[test]
    fn trimming_rule_reserves_clock_and_strips_trailing_characters() {
        for name in [
            "NUL",
            "nul",
            "con.dat",
            "PRN.txt",
            "AUX",
            "CONIN$",
            "CONOUT$",
            "CLOCK$",
            "clock$.txt",
            "COM1",
            "com9.dat",
            "lpt1.tar.gz",
            "NUL ",
            "NUL.",
        ] {
            assert!(
                is_reserved_device_name(name, TRIMMING_SET, true),
                "{name:?} must be reserved by the trimming rule"
            );
        }
        for name in ["conifer.dat", "nullable.bin", "com10.dat", "lpt", "COM0"] {
            assert!(
                !is_reserved_device_name(name, TRIMMING_SET, true),
                "{name:?} must be ordinary under the trimming rule"
            );
        }
    }

    #[test]
    fn raw_rule_omits_clock_and_keeps_trailing_characters() {
        for name in [
            "CON",
            "con",
            "aux.txt",
            "Prn.log",
            "NUL.",
            "CONIN$",
            "conout$.txt",
            "COM1",
            "com9.bin",
            "LPT3",
        ] {
            assert!(
                is_reserved_device_name(name, RAW_SET, false),
                "{name:?} must be reserved by the raw rule"
            );
        }
        // CLOCK$ and a trailing space in the stem are ordinary under this rule.
        // (A trailing dot is already dropped by the extension split, so it is
        // the trailing *space* that separates the two callers' trim policies.)
        for name in [
            "CLOCK$",
            "clock$.txt",
            "NUL ",
            "CONSOLE",
            "COM0",
            "LPT10",
            ".con",
        ] {
            assert!(
                !is_reserved_device_name(name, RAW_SET, false),
                "{name:?} must be ordinary under the raw rule"
            );
        }
    }

    #[test]
    fn trailing_dot_or_space_is_a_pure_mechanic() {
        for name in ["x.", "x ", "x...", "x   ", ".", " "] {
            assert!(has_trailing_dot_or_space(name), "{name:?}");
        }
        for name in ["x", "x.y", "x y", ".hidden", ""] {
            assert!(!has_trailing_dot_or_space(name), "{name:?}");
        }
    }

    #[test]
    fn ascii_fold_follows_the_callers_policy_flag() {
        assert_eq!(fold_ascii_case("ReadMe.TXT", true), "readme.txt");
        assert_eq!(fold_ascii_case("ReadMe.TXT", false), "ReadMe.TXT");
        // The fold is ASCII-only on both settings: a Unicode case mapping would
        // merge the Kelvin sign with `k`, which the filesystem keeps apart.
        for fold in [true, false] {
            assert_eq!(
                fold_ascii_case("\u{212A}/\u{00C4}", fold),
                "\u{212A}/\u{00C4}"
            );
        }
    }

    #[test]
    fn forward_slash_follows_the_callers_policy_flag() {
        assert_eq!(forward_slash("dir\\sub\\f.txt", true), "dir/sub/f.txt");
        assert_eq!(forward_slash("dir\\sub\\f.txt", false), "dir\\sub\\f.txt");
        // A leading backslash becomes a root only when the caller asks for the
        // rewrite; on the other host it stays a literal character.
        assert_eq!(forward_slash("\\lead.txt", true), "/lead.txt");
        assert_eq!(forward_slash("\\lead.txt", false), "\\lead.txt");
    }
}
