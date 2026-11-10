//! Unit-test support for directory symlinks.
//!
//! The `config.ini` writer's dangling-symlink case has to tell a host refusal
//! apart from a broken test environment, and that rule is not Windows-only:
//! the same case runs on every host. The classifier lives here, in a module of
//! its own, because the junction fixture support is declared for Windows only
//! and a rule that runs everywhere cannot live behind that gate.
//!
//! The module is declared for the crate's unit tests on every host, and the
//! platform's own refusal codes are inside it, so the rule is the same
//! wherever the case runs.
//!
//! This is test support, not a production helper: it is deliberately not
//! reachable from the integration tests' `test_support` seam.

/// Whether a failed **directory symlink** creation is the known privilege
/// denial that means the test cannot run on this host. Every other error
/// is a broken test environment and must fail loudly instead of silently
/// skipping.
///
/// A junction's refusal is not classified here: `mklink` reports it
/// without a raw code, so the shared junction classifier owns that case.
pub(super) fn link_creation_denied(e: &std::io::Error) -> bool {
    #[cfg(windows)]
    {
        // A directory symlink the host's policy refuses to create reports
        // `ERROR_PRIVILEGE_NOT_HELD` (1314); a directory symlink without
        // the privilege (or developer mode) is the usual case. Rust maps
        // it to `Uncategorized`, so the raw code is the reliable check.
        e.raw_os_error() == Some(1314)
    }
    #[cfg(unix)]
    {
        // `symlink(2)` reports `EPERM` (1) or `EACCES` (13) when the
        // caller lacks permission. Anything else (a read-only filesystem,
        // for example) is not the privilege denial this test skips on.
        matches!(e.raw_os_error(), Some(1) | Some(13))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = e;
        false
    }
}

/// Why a failed directory-symlink creation means the case cannot run on
/// this host, or `None` when it is a broken test environment that must
/// fail loudly.
///
/// A platform without directory symlinks reports `Unsupported`; the known
/// privilege denial is the other untestable host.
///
/// Treating every `Unsupported` as a host skip is deliberate: `std` may map
/// more platform failures there over time, and a host that cannot make the
/// link cannot exercise the case. Any other error is a broken test
/// environment and must still panic rather than quietly skip.
pub(super) fn symlink_skip_reason(e: &std::io::Error) -> Option<&'static str> {
    if e.kind() == std::io::ErrorKind::Unsupported {
        Some("directory symlinks are not supported on this platform")
    } else if link_creation_denied(e) {
        Some("creating a directory symlink is denied on this host")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only the known privilege denial or a platform without directory
    /// symlinks is a skip; other failures would hide a broken test
    /// environment.
    #[test]
    fn only_a_denied_or_unsupported_symlink_skips_the_link_test() {
        assert!(
            symlink_skip_reason(&std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "not a creation-denied error",
            ))
            .is_none(),
            "an unexpected failure must still fail the test"
        );
        assert_eq!(
            symlink_skip_reason(&std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "no directory symlinks",
            )),
            Some("directory symlinks are not supported on this platform"),
            "an unsupported platform is a skip, not a panic"
        );
        #[cfg(windows)]
        {
            assert!(link_creation_denied(&std::io::Error::from_raw_os_error(
                1314
            )));
            assert!(!link_creation_denied(&std::io::Error::from_raw_os_error(5)));
            assert_eq!(
                symlink_skip_reason(&std::io::Error::from_raw_os_error(1314)),
                Some("creating a directory symlink is denied on this host"),
                "the privilege denial skips, not panics"
            );
            assert!(
                symlink_skip_reason(&std::io::Error::from_raw_os_error(5)).is_none(),
                "another access failure must still fail the test"
            );
        }
        #[cfg(unix)]
        {
            assert!(link_creation_denied(&std::io::Error::from_raw_os_error(1)));
            assert!(link_creation_denied(&std::io::Error::from_raw_os_error(13)));
            assert!(!link_creation_denied(&std::io::Error::from_raw_os_error(
                30
            )));
            for denied in [1, 13] {
                assert_eq!(
                    symlink_skip_reason(&std::io::Error::from_raw_os_error(denied)),
                    Some("creating a directory symlink is denied on this host"),
                    "the privilege denial skips, not panics"
                );
            }
            assert!(
                symlink_skip_reason(&std::io::Error::from_raw_os_error(30)).is_none(),
                "a read-only filesystem must still fail the test"
            );
        }
    }
}
