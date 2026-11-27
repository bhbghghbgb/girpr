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
//! `COM1`–`COM9` / `LPT1`–`LPT9` suffix pattern. The two sets the programs use
//! are named here ([`DEVICE_NAMES_TRIMMING`] and [`DEVICE_NAMES_RAW`]) so the
//! copies do not drift; which one to use, and whether to trim, stays at the
//! call site. The trailing-dot/space check is host-independent and needs no
//! policy argument. The case-fold and separator checks take a boolean for the
//! caller's rule. None of them holds a program's policy.
//!
//! In addition to those name predicates, [`canonical_no_verbatim`] is the one
//! canonical-path spelling shared by the tools: it resolves links but strips
//! the Windows `\\?\` prefix so a canonical path compares directly with a
//! link's own target. A caller that needs the fail-closed longest-existing-
//! prefix variant (the repair binary does) keeps its own routine — that is a
//! different contract, not this helper's duplicate.

/// The canonical form of `path`, without the Windows `\\?\` verbatim prefix.
///
/// `std::fs::canonicalize` resolves links but, on Windows, returns a verbatim
/// path (`\\?\C:\...`). A junction's own `read_link` target has no such prefix,
/// so a verbatim canonical path cannot be compared component-wise with it.
/// This helper uses `dunce::canonicalize`, which resolves links identically
/// but omits the prefix, giving every caller one comparable spelling.
///
/// The `io::Result` is returned unchanged: a caller that treats `NotFound`
/// specially or reports its own context does so here, before any fallback.
pub fn canonical_no_verbatim(path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    dunce::canonicalize(path)
}

/// girpr's reserved set: the full list, including `CLOCK$`.
///
/// Paired with `trim_trailing_dot_or_space = true` by the repair binary and
/// its path-shape validator. The set is only data; the trim decision is
/// passed separately to [`is_reserved_device_name`].
pub const DEVICE_NAMES_TRIMMING: &[&str] =
    &["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "CLOCK$"];

/// gircap's reserved set: the list without `CLOCK$`.
///
/// Paired with `trim_trailing_dot_or_space = false` by the capture tool and
/// its strict path-shape validator. `CLOCK$` and a trailing space in the stem
/// are ordinary names under that rule.
pub const DEVICE_NAMES_RAW: &[&str] = &["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];

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

/// Spell `value` with the host's separator, rewriting `/` when asked.
///
/// The inverse of [`forward_slash`]: a `/`-separated rel is turned into a
/// path the host will open when `rewrite_forward_slash` is set. On a host
/// whose separator is `/` the rewrite is a no-op copy. The caller decides
/// whether the spelling needs rewriting; this crate never reads `cfg!`.
pub fn native_separator(value: &str, rewrite_forward_slash: bool) -> String {
    if rewrite_forward_slash {
        value.replace('/', std::path::MAIN_SEPARATOR_STR)
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEVICE_NAMES_RAW, DEVICE_NAMES_TRIMMING, canonical_no_verbatim, fold_ascii_case,
        forward_slash, has_trailing_dot_or_space, is_reserved_device_name, native_separator,
    };

    #[test]
    fn canonical_has_no_verbatim_prefix() {
        // A directory that exists on every host; canonicalization must resolve
        // it to an absolute path and, on Windows, drop the `\\?\` prefix.
        let resolved = canonical_no_verbatim(&std::env::temp_dir()).expect("temp dir resolves");
        assert!(resolved.is_absolute(), "{}", resolved.display());
        assert!(resolved.is_dir(), "{}", resolved.display());
        #[cfg(windows)]
        assert!(
            !resolved.to_string_lossy().starts_with(r"\\?\"),
            "{}",
            resolved.display()
        );
        // The helper is the one spelling: it agrees with dunce, which is what
        // it wraps, rather than with std's verbatim form.
        assert_eq!(
            resolved,
            dunce::canonicalize(std::env::temp_dir()).expect("temp dir resolves")
        );
    }

    /// The five pure policy cases, folded into one process so nextest spawns one
    /// test instead of five. Every original assertion and message is kept; a
    /// labelled subcase names the policy that failed. The canonical-path case
    /// stays separate because it resolves a host filesystem path.
    #[test]
    fn pure_policy_contracts() {
        let subcases: &[(&str, &dyn Fn())] = &[
            (
                "trimming_rule_reserves_clock_and_strips_trailing_characters",
                &|| {
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
                            is_reserved_device_name(name, DEVICE_NAMES_TRIMMING, true),
                            "{name:?} must be reserved by the trimming rule"
                        );
                    }
                    for name in ["conifer.dat", "nullable.bin", "com10.dat", "lpt", "COM0"] {
                        assert!(
                            !is_reserved_device_name(name, DEVICE_NAMES_TRIMMING, true),
                            "{name:?} must be ordinary under the trimming rule"
                        );
                    }
                },
            ),
            (
                "raw_rule_omits_clock_and_keeps_trailing_characters",
                &|| {
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
                            is_reserved_device_name(name, DEVICE_NAMES_RAW, false),
                            "{name:?} must be reserved by the raw rule"
                        );
                    }
                    // CLOCK$ and a trailing space in the stem are ordinary under
                    // this rule. (A trailing dot is already dropped by the
                    // extension split, so it is the trailing *space* that
                    // separates the two callers' trim policies.)
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
                            !is_reserved_device_name(name, DEVICE_NAMES_RAW, false),
                            "{name:?} must be ordinary under the raw rule"
                        );
                    }
                },
            ),
            ("trailing_dot_or_space_is_a_pure_mechanic", &|| {
                for name in ["x.", "x ", "x...", "x   ", ".", " "] {
                    assert!(has_trailing_dot_or_space(name), "{name:?}");
                }
                for name in ["x", "x.y", "x y", ".hidden", ""] {
                    assert!(!has_trailing_dot_or_space(name), "{name:?}");
                }
            }),
            ("ascii_fold_follows_the_callers_policy_flag", &|| {
                assert_eq!(fold_ascii_case("ReadMe.TXT", true), "readme.txt");
                assert_eq!(fold_ascii_case("ReadMe.TXT", false), "ReadMe.TXT");
                // The fold is ASCII-only on both settings: a Unicode case
                // mapping would merge the Kelvin sign with `k`, which the
                // filesystem keeps apart.
                for fold in [true, false] {
                    assert_eq!(
                        fold_ascii_case("\u{212A}/\u{00C4}", fold),
                        "\u{212A}/\u{00C4}"
                    );
                }
            }),
            ("forward_slash_follows_the_callers_policy_flag", &|| {
                assert_eq!(forward_slash("dir\\sub\\f.txt", true), "dir/sub/f.txt");
                assert_eq!(forward_slash("dir\\sub\\f.txt", false), "dir\\sub\\f.txt");
                // A leading backslash becomes a root only when the caller
                // asks for the rewrite; on the other host it stays a
                // literal character.
                assert_eq!(forward_slash("\\lead.txt", true), "/lead.txt");
                assert_eq!(forward_slash("\\lead.txt", false), "\\lead.txt");
            }),
            ("native_separator_is_the_inverse_rewrite", &|| {
                let sep = std::path::MAIN_SEPARATOR_STR;
                assert_eq!(
                    native_separator("dir/sub/f.txt", true),
                    format!("dir{sep}sub{sep}f.txt")
                );
                assert_eq!(native_separator("dir/sub/f.txt", false), "dir/sub/f.txt");
                // The two rewrites compose back to the caller's spelling on
                // the host the caller is rewriting for.
                assert_eq!(
                    forward_slash(&native_separator("dir/sub/f.txt", true), true),
                    "dir/sub/f.txt"
                );
            }),
        ];
        for (label, run) in subcases {
            println!("subcase: {label}");
            run();
        }
    }
}
