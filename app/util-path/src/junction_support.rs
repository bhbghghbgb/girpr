//! Windows-only unit-test support for directory junctions.
//!
//! The `config.ini` writer and the containment tests both need a junction
//! fixture and the same classifier for a failure to create one. The code lives
//! here once so the two test modules cannot drift on its messages or its skip
//! set. The module is declared only for the crate's unit tests on Windows.
//!
//! Creation is delegated to `junction::create`, which owns the Win32
//! reparse-point mechanics. What stays here is the policy around it: only a
//! proven host refusal skips a case, and every other failure is a broken test
//! environment that must fail loudly, so a run meant to prove the junction
//! behaviour cannot pass having checked nothing. Both callers take that
//! decision at the point where the fixture failed, and the cases that drive it
//! directly make a broken fixture fail instead of skipping.
//!
//! This is test support, not a production helper: it is deliberately not
//! reachable from the integration tests' `test_support` seam.

use crate::test_support::{FixtureRefusal, skip_fixture_refusal};
use std::path::Path;

/// Create a directory junction at `link` pointing at `target`.
///
/// `junction::create` takes the target first and the junction path second, and
/// owns the Win32 reparse-point mechanics: no shell, no quoting and no
/// environment-dependent rewriting of the path. NTFS is assumed. The target
/// need not exist — the path is stored in the reparse point, which is exactly
/// the dangling case one of the callers needs.
///
/// An error keeps its raw OS code, so [`junction_skip_reason`] can still tell a
/// proven host refusal from a broken fixture. An existing link path, for
/// example, is reported as `AlreadyExists`, not as a refusal, and fails the
/// case instead of skipping it.
pub fn try_junction_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    junction::create(target, link)
}

/// Why a failed junction creation means the case cannot run on this host, or
/// `None` when it is a broken test environment that must fail loudly.
///
/// Only a failure with a provable OS cause is classified: an unsupported
/// platform, or the known privilege denial (`ERROR_PRIVILEGE_NOT_HELD`). Every
/// other cause — a missing parent, a link path that already exists, or a
/// transient I/O error — is a broken fixture. Guessing from an error message
/// would also hide a broken test environment, which is exactly what this
/// classifier exists to expose.
///
/// A host whose volume cannot hold a junction still fails loudly rather than
/// silently skipping.
pub(super) fn junction_skip_reason(e: &std::io::Error) -> Option<&'static str> {
    if e.kind() == std::io::ErrorKind::Unsupported {
        Some("directory junctions are not supported on this platform")
    } else if e.raw_os_error() == Some(1314) {
        // `ERROR_PRIVILEGE_NOT_HELD`; Rust maps it to `Uncategorized`, so
        // the raw code is the reliable check.
        Some("creating a directory junction is denied on this host")
    } else {
        None
    }
}

/// What a caller does with a junction it could not create.
///
/// A proven host refusal skips the case; every other failure is a broken
/// test environment that must fail loudly, so a run meant to prove the
/// junction behaviour cannot pass having checked nothing.
///
/// The enum is private to this module on purpose: the decision that reads it
/// is [`junction_fixture_decision`], so a caller cannot take the broken arm
/// into a skip of its own.
enum JunctionFixture {
    /// The host refused to create the junction; the case skips with this
    /// reason and reports the failure it came from.
    HostRefused {
        /// Why this host cannot create the junction.
        why: &'static str,
        /// The failure the host reported.
        error: std::io::Error,
    },
    /// The junction was not created for another reason; the case fails with
    /// this error.
    Broken(std::io::Error),
}

/// Classify a failed junction creation for a caller's case.
///
/// The decision is the shared classifier's, so it is taken here once and can
/// be driven directly: a create that failed for an unproven reason is a broken
/// fixture and not a host refusal, so the case must fail rather than skip.
fn classify_junction_fixture(e: std::io::Error) -> JunctionFixture {
    match junction_skip_reason(&e) {
        Some(why) => JunctionFixture::HostRefused { why, error: e },
        None => JunctionFixture::Broken(e),
    }
}

/// The one skip-or-fail decision for a junction fixture.
///
/// A proven host refusal skips the case — or fails it when the switch says the
/// case must run — and every other failure is a broken test environment that
/// must fail loudly, so a run meant to prove the junction behaviour cannot
/// pass having checked nothing.
///
/// Both callers take this decision here, at the point where the fixture
/// failed, and the cases that need a broken fixture drive this function
/// directly: a decision that turned the broken arm into a skip would fail
/// those cases instead of leaving them green.
pub fn junction_fixture_decision(e: std::io::Error, required: bool, refusal: FixtureRefusal) {
    match classify_junction_fixture(e) {
        JunctionFixture::HostRefused { why, error } => {
            skip_junction(&format!("{why}: {error}"), required, refusal);
        }
        JunctionFixture::Broken(e) => {
            panic!("could not create the test directory junction: {e}");
        }
    }
}

/// What the shared decision did, for a case that drives it directly.
#[cfg(test)]
pub(super) enum JunctionDecision {
    /// The case skipped: the host refused the fixture.
    Skipped,
    /// The case failed: the fixture is broken, and the message is the
    /// failure's own.
    Failed(String),
}

/// Drive [`junction_fixture_decision`] and report what it did.
///
/// The decision fails a broken fixture by panicking, so a case that drives it
/// catches that panic: the outcome says whether the case skipped or failed,
/// and the failure carries the message the panic showed.
#[cfg(test)]
pub(super) fn drive_junction_decision(
    e: std::io::Error,
    required: bool,
    refusal: FixtureRefusal,
) -> JunctionDecision {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        junction_fixture_decision(e, required, refusal);
    })) {
        Ok(()) => JunctionDecision::Skipped,
        Err(payload) => JunctionDecision::Failed(panic_message(payload)),
    }
}

/// The message of a caught panic.
#[cfg(test)]
fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_else(|| "the panic carried no message".to_string())
}

/// Report that the junction could not be created, or fail when
/// `GIRPR_TEST_REQUIRE_JUNCTION` says the case must not be skipped. A run
/// meant to prove the junction behaviour must not pass having checked
/// nothing.
///
/// The refusal is the shared one, so the junction cases report a host
/// refusal the same way wherever they live.
pub(super) fn skip_junction(why: &str, required: bool, refusal: FixtureRefusal) {
    skip_fixture_refusal(
        refusal,
        "a directory junction",
        "GIRPR_TEST_REQUIRE_JUNCTION",
        why,
        required,
    );
}

/// Whether the junction cases must run on this host.
///
/// The switch is presence-based: any value, including `0`, requires the
/// junction case, so a run meant to prove the junction behaviour cannot pass
/// having checked nothing.
pub fn require_junction() -> bool {
    std::env::var_os("GIRPR_TEST_REQUIRE_JUNCTION").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a failure that proves an unsupported platform or the known
    /// privilege denial skips the junction case. Every other error kind a
    /// junction creation can report — including the raw access and
    /// already-exists codes — has no provable meaning as a host refusal, so
    /// it must fail the test rather than skip.
    #[test]
    fn only_a_proven_host_refusal_skips_the_junction() {
        assert!(
            junction_skip_reason(&std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "missing parent"
            ))
            .is_none(),
            "a missing parent is a broken environment, not a host skip"
        );
        assert!(
            junction_skip_reason(&std::io::Error::from_raw_os_error(5)).is_none(),
            "an access denial without the known privilege code must fail"
        );
        assert!(
            junction_skip_reason(&std::io::Error::from_raw_os_error(183)).is_none(),
            "an existing link path is a broken fixture, not a host skip"
        );
        assert!(
            junction_skip_reason(&std::io::Error::from_raw_os_error(1300)).is_none(),
            "a failed privilege adjustment is not the known denial code"
        );
        assert_eq!(
            junction_skip_reason(&std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "no directory junctions",
            )),
            Some("directory junctions are not supported on this platform"),
            "an unsupported platform is a skip, not a panic"
        );
        assert_eq!(
            junction_skip_reason(&std::io::Error::from_raw_os_error(1314)),
            Some("creating a directory junction is denied on this host"),
            "the privilege denial skips, not panics"
        );
    }

    /// A proven host refusal reaches the shared decision directly: the case
    /// skips, with the reason and the failure it came from.
    #[test]
    fn a_proven_host_refusal_skips_the_case() {
        // `ERROR_PRIVILEGE_NOT_HELD`; the one raw code the classifier proves.
        let err = std::io::Error::from_raw_os_error(1314);
        let outcome = drive_junction_decision(err, false, FixtureRefusal::Injected);
        assert!(
            matches!(outcome, JunctionDecision::Skipped),
            "a proven host refusal skips the case instead of failing it"
        );
    }

    /// The same refusal is a failure when the switch is set, whatever its
    /// value: the case must not pass having proved nothing.
    #[test]
    fn a_proven_host_refusal_fails_when_the_switch_is_set() {
        let err = std::io::Error::from_raw_os_error(1314);
        let outcome = drive_junction_decision(err, true, FixtureRefusal::Injected);
        match outcome {
            JunctionDecision::Skipped => {
                panic!("the switch must turn the skip into a failure")
            }
            JunctionDecision::Failed(message) => {
                assert!(
                    message.contains("GIRPR_TEST_REQUIRE_JUNCTION is set"),
                    "the failure names the switch: {message}"
                );
            }
        }
    }

    /// A real, unclassified `junction::create` failure reaches the same
    /// decision and fails the case: a plain directory at the link path cannot
    /// stand in for a junction, and the fixture's own error survives into the
    /// message.
    #[test]
    fn a_create_failure_with_an_unproven_cause_fails_the_case() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_junction", "broken_decision");
        let target = dir.join("real");
        let link = dir.join("game-dir-link");
        std::fs::create_dir(&link).expect("the link path already holds a plain directory");

        let err = try_junction_dir(&target, &link)
            .expect_err("an existing plain directory cannot become a junction");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists, "{err}");
        assert_eq!(err.raw_os_error(), Some(183), "{err}");
        assert!(
            junction_skip_reason(&err).is_none(),
            "an existing link is not a host refusal: {err}"
        );

        match drive_junction_decision(err, false, FixtureRefusal::Injected) {
            JunctionDecision::Skipped => {
                panic!("a plain directory at the link path is not a host refusal")
            }
            JunctionDecision::Failed(message) => {
                assert!(
                    message.contains("could not create the test directory junction"),
                    "the failure names the fixture: {message}"
                );
                assert!(
                    message.contains("already exists"),
                    "the create's own error survives: {message}"
                );
            }
        }
    }

    /// A successful create leaves exactly the fixture the callers need: a
    /// name-surrogate reparse point that points at the given target, with a
    /// target that need not exist and paths that need no quoting. The target
    /// comes first in `junction::create`, so `get_target` pins the argument
    /// order against a reversal.
    #[test]
    fn a_successful_create_leaves_a_name_surrogate_reparse_point() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_junction", "created_fixture");
        let target = dir.join("never-created%TARGET%");
        let link = dir.join("game-dir%LINK%");

        if let Err(e) = try_junction_dir(&target, &link) {
            junction_fixture_decision(e, require_junction(), FixtureRefusal::Host);
            return;
        }
        let md = std::fs::symlink_metadata(&link).expect("the junction is inspectable");
        assert!(
            md.file_type().is_symlink(),
            "the fixture is a name-surrogate reparse point"
        );
        assert!(!target.exists(), "the junction's target may be dangling");
        assert_eq!(
            junction::get_target(&link).expect("the junction has a target"),
            target,
            "the first argument is the target and the second is the junction"
        );
    }
}
