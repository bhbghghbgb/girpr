//! Unit-test support for directory junctions.
//!
//! The `config.ini` writer and the containment tests both need a junction
//! fixture and the same classifier for a failure to create one. The code lives
//! here once so the two test modules cannot drift on its messages or its skip
//! set. The module is declared only for the crate's unit tests on Windows.
//!
//! The command's outcome handling is split from the command's execution, so
//! a test can drive both branches — a successful exit that created no link,
//! a successful exit that created something that is not a junction, and a
//! failing exit — without running `mklink` at all. The command line is
//! split from its execution too, so a test can prove that a path `cmd`
//! would rewrite is refused before anything is run.
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
/// A path `cmd` would rewrite is refused before the command line is built,
/// so a fixture path can never be silently changed: the refusal names the
/// path and the reason, and it is an ordinary error rather than a spawn
/// failure, so [`junction_skip_reason`] cannot read it as a proven host
/// refusal.
///
/// The command's outcome is handled by [`junction_result`], split from the
/// command's execution so a test can drive both branches without running
/// `mklink`.
///
/// An exit of `0` only claims that the link exists; the test still inspects
/// it. A link that did not appear fails closed as missing, an entry that is
/// not a reparse point is refused as the wrong fixture, and an inspection
/// that fails for another reason keeps that cause, so a permission or
/// transient I/O failure is never reported as missing.
pub(super) fn try_junction_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    try_junction_dir_with(target, link, run_command_line)
}

/// The same, with the command's execution injected, so a test can prove
/// that a refusal happened before the command line was built and run.
fn try_junction_dir_with(
    target: &Path,
    link: &Path,
    run: impl FnOnce(&str) -> std::io::Result<std::process::Output>,
) -> std::io::Result<()> {
    let command = junction_command(target, link)?;
    let out = run(&command)?;
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

/// The `mklink /J` command line for `target` and `link`.
///
/// `cmd` reparses the command line itself, and `Command::arg` quotes for
/// the argv convention `cmd` does not follow: a temp path with a space or
/// a command separator could split into extra arguments. Pass one
/// pre-quoted line verbatim instead, so each path reaches `mklink` as a
/// single argument. Windows paths cannot contain `"`, so a plain quote
/// pair is enough.
///
/// Quotes do not stop `cmd`'s percent expansion, though: a `%NAME%` pair
/// is expanded when that variable is set, inside quotes as well, and the
/// pair is resolved inside the one quoted path it sits in. A path that
/// carries a percent is refused before the line exists — see
/// [`reject_percent_path`].
fn junction_command(target: &Path, link: &Path) -> Result<String, std::io::Error> {
    reject_percent_path(target)?;
    reject_percent_path(link)?;
    Ok(format!(
        "mklink /J \"{}\" \"{}\"",
        link.display(),
        target.display()
    ))
}

/// Refuse a path that `cmd` would rewrite, before any command is built.
///
/// `cmd` expands a `%NAME%` pair when that variable is set — inside
/// double quotes as well — and whether a name is set depends on the
/// environment, so a fixture path carrying one would reach `mklink` as a
/// path nobody asked for, silently.
///
/// The pair is resolved inside the one quoted argument it sits in, so
/// each path can carry one on its own: both paths are checked.
///
/// A stray percent is refused with the pairs. It cannot close a pair by
/// itself — `cmd` leaves a lone percent as it is — but which names are
/// set is the environment's business, so the rule cannot ask the
/// environment which percents matter. It refuses every percent instead:
/// the rule stays decidable from the paths alone, because a path `cmd`
/// may rewrite cannot be relied on to arrive as it was given.
///
/// The check and the command read the path the same way: the command
/// line writes it with `Path::display`, and the check searches that same
/// lossy text component by component, so a percent the check finds is a
/// percent `cmd` receives.
///
/// The refusal names the path and the reason, and it is an ordinary error
/// rather than a spawn failure: [`junction_skip_reason`] classifies only a
/// spawn failure, so this can never be read as a proven host refusal — the
/// case fails instead of skipping, and nothing was created.
fn reject_percent_path(path: &Path) -> std::io::Result<()> {
    let carries_a_percent = path
        .components()
        .any(|component| component.as_os_str().to_string_lossy().contains('%'));
    if carries_a_percent {
        return Err(std::io::Error::other(format!(
            "refusing the junction fixture path {}: cmd expands a %NAME% pair when that variable is set, even inside quotes, and the pair is resolved inside the one quoted path it sits in, so a percent in the path can leave mklink with a path other than the one given",
            path.display()
        )));
    }
    Ok(())
}

/// Run the command line `cmd` was given and hand back what it reported.
fn run_command_line(command: &str) -> std::io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;

    std::process::Command::new("cmd")
        .arg("/C")
        .raw_arg(command)
        .output()
}

/// The result of one `mklink /J` outcome.
///
/// A failing exit keeps the command's own report. A successful exit is only
/// a claim that the link exists, so the link is inspected: a link that never
/// appeared fails closed as missing, an entry that is not a reparse point is
/// refused as the wrong fixture, and an inspection that fails for another
/// reason keeps that cause, so a permission or transient I/O failure is
/// never reported as missing.
///
/// Every failure stays `Other`, so none of them can be mistaken for a proven
/// host refusal by [`junction_skip_reason`].
pub(super) fn junction_result(outcome: &JunctionOutcome, link: &Path) -> std::io::Result<()> {
    if let JunctionOutcome::Failed { status, stderr } = outcome {
        return Err(std::io::Error::other(format!(
            "mklink /J exited with {status}: {stderr}"
        )));
    }
    match std::fs::symlink_metadata(link) {
        // A junction is a name-surrogate reparse point, so std reports it as
        // a link: the link itself is what the case needs.
        Ok(md) if md.file_type().is_symlink() => Ok(()),
        // Something is there, but it is not the junction `mklink` claimed: a
        // plain directory would stand in for the fixture and satisfy a later
        // assertion for the wrong reason.
        Ok(_) => Err(junction_not_a_reparse_point_error(link)),
        Err(e) => Err(junction_inspection_error(link, e)),
    }
}

/// The failure for a link path that holds something that is not a reparse
/// point.
///
/// `mklink /J` reported success and the path exists, so the entry there is
/// not the junction the case needs: the failure names the path and says why
/// it was refused.
///
/// The failure stays `Other` and carries no raw code, so
/// [`junction_skip_reason`] cannot read it as a proven host refusal — the
/// case fails loudly instead of skipping, and a fixture that is not the
/// fixture cannot pass for one.
fn junction_not_a_reparse_point_error(link: &Path) -> std::io::Error {
    std::io::Error::other(format!(
        "mklink /J reported success but {} is not a reparse point, so it is not the directory junction the case needs",
        link.display()
    ))
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
/// is exactly what this classifier exists to expose.
///
/// The raw-code arm is defensive, not a path a junction creation takes:
/// spawning `cmd` never needs `SeCreateSymbolicLinkPrivilege`, and `cmd`'s
/// own refusals arrive as an `ExitStatus` failure that carries no raw code,
/// so a `mklink` refusal can never reach the arm. It is kept because a
/// spawn error that does carry the raw code `1314` is a host refusal, and
/// dropping the arm would turn that refusal into a panic instead of a skip.
/// Only the test that pins the classifier's contract reaches it in practice,
/// and that is what keeps it honest.
///
/// A host whose volume cannot hold a junction still fails loudly rather
/// than silently skipping.
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

    /// A successful exit that left something at the link path which is not a
    /// reparse point: a plain directory would stand in for the junction and
    /// satisfy a later assertion for the wrong reason, so the fixture must
    /// be refused instead of used.
    ///
    /// The command's execution is injected with a successful exit, so no
    /// `mklink` is run; the link path holds a plain directory, so the case
    /// runs on every host.
    #[test]
    fn a_successful_exit_that_is_not_a_reparse_point_is_refused() {
        use std::os::windows::process::ExitStatusExt;

        let dir = girpr_tempdir::TempDir::new("girpr_test_junction", "not_a_reparse_point");
        let target = dir.join("real");
        let link = dir.join("game-dir-link");
        std::fs::create_dir_all(&link).expect("the scratch directory is creatable");

        let mut ran = false;
        let err = try_junction_dir_with(&target, &link, |command| {
            ran = true;
            assert!(
                command.starts_with("mklink /J "),
                "the injected run sees the command line: {command}"
            );
            Ok(std::process::Output {
                status: std::process::ExitStatus::from_raw(0),
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        })
        .expect_err("a path that is not a reparse point must be refused");
        assert!(
            ran,
            "the command line was built and handed to the injected run"
        );

        assert_eq!(err.kind(), std::io::ErrorKind::Other, "{err}");
        assert!(err.raw_os_error().is_none(), "{err}");
        let msg = format!("{err}");
        assert!(msg.contains(&link.display().to_string()), "{msg}");
        assert!(
            msg.contains("is not a reparse point"),
            "the reason is named: {msg}"
        );
        assert!(
            junction_skip_reason(&err).is_none(),
            "a path that is not a reparse point is not a host refusal and must fail, not skip: {msg}"
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

    /// A path that carries a percent pair is refused, and the refusal is
    /// the only thing that happens: `cmd` expands a `%NAME%` pair when
    /// that variable is set — inside quotes as well — so the path
    /// `mklink` received could be a path nobody asked for, silently.
    ///
    /// The command's execution is injected, so a command that was built
    /// would be seen; and the refusal names the path and the reason, so
    /// the fixture that cannot be used says which path it was.
    #[test]
    fn a_path_with_a_percent_pair_is_refused_before_anything_is_run() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_junction", "percent_pair");
        let target = dir.join("tar%get");
        let link = dir.join("game-dir-link");

        let mut ran = false;
        let err = try_junction_dir_with(&target, &link, |command| {
            ran = true;
            Err(std::io::Error::other(format!(
                "the command was run: {command}"
            )))
        })
        .expect_err("a path with a percent pair must be refused");
        assert!(!ran, "the command must not be built or run");
        assert!(!link.exists(), "a refused path must leave no link behind");

        assert_eq!(err.kind(), std::io::ErrorKind::Other, "{err}");
        assert!(err.raw_os_error().is_none(), "{err}");
        let msg = format!("{err}");
        assert!(msg.contains(&target.display().to_string()), "{msg}");
        assert!(msg.contains("percent"), "the reason is named: {msg}");
        assert!(msg.contains("%NAME%"), "the reason is named: {msg}");
        assert!(msg.contains("cmd"), "the reason is named: {msg}");
        assert!(
            junction_skip_reason(&err).is_none(),
            "a refused path is not a host refusal and must fail, not skip: {msg}"
        );
    }

    /// A stray percent is refused with the pair, and the refusal names the
    /// path that carries it — the rule is decidable from the path alone,
    /// and a path `cmd` may rewrite cannot be relied on to arrive as it
    /// was given.
    #[test]
    fn a_stray_percent_in_the_link_path_is_refused_too() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_junction", "stray_percent");
        let target = dir.join("real");
        let link = dir.join("game-dir%link");

        let mut ran = false;
        let err = try_junction_dir_with(&target, &link, |command| {
            ran = true;
            Err(std::io::Error::other(format!(
                "the command was run: {command}"
            )))
        })
        .expect_err("a stray percent must be refused");
        assert!(!ran, "the command must not be built or run");
        assert!(!link.exists(), "a refused path must leave no link behind");

        let msg = format!("{err}");
        assert!(msg.contains(&link.display().to_string()), "{msg}");
        assert!(
            !msg.contains(&target.display().to_string()),
            "the refusal names the offending path, not the other one: {msg}"
        );
        assert!(
            junction_skip_reason(&err).is_none(),
            "a refused path is not a host refusal and must fail, not skip: {msg}"
        );
    }

    /// A path without a percent is still accepted, so the refusal is a
    /// refusal of the percent and not of the path: the command line is
    /// built, with both paths quoted as one argument each.
    #[test]
    fn a_path_without_a_percent_still_builds_the_command() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_junction", "no_percent");
        let target = dir.join("real");
        let link = dir.join("game-dir-link");

        let mut built: Option<String> = None;
        let err = try_junction_dir_with(&target, &link, |command| {
            built = Some(command.to_string());
            Err(std::io::Error::other("the command was not run to the end"))
        })
        .expect_err("the injected run fails on purpose");
        let command = built.expect("the command line was built");
        assert_eq!(
            command,
            format!("mklink /J \"{}\" \"{}\"", link.display(), target.display())
        );
        assert!(
            !command.contains('%'),
            "a path without a percent reaches the command line as it was: {command}"
        );
        assert_eq!(err.kind(), std::io::ErrorKind::Other, "{err}");
    }
}
