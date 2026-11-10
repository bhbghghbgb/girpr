//! Unit-test support for directory junctions.
//!
//! The `config.ini` writer and the containment tests both need a junction
//! fixture and the same classifier for a failure to create one. The code lives
//! here once so the two test modules cannot drift on its messages or its skip
//! set. The module is declared only for the crate's unit tests on Windows.
//!
//! The command's outcome handling is split from the command's execution, so
//! a test can drive both branches — a successful exit that created no link,
//! and a failing exit — without running `mklink` at all.
//!
//! This is test support, not a production helper: it is deliberately not
//! reachable from the integration tests' `test_support` seam.

use std::path::Path;

/// What the `mklink /J` command reported.
///
/// The outcome is split from the command's execution so a test can hand the
/// helper an exit without running `mklink`: a successful exit that created
/// no link, and a failing exit, are both reachable on a host that cannot
/// create a junction at all.
pub(super) enum JunctionOutcome {
    /// The command exited with a success status. That only claims the link
    /// exists; the link is still inspected.
    Success,
    /// The command exited with a failure status.
    Failed {
        /// The exit status, spelled as `ExitStatus` spells it.
        status: String,
        /// The command's error output, trimmed.
        stderr: String,
    },
}

/// Create a directory junction with `mklink /J`, which (unlike a symlink)
/// needs no privilege. `target` need not exist: Windows stores the path in
/// the reparse point, which is exactly the dangling case.
///
/// A failure to spawn `cmd` keeps its `std::io::Error`, so the raw OS code
/// is still available to [`junction_skip_reason`]. `mklink` itself exits
/// with `1` and prints a localized message, not the Win32 code, for every
/// refusal. A non-zero exit therefore becomes an `Other` error: its cause
/// cannot be proved, so the test fails loudly instead of guessing that the
/// host refused the junction.
///
/// The command's outcome is handled by [`junction_result`], split from the
/// command's execution so a test can drive both branches without running
/// `mklink`.
///
/// An exit of `0` only claims that the link exists; the test still inspects
/// it. A link that did not appear fails closed as missing, and an
/// inspection that fails for another reason keeps that cause, so a
/// permission or transient I/O failure is never reported as missing.
pub(super) fn try_junction_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    let out = run_mklink(target, link)?;
    let outcome = if out.status.success() {
        JunctionOutcome::Success
    } else {
        JunctionOutcome::Failed {
            status: out.status.to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        }
    };
    junction_result(&outcome, link)
}

/// Run the `mklink /J` command and hand back what it reported.
///
/// `cmd` reparses the command line itself, and `Command::arg` quotes for
/// the argv convention `cmd` does not follow: a temp path with a space or
/// a command separator could split into extra arguments. Pass one
/// pre-quoted line verbatim instead, so each path reaches `mklink` as a
/// single argument. Windows paths cannot contain `"`, so a plain quote
/// pair is enough. Quotes do not stop `cmd`'s `%VAR%` expansion, though:
/// a path carrying a literal `%NAME%` would still be rewritten, so the
/// caller must keep percent pairs out of the fixture paths.
fn run_mklink(target: &Path, link: &Path) -> std::io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;

    let command = format!("mklink /J \"{}\" \"{}\"", link.display(), target.display());
    std::process::Command::new("cmd")
        .arg("/C")
        .raw_arg(command)
        .output()
}

/// The result of one `mklink /J` outcome.
///
/// A failing exit keeps the command's own report. A successful exit is only
/// a claim that the link exists, so the link is inspected: a link that never
/// appeared fails closed as missing, and an inspection that fails for
/// another reason keeps that cause, so a permission or transient I/O
/// failure is never reported as missing.
///
/// Both failures stay `Other`, so neither can be mistaken for a proven host
/// refusal by [`junction_skip_reason`].
pub(super) fn junction_result(outcome: &JunctionOutcome, link: &Path) -> std::io::Result<()> {
    if let JunctionOutcome::Failed { status, stderr } = outcome {
        return Err(std::io::Error::other(format!(
            "mklink /J exited with {status}: {stderr}"
        )));
    }
    std::fs::symlink_metadata(link)
        .map(|_| ())
        .map_err(|e| junction_inspection_error(link, e))
}

/// Turn a failed inspection of the link that `mklink` claims to have
/// created into the test's failure. `NotFound` is the link that never
/// appeared; every other cause (a permission or transient I/O failure)
/// keeps its own wording, so the panic neither claims the link is missing
/// nor names the wrong cause. Both stay `Other`, so neither can be
/// mistaken for a proven host refusal by [`junction_skip_reason`].
pub(super) fn junction_inspection_error(link: &Path, e: std::io::Error) -> std::io::Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        std::io::Error::other(format!(
            "mklink /J reported success but {} is missing",
            link.display()
        ))
    } else {
        std::io::Error::other(format!(
            "mklink /J reported success but {} could not be inspected: {e}",
            link.display()
        ))
    }
}

/// Why a failed directory-junction creation means the case cannot run on
/// this host, or `None` when it is a broken test environment that must fail
/// loudly.
///
/// Only a spawn failure can carry a raw OS code, so the two provable host
/// refusals are an unsupported platform and the known privilege denial
/// (`ERROR_PRIVILEGE_NOT_HELD`). `mklink` itself reports every refusal as
/// exit `1` plus localized text and no Win32 code, so its failure arrives
/// as `Other` and stays unclassified: guessing from the exit status or the
/// message text would also hide a broken `cmd`/`mklink` environment, which
/// is exactly what this classifier exists to expose. The raw-code arm
/// therefore belongs to the spawn path — and to the injected test that
/// pins the classifier's contract — not to a `mklink` refusal that only
/// looks like it. A host whose volume cannot hold a junction still fails
/// loudly rather than silently skipping.
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

mod tests {
    use super::*;

    /// Only a spawn failure that proves an unsupported platform or the known
    /// privilege denial skips the junction case. A failed `mklink` command has
    /// no raw code to classify, whatever its text says, so it must fail the
    /// test rather than skip.
    #[test]
    fn only_a_proven_host_refusal_skips_the_junction() {
        assert!(
            junction_skip_reason(&std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "cmd is missing",
            ))
            .is_none(),
            "a missing cmd is a broken environment, not a host skip"
        );
        assert!(
            junction_skip_reason(&std::io::Error::other(
                "mklink /J exited with exit code: 1: access denied",
            ))
            .is_none(),
            "a failed mklink command has no provable raw code and must fail"
        );
        assert!(
            junction_skip_reason(&std::io::Error::other(
                "mklink /J exited with exit code: 1: 1314 privilege not held",
            ))
            .is_none(),
            "a 1314 in the localized text is not a raw code and must fail"
        );
        assert!(
            junction_skip_reason(&std::io::Error::from_raw_os_error(5)).is_none(),
            "another access failure must still fail the test"
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

    /// `mklink` exiting `0` is a claim, not proof: the inspection that follows
    /// separates a link that never appeared from one that could not be looked
    /// at. A permission or transient I/O cause is carried verbatim instead of
    /// being relabelled missing, and neither outcome may skip the case.
    #[test]
    fn an_uninspectable_junction_names_its_cause_and_fails_closed() {
        let link = Path::new("game-dir-link");

        let missing = junction_inspection_error(
            link,
            std::io::Error::new(std::io::ErrorKind::NotFound, "nothing there"),
        );
        let msg = format!("{missing:#}");
        assert!(msg.contains("is missing"), "{msg}");
        assert!(!msg.contains("could not be inspected"), "{msg}");
        assert!(
            junction_skip_reason(&missing).is_none(),
            "a link that never appeared must fail, not skip: {msg}"
        );

        let inspected = junction_inspection_error(
            link,
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "the link could not be read",
            ),
        );
        let msg = format!("{inspected:#}");
        assert!(msg.contains("could not be inspected"), "{msg}");
        assert!(
            msg.contains("the link could not be read"),
            "the inspection cause survives: {msg}"
        );
        assert!(
            !msg.contains("is missing"),
            "an unreadable link is not a missing one: {msg}"
        );
        assert!(
            junction_skip_reason(&inspected).is_none(),
            "an inspection failure must fail, not skip: {msg}"
        );
    }

    /// The exit-`0` branch, driven through the outcome seam: `mklink`
    /// claims the link exists, and the link never appeared. The failure
    /// must be honest — the link is missing — and must not read as a
    /// proven host refusal, so the case fails instead of skipping.
    ///
    /// The link path is a scratch directory's child that was never
    /// created, so the case runs on every host: no `mklink` is run.
    #[test]
    fn a_successful_exit_without_a_link_fails_closed_as_missing() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_junction", "missing_link");
        let link = dir.join("game-dir-link");

        let err = junction_result(&JunctionOutcome::Success, &link)
            .expect_err("a claimed link that never appeared must fail");
        assert_eq!(err.kind(), std::io::ErrorKind::Other, "{err}");
        let msg = format!("{err}");
        assert!(msg.contains("is missing"), "{msg}");
        assert!(msg.contains(&link.display().to_string()), "{msg}");
        assert!(
            junction_skip_reason(&err).is_none(),
            "a link that never appeared must fail, not skip: {msg}"
        );
    }

    /// The failing exit, driven through the same seam: the command's own
    /// report is the error. `mklink` reports every refusal as exit `1`
    /// plus localized text and no Win32 code, so the cause cannot be
    /// proved and the failure must not be read as a host refusal.
    #[test]
    fn a_failing_exit_keeps_the_command_report_and_does_not_skip() {
        let err = junction_result(
            &JunctionOutcome::Failed {
                status: "exit code: 1".to_string(),
                stderr: "access denied".to_string(),
            },
            Path::new("game-dir-link"),
        )
        .expect_err("a failed mklink must fail");
        assert_eq!(err.kind(), std::io::ErrorKind::Other, "{err}");
        let msg = format!("{err}");
        assert!(msg.contains("exited with exit code: 1"), "{msg}");
        assert!(msg.contains("access denied"), "{msg}");
        assert!(
            junction_skip_reason(&err).is_none(),
            "a failed mklink has no provable raw code and must fail, not skip: {msg}"
        );
    }
}
