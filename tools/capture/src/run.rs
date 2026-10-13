//! Command dispatch: the four entry points a caller can invoke, and the one
//! failure summary they share.
//!
//! Each returns a [`report::Summary`] plus the [`exit::ExitCode`] it earned, so
//! the shim in `main.rs` holds no logic: it dispatches, prints, exits. Commands
//! land here as their milestone arrives — `capture` in M8, the offline trio in
//! M9 — and until then each reports the honest fact that it has nothing to do
//! yet rather than pretending to have done it.

use std::path::PathBuf;

use crate::config::{CapturePlan, MaterializePlan, VerifyPlan};
use crate::exit::ExitCode;
use crate::report::{OutputFormat, Summary};

/// What a command hands back: its summary line, and the code to exit with.
///
/// The code travels with the summary rather than being returned separately so a
/// command cannot print a summary that disagrees with the status it exits with.
pub type Outcome = (Summary, ExitCode);

/// `gircap capture` — fetch from the live APIs and write a bundle.
pub async fn capture(_plan: CapturePlan) -> Result<Outcome, ExitCode> {
    tracing::warn!("capture is not implemented yet");
    Err(ExitCode::AssertionFailed)
}

/// `gircap verify` — re-check a bundle from its stored bytes, no network.
pub async fn verify(_plan: VerifyPlan) -> Result<Outcome, ExitCode> {
    tracing::warn!("verify is not implemented yet");
    Err(ExitCode::AssertionFailed)
}

/// `gircap materialize` — rebuild both files from `chunks/`, no network.
pub async fn materialize(_plan: MaterializePlan) -> Result<Outcome, ExitCode> {
    tracing::warn!("materialize is not implemented yet");
    Err(ExitCode::AssertionFailed)
}

/// `gircap list` — index every capture under a root.
pub async fn list(_root: PathBuf, _output: OutputFormat) -> Result<Outcome, ExitCode> {
    tracing::warn!("list is not implemented yet");
    Err(ExitCode::AssertionFailed)
}

/// The summary a command prints when it failed before it could build one.
///
/// Every command prints a summary on every path, so a harness that only reads
/// stdout never has to distinguish "the tool said nothing" from "the tool
/// succeeded quietly". The fields are the ones that are true of *any* failure:
/// which command, which code, and what it meant.
pub fn failure_summary(event: &'static str, code: ExitCode) -> Summary {
    Summary::new(event)
        .set("result", "error")
        .set("exit_code", code.code())
        .set("exit_code_slug", code.slug())
        .set("exit_code_meaning", code.describe())
        .set("artifact_trustworthy", !code.artifact_is_untrustworthy())
}