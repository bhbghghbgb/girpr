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
//! Every case that needs a directory symlink asks this module for one: the
//! creation, the classifier, the skip-or-fail decision and the switch are
//! defined here once, so two cases cannot disagree about what a host refusal
//! is, about which failures are worth a skip, or about the variable that
//! turns a skip into a failure.
//!
//! This is test support, not a production helper: it is deliberately not
//! reachable from the integration tests' `test_support` seam.

use crate::test_support::{FixtureRefusal, skip_fixture_refusal};
use std::path::Path;

/// Create a directory symlink at `link` that points at `target`.
///
/// The platform's own call is used, so a refusal carries the platform's own
/// code: `symlink(2)` on Unix and `CreateSymbolicLinkW` on Windows. A
/// platform with neither call refuses with an `Unsupported` error of its own,
/// which carries no raw code and is the one refusal
/// [`platform_cannot_create_symlinks`] can name.
///
/// A junction is not a symlink and needs no privilege, so this is the
/// directory-symlink fixture only: the junction support owns that one.
pub(crate) fn try_symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "directory symlinks are not supported on this platform",
        ))
    }
}

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

/// The Unix errno values that mean "there is no directory symlink here":
/// `ENOSYS`, the syscall is not implemented on this platform, and
/// `EOPNOTSUPP`, the filesystem cannot hold symlinks.
///
/// The numbers are the platform's own, not Rust's, and the Unix flavours
/// number them differently, so each target names its own values: 38 and 95 in
/// Linux's asm-generic errno table, which Android shares; 78 and 45 on the
/// BSDs; and 78, 45 and 102 on Apple platforms, which number the two
/// "not supported" errors separately: 45 is `ENOTSUP`, the answer a
/// filesystem that cannot hold the entry gives, and 102 is `EOPNOTSUPP`, the
/// socket spelling of the same refusal. A Unix target this table does not
/// name — the MIPS and SPARC Linux targets, which number their errors
/// differently, and every other flavour — names no code at all, so its
/// refusals fail closed instead of skipping.
#[cfg(unix)]
fn unix_unsupported_codes() -> &'static [i32] {
    if cfg!(target_vendor = "apple") {
        &[78, 45, 102]
    } else if cfg!(any(
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "openbsd",
        target_os = "netbsd"
    )) {
        &[78, 45]
    } else if cfg!(any(target_os = "linux", target_os = "android"))
        && !cfg!(any(
            target_arch = "mips",
            target_arch = "mips32r6",
            target_arch = "mips64",
            target_arch = "mips64r6",
            target_arch = "sparc",
            target_arch = "sparc64"
        ))
    {
        &[38, 95]
    } else {
        &[]
    }
}

/// Whether the platform itself cannot create a directory symlink.
///
/// The answer comes from the raw OS code, never from `std`'s
/// `ErrorKind::Unsupported` on its own: that kind is `std`'s own mapping,
/// and it may widen, so a failure it starts calling "unsupported" would be
/// skipped instead of failing loudly.
///
/// The codes are the platform's own "there is no symlink here" answers:
///
/// - Unix: `ENOSYS` and `EOPNOTSUPP`/`ENOTSUP`, as
///   [`unix_unsupported_codes`] numbers them for this target. A Unix target
///   that table does not name names no code.
/// - Windows: `ERROR_CALL_NOT_IMPLEMENTED` (1201).
///
/// An `Unsupported` error that carries no raw code did not come from a
/// syscall: it is this crate's own statement that the platform has no
/// directory-symlink API (the platform branch of [`try_symlink_dir`]).
///
/// Every other raw code fails closed: this rule has not named it, so the
/// case fails loudly instead of skipping.
fn platform_cannot_create_symlinks(kind: std::io::ErrorKind, raw: Option<i32>) -> bool {
    match raw {
        #[cfg(unix)]
        Some(code) => unix_unsupported_codes().contains(&code),
        #[cfg(windows)]
        Some(1201) => true,
        _ => kind == std::io::ErrorKind::Unsupported && raw.is_none(),
    }
}

/// Why a failed directory-symlink creation means the case cannot run on
/// this host, or `None` when it is a broken test environment that must
/// fail loudly.
///
/// Two refusals are provable, and only these two: the platform cannot
/// create a directory symlink at all
/// ([`platform_cannot_create_symlinks`]), and the creation was denied
/// ([`link_creation_denied`]). Every other error is a broken test
/// environment and must panic rather than quietly skip.
///
/// The rule does not lean on `std`'s `ErrorKind::Unsupported` alone: that
/// kind is `std`'s mapping, and it may widen, so a failure it starts
/// calling "unsupported" would swallow a broken environment. The raw code
/// decides instead, and a code the rule cannot name fails the case.
pub(crate) fn symlink_skip_reason(e: &std::io::Error) -> Option<&'static str> {
    if platform_cannot_create_symlinks(e.kind(), e.raw_os_error()) {
        Some("directory symlinks are not supported on this platform")
    } else if link_creation_denied(e) {
        Some("creating a directory symlink is denied on this host")
    } else {
        None
    }
}

/// The one skip-or-fail decision for a directory-symlink fixture.
///
/// A proven host refusal skips the case — or fails it when the switch says
/// the case must run — and every other failure is a broken test environment
/// that must fail loudly, so a run meant to prove the symlink behaviour
/// cannot pass having checked nothing.
///
/// Every caller takes this decision here, at the point where the fixture
/// failed, so a case that skips and a case that fails cannot disagree about
/// which is which.
pub(crate) fn symlink_fixture_decision(e: std::io::Error, required: bool, refusal: FixtureRefusal) {
    match symlink_skip_reason(&e) {
        Some(why) => skip_symlink(&format!("{why}: {e}"), required, refusal),
        None => panic!("could not create the test directory symlink: {e}"),
    }
}

/// Report that the directory symlink could not be created, or fail when
/// `GIRPR_TEST_REQUIRE_SYMLINK` says the case must not be skipped. A run
/// meant to prove the symlink behaviour must not pass having checked
/// nothing.
///
/// The refusal is the shared one, so the symlink cases report a host
/// refusal the same way wherever they live.
pub(crate) fn skip_symlink(why: &str, required: bool, refusal: FixtureRefusal) {
    skip_fixture_refusal(
        refusal,
        "a directory symlink",
        "GIRPR_TEST_REQUIRE_SYMLINK",
        why,
        required,
    );
}

/// Whether the symlink cases must run on this host.
///
/// The switch is presence-based: any value, including `0`, requires the
/// case, so a run meant to prove the symlink behaviour cannot pass having
/// checked nothing.
pub(crate) fn require_symlink() -> bool {
    std::env::var_os("GIRPR_TEST_REQUIRE_SYMLINK").is_some()
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

    /// The `Unsupported` rule is decided from the raw code, not from `std`'s
    /// mapping: a code the rule cannot name fails closed, even when `std`
    /// calls the failure "unsupported". A mapping that widens must not be
    /// able to swallow a broken test environment.
    #[test]
    fn an_unsupported_failure_the_rule_cannot_name_fails_closed() {
        assert!(
            !platform_cannot_create_symlinks(std::io::ErrorKind::Unsupported, Some(9999)),
            "a raw code the rule does not name is not a proven platform refusal"
        );
        assert!(
            !platform_cannot_create_symlinks(std::io::ErrorKind::Unsupported, Some(5)),
            "a permission failure is not a platform refusal, whatever its kind says"
        );
        assert!(
            platform_cannot_create_symlinks(std::io::ErrorKind::Unsupported, None),
            "the crate's own unsupported refusal carries no raw code"
        );
        assert!(
            !platform_cannot_create_symlinks(std::io::ErrorKind::PermissionDenied, None),
            "a refusal with no raw code is only a platform refusal when it says so"
        );
    }

    /// The platform's own "there is no symlink here" answers are proven
    /// refusals, so a host that cannot hold one skips instead of failing.
    /// The numbers are the platform's, not this module's: they are pinned per
    /// target below, so a flavour that renumbers them fails here rather than
    /// skipping on a code that means something else.
    #[test]
    fn the_platforms_own_unsupported_codes_are_a_proven_refusal() {
        #[cfg(unix)]
        {
            #[cfg(target_vendor = "apple")]
            assert_eq!(
                unix_unsupported_codes(),
                [78, 45, 102],
                "ENOSYS, ENOTSUP and EOPNOTSUPP on Apple platforms"
            );
            #[cfg(any(
                target_os = "freebsd",
                target_os = "dragonfly",
                target_os = "openbsd",
                target_os = "netbsd"
            ))]
            assert_eq!(
                unix_unsupported_codes(),
                [78, 45],
                "ENOSYS and EOPNOTSUPP on the BSDs"
            );
            #[cfg(all(
                any(target_os = "linux", target_os = "android"),
                not(any(
                    target_arch = "mips",
                    target_arch = "mips32r6",
                    target_arch = "mips64",
                    target_arch = "mips64r6",
                    target_arch = "sparc",
                    target_arch = "sparc64"
                ))
            ))]
            assert_eq!(
                unix_unsupported_codes(),
                [38, 95],
                "ENOSYS and EOPNOTSUPP in Linux's asm-generic errno table"
            );
            for &code in unix_unsupported_codes() {
                assert!(
                    platform_cannot_create_symlinks(std::io::ErrorKind::Unsupported, Some(code)),
                    "the platform's own code {code} is a refusal"
                );
                assert_eq!(
                    symlink_skip_reason(&std::io::Error::from_raw_os_error(code)),
                    Some("directory symlinks are not supported on this platform"),
                    "a filesystem that cannot hold a symlink skips, not panics"
                );
            }
        }
        #[cfg(windows)]
        {
            assert!(
                platform_cannot_create_symlinks(std::io::ErrorKind::Unsupported, Some(1201)),
                "the platform's own code 1201 is a refusal"
            );
            assert_eq!(
                symlink_skip_reason(&std::io::Error::from_raw_os_error(1201)),
                Some("directory symlinks are not supported on this platform"),
                "a platform that cannot make the link skips, not panics"
            );
        }
    }

    /// Without the switch, a proven host refusal skips the symlink case out
    /// loud instead of failing it.
    #[test]
    fn a_symlink_fixture_refusal_skips_without_the_switch() {
        skip_symlink("injected refusal", false, FixtureRefusal::Injected);
    }

    /// The same refusal is a failure when the switch is set, whatever its
    /// value: the case must not pass having proved nothing.
    #[test]
    #[should_panic(expected = "GIRPR_TEST_REQUIRE_SYMLINK is set")]
    fn a_symlink_fixture_refusal_fails_with_the_switch() {
        skip_symlink("injected refusal", true, FixtureRefusal::Injected);
    }
}
