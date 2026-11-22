//! Shared lexical shape validation for values that name something relative to
//! a root.
//!
//! Two programs ask the same low-level questions of a `/`-spelled relative
//! path — is it empty, does it leave its root, does a component end in the dot
//! or space Windows strips, is a component a reserved device name — but they
//! answer them under opposite normalization rules. One rejects every spelling
//! that is not already the plain form and keeps the caller's exact bytes; the
//! other rewrites `\` to `/`, drops `.` components and collapses repeated
//! separators, then stores the cleaned form it returns. Which rule is right is
//! a separate question: a wrongly kept `./` can make one program's cleanup
//! delete the file it just repaired, and the other program's stable refusal
//! identifiers are published in a diagnostics document.
//!
//! This crate owns only the lexical skeleton: one pass over the value, in one
//! fixed order, calling the name predicates in [`girpr_fs_names`]. It owns no
//! program wording, no stable identifier and no default rule. [`ShapePolicy`]
//! carries the caller's normalization and device parameters, and
//! [`ShapeProblem`] is the pure reason the caller maps to its own message (and,
//! where it publishes one, its own byte-stable identifier).

/// A lexical reason a value is not a legal relative path shape.
///
/// Pure and program-free: a caller decides what each variant means in its own
/// error type, wording and, if it has one, stable rule identifier. The variants
/// are ordered by the check that produces them; callers that publish a first
/// failing rule depend on that order staying fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeProblem {
    /// The value is empty.
    Empty,
    /// The value contains a control character.
    Control,
    /// The value uses `\` where a `/`-separated shape is required.
    Backslash,
    /// The value is rooted or absolute.
    Absolute,
    /// The value opens with a drive qualifier (`C:…`).
    DrivePrefix,
    /// A component carries a `:` (an NTFS alternate-stream separator).
    Colon,
    /// A component is `..`, which leaves the root.
    Parent,
    /// A component is `.`.
    DotComponent,
    /// A component ends in the `.` or space Windows strips when resolving.
    WindowsFold,
    /// A component names a reserved device.
    Device,
    /// A component is empty (a doubled or trailing separator).
    EmptyComponent,
    /// The value ends in a separator, so it names no single object.
    TrailingSeparator,
    /// After normalization the value names no component.
    NoName,
}

/// How a caller spells a relative path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Normalization {
    /// Refuse every spelling that is not already the plain `/` form: reject
    /// `\`, `.` and empty components and control characters, and return the
    /// caller's value unchanged.
    Strict,
    /// Rewrite `\` to `/`, drop `.` components and collapse empty ones, then
    /// validate and return the cleaned `/`-spelled form for the caller to
    /// store.
    Normalizing,
}

/// The caller's path-shape policy.
///
/// Only the parameters that genuinely differ between callers are fields. The
/// set of reserved device names, whether the stem is trimmed before comparing
/// it, and whether devices are refused at all are the caller's rule, not this
/// crate's; the shape checks and their order are shared.
#[derive(Debug, Clone, Copy)]
pub struct ShapePolicy<'a> {
    pub normalization: Normalization,
    /// Literal reserved names; `COM1`–`COM9` and `LPT1`–`LPT9` are matched by
    /// the shared predicate regardless of this set.
    pub device_names: &'a [&'a str],
    /// Whether trailing `.` and ` ` are stripped from the stem before it is
    /// compared with [`Self::device_names`].
    pub device_trim: bool,
    /// Whether a reserved device component is refused (a logical spelling may
    /// allow it and escape it on disk instead).
    pub reject_device: bool,
}

/// Validate `value` under `policy`, returning the spelling the caller should
/// use.
///
/// Under [`Normalization::Strict`] the returned string is `value` itself: a
/// non-plain spelling is refused, never rewritten. Under
/// [`Normalization::Normalizing`] it is the cleaned `/`-separated form, with
/// `.` components and empty components removed. The check order is fixed
/// because a caller that records the first failing rule publishes it.
pub fn validate(value: &str, policy: &ShapePolicy<'_>) -> Result<String, ShapeProblem> {
    if value.is_empty() {
        return Err(ShapeProblem::Empty);
    }
    let normalizing = policy.normalization == Normalization::Normalizing;

    // A strict spelling is printable text; a normalising caller carries the
    // value as data and only joins the cleaned result to a path.
    if !normalizing && value.chars().any(char::is_control) {
        return Err(ShapeProblem::Control);
    }

    let normalized;
    let spelling: &str = if normalizing {
        normalized = girpr_fs_names::forward_slash(value, true);
        normalized.as_str()
    } else {
        if value.contains('\\') {
            return Err(ShapeProblem::Backslash);
        }
        value
    };

    if normalizing && spelling.ends_with('/') {
        return Err(ShapeProblem::TrailingSeparator);
    }
    if spelling.starts_with('/') {
        return Err(ShapeProblem::Absolute);
    }
    let bytes = spelling.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(ShapeProblem::DrivePrefix);
    }
    if spelling.contains(':') {
        return Err(ShapeProblem::Colon);
    }

    let parts: Vec<&str> = spelling.split('/').collect();
    // A normalising spelling drops `.` and empty components before any name
    // rule is applied; a strict spelling keeps them so they can be refused in
    // the fixed order below.
    let checked: Vec<&str> = if normalizing {
        parts
            .iter()
            .copied()
            .filter(|part| !part.is_empty() && *part != ".")
            .collect()
    } else {
        parts.clone()
    };
    if checked.contains(&"..") {
        return Err(ShapeProblem::Parent);
    }
    if !normalizing && checked.contains(&".") {
        return Err(ShapeProblem::DotComponent);
    }
    if checked
        .iter()
        .any(|part| girpr_fs_names::has_trailing_dot_or_space(part))
    {
        return Err(ShapeProblem::WindowsFold);
    }
    if policy.reject_device
        && checked.iter().any(|part| {
            girpr_fs_names::is_reserved_device_name(part, policy.device_names, policy.device_trim)
        })
    {
        return Err(ShapeProblem::Device);
    }
    if !normalizing && checked.iter().any(|part| part.is_empty()) {
        return Err(ShapeProblem::EmptyComponent);
    }

    let mut clean = String::new();
    for part in checked {
        if !clean.is_empty() {
            clean.push('/');
        }
        clean.push_str(part);
    }
    if clean.is_empty() {
        return Err(ShapeProblem::NoName);
    }
    Ok(clean)
}

#[cfg(test)]
mod tests {
    use super::{Normalization, ShapePolicy, ShapeProblem, validate};

    // girpr's rule: the full set, including `CLOCK$`, with trailing trim.
    const TRIMMING: &[&str] = &["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "CLOCK$"];
    // gircap's rule: the set without `CLOCK$`, compared on the raw stem.
    const RAW: &[&str] = &["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];

    fn strict() -> ShapePolicy<'static> {
        ShapePolicy {
            normalization: Normalization::Strict,
            device_names: RAW,
            device_trim: false,
            reject_device: true,
        }
    }

    fn strict_logical() -> ShapePolicy<'static> {
        ShapePolicy {
            normalization: Normalization::Strict,
            device_names: RAW,
            device_trim: false,
            reject_device: false,
        }
    }

    fn normalizing() -> ShapePolicy<'static> {
        ShapePolicy {
            normalization: Normalization::Normalizing,
            device_names: TRIMMING,
            device_trim: true,
            reject_device: true,
        }
    }

    /// The strict spelling refuses everything that is not already plain, and
    /// returns the caller's exact bytes on success.
    #[test]
    fn strict_refuses_non_plain_spellings_and_keeps_the_value() {
        for good in [
            "a",
            "a/b/c.blk",
            "GenshinImpact_Data/Persistent/audio_lang_14",
            "a/b.c",
            "a/7.1.0",
            "a/.hidden",
            "a/b c/d",
            "a/\u{4e2d}\u{6587}",
        ] {
            assert_eq!(validate(good, &strict()).as_deref(), Ok(good), "{good:?}");
        }
        for (bad, reason) in [
            ("", ShapeProblem::Empty),
            ("a\u{0}b", ShapeProblem::Control),
            ("a\\b", ShapeProblem::Backslash),
            ("/etc/passwd", ShapeProblem::Absolute),
            ("C:/Windows", ShapeProblem::DrivePrefix),
            ("ab:c", ShapeProblem::Colon),
            ("../x", ShapeProblem::Parent),
            ("./x", ShapeProblem::DotComponent),
            ("a/b.", ShapeProblem::WindowsFold),
            ("CON", ShapeProblem::Device),
            ("a//b", ShapeProblem::EmptyComponent),
        ] {
            assert_eq!(validate(bad, &strict()), Err(reason), "{bad:?}");
        }
    }

    /// The check order is part of the contract: a caller that publishes the
    /// first failing rule must see the same one for a value that violates
    /// several rules at once.
    #[test]
    fn strict_reports_the_first_violation_in_the_fixed_order() {
        for (value, reason) in [
            ("a//CON", ShapeProblem::Device),
            ("a/./CON", ShapeProblem::DotComponent),
            ("a/../CON", ShapeProblem::Parent),
            ("C:/../CON", ShapeProblem::DrivePrefix),
            ("a\\b/../CON", ShapeProblem::Backslash),
            ("a\u{0}/../CON", ShapeProblem::Control),
        ] {
            assert_eq!(validate(value, &strict()), Err(reason), "{value:?}");
        }
    }

    /// A logical spelling may name a device without being refused.
    #[test]
    fn strict_logical_allows_devices_but_nothing_that_leaves_the_root() {
        for good in ["a/CON", "a/aux.txt", "a/%CON"] {
            assert_eq!(validate(good, &strict_logical()).as_deref(), Ok(good));
        }
        for bad in ["../x", "a\\b", "/abs", "C:/x", "a/./b", "a/b.", "a//b", ""] {
            assert!(validate(bad, &strict_logical()).is_err(), "{bad:?}");
        }
    }

    /// The normalizing spelling rewrites separators, drops `.`, collapses
    /// repeated separators and returns the plain form the caller stores.
    #[test]
    fn normalizing_returns_the_clean_slash_form() {
        for (input, want) in [
            ("a/b.dat", "a/b.dat"),
            ("a\\b.dat", "a/b.dat"),
            ("a//b.dat", "a/b.dat"),
            ("a\\\\b.dat", "a/b.dat"),
            ("./a/b.dat", "a/b.dat"),
            ("a/./b.dat", "a/b.dat"),
            ("./a//./b.dat", "a/b.dat"),
            ("game/conifer.dat", "game/conifer.dat"),
            ("game/com10.dat", "game/com10.dat"),
        ] {
            assert_eq!(
                validate(input, &normalizing()).as_deref(),
                Ok(want),
                "{input:?}"
            );
        }
    }

    /// The normalizing spelling still refuses traversal, absolute and drive
    /// spellings, trailing separators and Windows name rules.
    #[test]
    fn normalizing_keeps_every_refusal() {
        for bad in [
            "",
            ".",
            "..",
            "../outside.dat",
            "/abs/evil.dat",
            "//server/share/evil.dat",
            "\\\\server\\share\\evil.dat",
            "C:evil.dat",
            "C:/evil.dat",
            "game/",
            "game\\",
            "game/./",
            "NUL",
            "game/CLOCK$",
            "game/a:b",
            "game/trailing.",
            "dir./file.dat",
        ] {
            assert!(validate(bad, &normalizing()).is_err(), "{bad:?}");
        }
    }

    /// A device and a literal percent are distinct under both rules, and the
    /// escape prefix is a legal spelling of a device on disk.
    #[test]
    fn escaped_device_spellings_are_plain_paths() {
        for escaped in ["%CON", "%CON.txt", "%%CON"] {
            assert_eq!(validate(escaped, &strict()).as_deref(), Ok(escaped));
            assert_eq!(validate(escaped, &normalizing()).as_deref(), Ok(escaped));
        }
    }
}
