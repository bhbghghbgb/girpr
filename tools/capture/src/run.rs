//! Command dispatch: the four entry points a caller can invoke, and the one
//! failure summary they share.
//!
//! Each returns a [`report::Summary`] plus the [`exit::ExitCode`] it earned, so
//! the shim in `main.rs` holds no logic: it dispatches, prints, exits. Commands
//! land here as their milestone arrives — `capture` in M8, the offline trio in
//! M9 — and until then each reports the honest fact that it has nothing to do
//! yet rather than pretending to have done it.
//!
//! `capture` is the exception that arrived early: M1 needs it end to end to
//! prove the transport, and the version pair has to be resolved to name the
//! bundle's directory at all.

use std::path::PathBuf;

use crate::biz::{Biz, Endpoints};
use crate::config::{CapturePlan, MaterializePlan, VerifyPlan, artifact_dir};
use crate::exit::ExitCode;
use crate::http::{ApiError, Fetcher, RequestLog, Transport, probe};
use crate::report::{OutputFormat, Summary};
use crate::{emit, exit, hyp};

/// What a command hands back: its summary line, and the code to exit with.
///
/// The code travels with the summary rather than being returned separately so a
/// command cannot print a summary that disagrees with the status it exits with.
pub type Outcome = (Summary, ExitCode);

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
}

impl Failure {
    /// A failure discovered by validating the request. Still exit `1`: the
    /// request was wrong, and nothing about it was the network's fault.
    pub fn usage(message: impl std::fmt::Display) -> Self {
        Failure {
            code: ExitCode::Usage,
            detail: message.to_string(),
        }
    }

    /// A failure of the network or the API. Always exit `2`.
    pub fn api(what: &str, e: &ApiError) -> Self {
        Failure {
            code: ExitCode::Network,
            detail: format!("{what}: {e}"),
        }
    }

    /// The summary a caller sees for this failure.
    pub fn summary(&self, event: &'static str) -> Summary {
        failure_summary_detail(event, self.code, Some(&self.detail))
    }
}

/// `gircap capture` — fetch from the live APIs and write a bundle.
///
/// **M2.** Writes the seven `api/` documents: the four HoYoPlay bodies and both
/// `getBuild`s byte-exact, plus the generated tag probe, and
/// `api/requests.ndjson` beside them. Manifests, chunks and the `game/` fixture
/// land in M3, M6 and M8.
///
/// The order is [`hyp::HypClient::capture_all`]'s, not this function's:
/// `getGameBranches` carries the `package_id` every `getBuild` URL is built from,
/// so nothing after it can be requested before it.
///
/// The bundle's directory is `<biz>/<old>__<new>/<id>/`, which is why the pair
/// is resolved before anything is written rather than after.
pub async fn capture(plan: CapturePlan) -> Result<Outcome, Failure> {
    let started = chrono::Utc::now();
    let biz = Biz::resolve(plan.biz);
    let log = RequestLog::new();
    let fetcher = Fetcher::new(
        Transport {
            jobs: plan.jobs,
            retries: plan.retries,
            timeout: plan.timeout,
        },
        log.clone(),
    )
    .map_err(|e| apierr("build the http client", &e))?;

    // A testing seam, and the only thing that redirects a capture: the ids still
    // come from the `--biz` mapping, so a mock run sends exactly what a live one
    // sends.
    let endpoints = if plan.hyp_base.is_none() && plan.sophon_base.is_none() {
        biz.bases()
    } else {
        Endpoints::owned(
            plan.hyp_base.clone().unwrap_or_else(|| biz.hyp_base().to_string()),
            plan.sophon_base
                .clone()
                .unwrap_or_else(|| biz.sophon_base().to_string()),
        )
    };
    tracing::info!(
        hyp_base = endpoints.hyp,
        sophon_base = endpoints.sophon,
        override_in_play = plan.hyp_base.is_some() || plan.sophon_base.is_some(),
        "endpoints resolved"
    );

    // 1-4. All six metadata bodies, byte-exact, and the version pair.
    //
    // `capture_all` owns the ordering because it has to: `getGameBranches`
    // carries the `package_id` every `getBuild` URL is built from, so a caller
    // that fetched them out of order would request a URL with no package in it.
    let client = hyp::HypClient::new(endpoints.clone(), biz, &fetcher);
    let captured = client
        .capture_all(plan.new_selector(), plan.old_selector())
        .await
        .map_err(|e| apierr("metadata capture", &e))?;

    // Assert what the capture depends on, before a blob is fetched: the launcher
    // must be on chunk delivery, both builds must echo the tags they were asked
    // for, and every published manifest must be storable.
    captured.validate().map_err(|e| assertion("validate the capture", &e))?;

    let versions = &captured.versions;
    let params = captured
        .branch_params()
        .map_err(|e| apierr("re-read the captured branch", &e))?;
    tracing::info!(
        biz = biz.as_str(),
        package_id = %params.package_id,
        old_tag = %versions.old.tag,
        old_source = %versions.old.source,
        old_build_id = %versions.old.build_id,
        new_tag = %versions.new.tag,
        new_source = %versions.new.source,
        new_build_id = %versions.new.build_id,
        diff_tags = ?versions.diff_tags,
        "version pair resolved"
    );
    let (new_tag, old_tag) = (versions.new.tag.clone(), versions.old.tag.clone());

    // 5. Which tags the API will serve at all. Cheap, and the fact that decides
    //    whether an old file is reconstructible today.
    let tags = probe::compose_tags(
        &[old_tag.clone(), new_tag.clone()],
        &params.diff_tags,
        &plan.probe_tags,
    );
    // The pair's `getBuild` was already fetched, so the probe does not pay for it
    // twice. This is the largest avoidable cost in a metadata-only run.
    let known = vec![
        (old_tag.clone(), versions.old.build_id.clone()),
        (new_tag.clone(), versions.new.build_id.clone()),
    ];
    let probe = probe::probe_tags(
        &fetcher,
        endpoints.sophon,
        &params,
        &tags,
        &params.diff_tags,
        &known,
    )
        .await
        .map_err(|e| apierr("tag probe", &e))?;
    tracing::info!(probe = %probe.describe(), served = ?probe.served(), "tag probe complete");

    // 6. The tree. Both writes happen even though the run is incomplete: what
    //    was asked, and what it cost, is the part that stays useful, and the
    //    bodies are the evidence behind every number in the summary.
    let dir = artifact_dir(&plan.out, plan.biz, &old_tag, &new_tag, started);
    // `write_api` returns the relative paths it wrote, so the count in the
    // summary is the count on disk rather than a second bookkeeping total.
    let written = emit::write_api(&dir, &captured, &probe)
        .map_err(|e| {
            apierr(
                &format!("write {}", dir.display()),
                &ApiError::shape("artifact writer", e),
            )
        })?;
    emit::write_requests_log(&dir, &log).map_err(|e| {
        apierr(
            &format!("write {}", dir.display()),
            &ApiError::shape("artifact writer", e),
        )
    })?;
    tracing::info!(dir = %dir.display(), records = log.len(), documents = written.len(), "api/ written");

    let legacy_behind = captured
        .legacy_staleness()
        .map(|(n, _)| n)
        .unwrap_or(0);
    let summary = Summary::new("capture")
        .set("result", "ok")
        .set("stage", "m2-metadata")
        .set("biz", biz.as_str())
        .set("old_tag", &old_tag)
        .set("new_tag", &new_tag)
        .set("package_id", &params.package_id)
        .set("old_build_id", &versions.old.build_id)
        .set("new_build_id", &versions.new.build_id)
        .set("artifact", dir.as_path())
        .set("api_documents", written.len() as u64)
        .set("requests", log.len() as u64)
        .set("bytes_downloaded", log.bytes_in())
        .set("tags_probed", probe.summary.requested as u64)
        .set("tags_served", probe.summary.served as u64)
        .set("tags_served_outside_diff_tags", probe.summary.not_in_diff_tags_but_served as u64)
        .set("legacy_endpoint_majors_behind", legacy_behind)
        .set("exit_code", ExitCode::Ok.code());

    Ok((summary, ExitCode::Ok))
}

/// `gircap verify` — re-check a bundle from its stored bytes, no network.
pub async fn verify(_plan: VerifyPlan) -> Result<Outcome, Failure> {
    tracing::warn!("verify is not implemented yet");
    Err(Failure { code: ExitCode::AssertionFailed, detail: "verify is not implemented yet".into() })
}

/// `gircap materialize` — rebuild both files from `chunks/`, no network.
pub async fn materialize(_plan: MaterializePlan) -> Result<Outcome, Failure> {
    tracing::warn!("materialize is not implemented yet");
    Err(Failure { code: ExitCode::AssertionFailed, detail: "materialize is not implemented yet".into() })
}

/// `gircap list` — index every capture under a root.
pub async fn list(_root: PathBuf, _output: OutputFormat) -> Result<Outcome, Failure> {
    tracing::warn!("list is not implemented yet");
    Err(Failure { code: ExitCode::AssertionFailed, detail: "list is not implemented yet".into() })
}

/// Turn an [`ApiError`] into exit `2`, having named it on stderr.
fn apierr(what: &str, e: &ApiError) -> Failure {
    tracing::error!(subject = what, error = %e, url = e.url().unwrap_or(""), "api failure");
    Failure::api(what, e)
}

/// See [`Failure::usage`]; this adds the narration, so the dispatch in `main.rs`
/// has one place to report a rejected request rather than three.
pub fn usage(what: &str, message: &str) -> Failure {
    tracing::error!(subject = what, error = %message, "usage error");
    Failure::usage(message)
}

/// An assertion on captured API state: the answer arrived and it says something
/// this capture cannot be built from.
///
/// Exit `3`, not `2`. The request worked — a `2` would blame the network for a
/// fact about the world — and not `1`, because the request was not wrong either.
/// What is wrong is the artifact, and `3` is the code that says so.
fn assertion(what: &str, e: &ApiError) -> Failure {
    tracing::error!(subject = what, error = %e, "assertion failed");
    Failure {
        code: ExitCode::AssertionFailed,
        detail: format!("{what}: {e}"),
    }
}

/// The summary a command prints when it failed before it could build one.
///
/// Every command prints a summary on every path, so a harness that only reads
/// stdout never has to distinguish "the tool said nothing" from "the tool
/// succeeded quietly". The fields are the ones that are true of *any* failure:
/// which command, which code, what it meant, and — for the ones that leave a
/// tree behind — whether that tree can be believed.
pub fn failure_summary(event: &'static str, code: ExitCode) -> Summary {
    failure_summary_detail(event, code, None)
}

/// [`failure_summary`] with the one-line cause, when there is one.
///
/// The cause is a field rather than prose, so the whole line stays
/// whitespace-free and splittable.
pub fn failure_summary_detail(
    event: &'static str,
    code: ExitCode,
    detail: Option<&str>,
) -> Summary {
    let s = Summary::new(event)
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

/// Re-exported so `main.rs` can name the process exit without importing two
/// modules for it.
pub use exit::exit_with;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Val;

    /// An assertion on captured API state is exit 3, and says why: neither the
    /// network nor the request is at fault.
    #[test]
    fn an_assertion_failure_is_three_and_is_not_mistaken_for_a_network_error() {
        let f = assertion("validate the capture", &ApiError::shape("x", "y"));
        assert_eq!(f.code, ExitCode::AssertionFailed);
        assert!(f.detail.contains("validate the capture"), "{}", f.detail);
        assert!(f.detail.contains("y"), "{}", f.detail);
        assert_eq!(
            f.summary("capture").get("exit_code"),
            Some(&Val::Int(3)),
            "not 2: the request worked and said something unusable"
        );
    }

    #[test]
    fn a_usage_failure_is_one_and_carries_the_message() {
        let f = usage("--jobs", "--jobs must be at least 1");
        assert_eq!(f.code, ExitCode::Usage);
        assert_eq!(
            f.summary("capture").get("detail"),
            Some(&Val::Str("--jobs must be at least 1".into()))
        );
    }

    #[test]
    fn a_failure_summary_names_the_code_and_says_whether_the_tree_can_be_believed() {
        let s = failure_summary("capture", ExitCode::Network);
        assert_eq!(s.get("exit_code"), Some(&crate::report::Val::Int(2)));
        assert_eq!(s.get("exit_code_slug"), Some(&crate::report::Val::Str("network".into())));
        assert_eq!(s.get("result"), Some(&crate::report::Val::Str("error".into())));
        assert_eq!(s.get("detail"), None);

        let d = failure_summary_detail("capture", ExitCode::AssertionFailed, Some("boom"));
        assert_eq!(d.get("detail"), Some(&crate::report::Val::Str("boom".into())));
        assert_eq!(
            d.get("artifact_trustworthy"),
            Some(&crate::report::Val::Bool(false)),
            "an assertion failure leaves a tree that must be read, not deleted"
        );
    }

    #[test]
    fn the_detail_survives_the_text_rendering_as_one_token() {
        let line = failure_summary_detail("capture", ExitCode::Network, Some("getBuild: retcode -202 (not found)"))
            .to_text_line();
        assert_eq!(
            line.split_whitespace().filter(|t| t.contains('=')).count(),
            line.split_whitespace().count() - 1,
            "a spaced detail would break tokenisation: {line}"
        );
        assert!(line.contains("retcode%20-202"), "{line}");
    }
}
