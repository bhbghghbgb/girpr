//! The mechanism behind the "is this component a Windows device name?" check.
//!
//! Both the repair binary and the capture tool ask this question, but they do
//! not ask it with the same rule: girpr reserves `CLOCK$` and strips trailing
//! dot/space before comparing, gircap reserves one fewer name and compares the
//! raw stem. Which rule is right is a separate question (and changing either
//! can move bundle bytes), so this crate deliberately does **not** choose. It
//! owns only the shared mechanism: split off the extension, optionally trim,
//! compare case-insensitively against a caller-supplied set, and apply the
//! `COM1`–`COM9` / `LPT1`–`LPT9` suffix pattern.
//!
//! Each caller passes its own reserved names and trimming flag, so this crate
//! holds no program policy and cannot make the two rules drift into agreement.

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

#[cfg(test)]
mod tests {
    use super::is_reserved_device_name;

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
}
