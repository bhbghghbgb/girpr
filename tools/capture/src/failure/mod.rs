//! How a command says it failed: the type, the constructors, and the summary.
//!
//! **One module, because there were three places to edit to add an exit code.**
//! `Failure::usage` and `Failure::api` were methods on the type, `assertion` was
//! a free function that duplicated `Failure::api`'s struct literal with a
//! different code, and `failure_summary_detail` built the line those fields
//! travel in. A fourth exit code meant editing all three.
//!
//! It also means the type is not [`crate::run`]'s. `manifest`, `emit::selfcheck`
//! and `blobs` all have things to say on failure, and while the error type lived
//! in `run` they could not say it without depending on the command dispatcher —
//! which is most of why `verify_manifest` (49 lines) and
//! `closing_assertions` (173 lines) sat in `run.rs` rather than in the modules
//! that own them.
//!
//! The constructors are `pub` and re-exported from [`crate::run`], so
//! `run::usage` and `run::failure_summary` still resolve: moving a type is not a
//! reason to touch every call site.

use crate::exit::ExitCode;
use crate::http::ApiError;
use crate::report::Record;

/// What a command hands back: its summary line, and the code to exit with.
///
/// The code travels with the summary rather than being returned separately so a
/// command cannot print a summary that disagrees with the status it exits with.
pub type Outcome = (Record, ExitCode);

/// Why a command failed, in a form the summary can carry.
///
/// A code alone is not an answer. Exit `1` and exit `2` are both "something was
/// wrong" and the operator needs to know what — so the cause rides out on stdout
/// as a `detail=` field instead of living only in the stderr narration that a
/// `--output json` pipeline is not reading.
#[derive(Debug, Clone)]
pub struct Failure {
    /// What to exit with.
    pub code: ExitCode,
    /// The one-line cause.
    pub detail: String,
    /// A record to print **before** this failure's own summary.
    ///
    /// A run can fail late enough to have measured everything. A capture that
    /// writes its bundle and then finds a row that did not hold knows how many
    /// captures it admitted, what they cost and how many classes it satisfied —
    /// and none of that is in `detail`, which is one line about one assertion.
    ///
    /// Without this the numbers are simply gone: a failing run printed a
    /// failure summary in place of the capture summary, so `budget_limit`,
    /// `captures_over_budget` and `capture_bytes_admitted` were on neither
    /// stdout nor anywhere in the bundle. The bundle is the evidence about the
    /// *artifact*; these are the numbers about the *run*, and a caller who set a
    /// budget it could not afford wants them most precisely when it failed.
    ///
    /// Carried on the failure rather than emitted where it is built, because
    /// the command does not know the output format - `--output` is resolved in
    /// the shim, above every command.
    pub preceded_by: Option<crate::report::Record>,
}

impl std::fmt::Display for Failure {
    /// The one-line cause, on its own.
    ///
    /// The code is deliberately *not* in the output: `detail` is a field of a
    /// `SUMMARY` line and is consumed there, so printing a `Failure` in a log or
    /// an assertion message should not invent a second, differently-shaped
    /// spelling of the same thing.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}

impl Failure {
    /// A failure discovered by validating the request. Still exit `1`: the
    /// request was wrong, and nothing about it was the network's fault.
    pub fn usage(message: impl std::fmt::Display) -> Self {
        Failure {
            code: ExitCode::Usage,
            detail: message.to_string(),
            preceded_by: None,
        }
    }

    /// A failure of the network or the API. Always exit `2`.
    pub fn api(what: &str, e: &ApiError) -> Self {
        Failure {
            code: ExitCode::Network,
            detail: format!("{what}: {e}"),
            preceded_by: None,
        }
    }

    /// An assertion on captured API state: the answer arrived and it says
    /// something this capture cannot be built from.
    ///
    /// Exit `3`, not `2`. The request worked — a `2` would blame the network for
    /// a fact about the world — and not `1`, because the request was not wrong
    /// either. What is wrong is the artifact, and `3` is the code that says so.
    pub fn assertion(what: &str, e: &ApiError) -> Self {
        Failure {
            code: ExitCode::AssertionFailed,
            detail: format!("{what}: {e}"),
            preceded_by: None,
        }
    }

    /// Attach a record to print before this failure's own summary.
    ///
    /// See [Failure::preceded_by]. A builder rather than a field so that the
    /// common case - a failure with no run to report - is spelled once.
    pub fn preceded_by(mut self, record: Record) -> Self {
        self.preceded_by = Some(record);
        self
    }

    /// The summary a caller sees for this failure.
    pub fn summary(&self, event: &'static str) -> Record {
        failure_summary_detail(event, self.code, Some(&self.detail))
    }
}

/// Turn an [`ApiError`] into exit `2`, having named it on stderr.
pub fn apierr(what: &str, e: &ApiError) -> Failure {
    tracing::error!(subject = what, error = %e, url = e.url().unwrap_or(""), "api failure");
    Failure::api(what, e)
}

/// See [`Failure::usage`]; this adds the narration, so the dispatch in `main.rs`
/// has one place to report a rejected request rather than three.
pub fn usage(what: &str, message: &str) -> Failure {
    tracing::error!(subject = what, error = %message, "usage error");
    Failure::usage(message)
}

/// See [`Failure::assertion`]; this adds the narration, so every assertion in the
/// crate reports itself the same way rather than each call site remembering to.
pub fn assertion(what: &str, e: &ApiError) -> Failure {
    tracing::error!(subject = what, error = %e, "assertion failed");
    Failure::assertion(what, e)
}

/// Turn a writer's `io::Error` into exit `2` naming the directory.
///
/// Every write in the close-out goes through here so the refusal says *where*
/// the write failed; a bare "No such file or directory" from a 90 MB capture is
/// not an actionable report.
///
/// Exit `2`, not `3` — a write that fails mid-capture is treated as a failure of
/// the path we were writing to, and that is the code this has always returned.
pub fn write_err(dir: &std::path::Path, e: std::io::Error) -> Failure {
    apierr(
        &format!("write {}", dir.display()),
        &ApiError::shape("artifact writer", e),
    )
}

/// The summary a command prints when it failed before it could build one.
///
/// Every command prints a summary on every path, so a harness that only reads
/// stdout never has to distinguish "the tool said nothing" from "the tool
/// succeeded quietly". The fields are the ones that are true of *any* failure:
/// which command, which code, what it meant, and — for the ones that leave a
/// tree behind — whether that tree can be believed.
pub fn failure_summary(event: &'static str, code: ExitCode) -> Record {
    failure_summary_detail(event, code, None)
}

/// [`failure_summary`] with the one-line cause, when there is one.
///
/// The cause is a field rather than prose, and fields are separated by spaces
/// while **values are percent-escaped** — so a space inside
/// `exit_code_meaning=usage / configuration error` arrives as `%20` and a
/// harness can split the line on `' '` without quoting rules. (This said
/// "whitespace-free", which was never true: the separator *is* a space.)
pub fn failure_summary_detail(event: &'static str, code: ExitCode, detail: Option<&str>) -> Record {
    let s = Record::summary(event)
        .set("result", "error")
        .set("exit_code", code.code())
        .set("exit_code_slug", code.slug())
        .set("exit_code_meaning", code.describe())
        .set("artifact_trustworthy", !code.artifact_is_untrustworthy());
    match detail {
        Some(d) => s.set("detail", d),
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every failure a command can hand back must produce one summary line, and
    /// that line must say the same things whatever the code.
    ///
    /// This is the test that makes "one module" worth having: the three
    /// constructors are checked against one shape, so a fourth exit code cannot
    /// be added in one place and forgotten in another.
    #[test]
    fn every_failure_constructor_agrees_on_the_summary_shape() {
        let api = ApiError::shape("getBuild", "connection reset");
        let cases: Vec<(&str, Failure)> = vec![
            ("usage", usage("capture", "--jobs must be at least 1")),
            ("api", apierr("metadata capture", &api)),
            (
                "assertion",
                assertion("verify the 7.0.0 game manifest", &api),
            ),
            (
                "write",
                write_err(
                    std::path::Path::new("/tmp/x"),
                    std::io::Error::other("disk full"),
                ),
            ),
        ];
        let expected = [
            ExitCode::Usage,
            ExitCode::Network,
            ExitCode::AssertionFailed,
            // A write failure is exit 2, which is what it has always been.
            ExitCode::Network,
        ];
        for ((name, f), code) in cases.iter().zip(expected) {
            assert_eq!(f.code, code, "{name}");
            assert!(!f.detail.is_empty(), "{name} carries a cause");
            let line = f.summary("capture");
            let text = line.to_text_line();
            // The line terminator is not part of the claim.
            let text = text.trim_end();
            for field in [
                "event=capture",
                "result=error",
                &format!("exit_code={}", code.code()),
                &format!("exit_code_slug={}", code.slug()),
                "exit_code_meaning=",
                "artifact_trustworthy=",
                "detail=",
            ] {
                assert!(text.contains(field), "{name}: {field} missing from {text}");
            }
            // Splittable on a space: the separator is the format, and every
            // *value* is percent-escaped, so no value contains a space and a
            // harness can `split(' ')` without quoting rules.
            for token in text.split(' ').skip(1) {
                assert!(
                    token.contains('='),
                    "{name}: {token} is not a key=value field in {text}"
                );
                let value = token.split_once('=').expect("checked above").1;
                assert!(
                    !value.contains(' '),
                    "{name}: value {value} contains a raw space in {text}"
                );
            }
        }
    }
}
