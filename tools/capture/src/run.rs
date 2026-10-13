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

use crate::biz::{BranchParams, Biz, Endpoints, HypEndpoint};
use crate::config::{CapturePlan, MaterializePlan, VerifyPlan, artifact_dir};
use crate::exit::ExitCode;
use crate::http::{ApiError, Fetcher, Kind, RequestLog, Transport, probe};
use crate::report::{OutputFormat, Summary};
use crate::{emit, exit};

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

/// The language every HoYoPlay endpoint is asked in.
///
/// `gircap` captures bytes, not locales, and the responses are byte-exact either
/// way. `en-us` is the international launcher's own default, so a capture made
/// with `--biz hk4e_global` is a capture of what that launcher actually asks for.
const CAPTURE_LANGUAGE: &str = "en-us";

/// `gircap capture` — fetch from the live APIs and write a bundle.
///
/// **M1 slice.** The transport is exercised end to end: both HoYoPlay calls, the
/// version pair, both `getBuild`s, and the tag probe. It writes
/// `api/requests.ndjson` and stops there — the response *bodies*, the manifests,
/// the chunks and the fixture tree land in M2, M3, M6 and M8.
///
/// The version pair is resolved here rather than in M2 for one reason: the
/// bundle's directory is `<biz>/<old>__<new>/<id>/`, so a capture that wrote a
/// log before it knew the pair would have to move it afterwards. Resolving two
/// strings early is cheaper than a rename later, and the full byte-exact capture
/// of both bodies is still M2's job.
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

    // 1. Launcher configuration. Cheap, and the source of the fixture's paths.
    let configs_url = endpoints.hyp_url(&biz, HypEndpoint::GameConfigs, CAPTURE_LANGUAGE);
    let (_configs_body, _configs) = fetcher
        .get_json(&configs_url, Kind::Api)
        .await
        .map_err(|e| apierr("getGameConfigs", &e))?;

    // 2. Branch parameters. This is what replaces a hardcoded package_id.
    let branches_url = endpoints.hyp_url(&biz, HypEndpoint::GameBranches, CAPTURE_LANGUAGE);
    let (branches_body, _branches) = fetcher
        .get_json(&branches_url, Kind::Api)
        .await
        .map_err(|e| apierr("getGameBranches", &e))?;
    let params = BranchParams::from_branches(&branches_body).map_err(|e| apierr("getGameBranches", &e))?;
    tracing::info!(
        biz = biz.as_str(),
        package_id = %params.package_id,
        branch_tag = %params.tag,
        diff_tags = ?params.diff_tags,
        "branch resolved"
    );

    // 3. The pair. `latest` is resolved *here*, against a live answer, which is
    //    the whole point of making it a flag value rather than a lookup.
    let new_tag = resolve_new_tag(&plan, &params).map_err(|m| usage("resolve --new-tag", &m))?;
    let old_tag = resolve_old_tag(&plan, &params)
        .map_err(|m| apierr("resolve --old-tag", &ApiError::shape("diff_tags", &m)))?;
    tracing::info!(%old_tag, %new_tag, "version pair resolved");

    // 4. Both builds. An unservable tag is exit 2 here, not a silent fallback:
    //    a capture that quietly patched 7.1.0 to 7.1.0 would be worse than none.
    let new_build = get_build(&fetcher, &endpoints, &params, &new_tag).await?;
    let old_build = get_build(&fetcher, &endpoints, &params, &old_tag).await?;

    // 5. Which tags the API will serve at all. Cheap, and the fact that decides
    //    whether an old file is reconstructible today.
    let tags = probe::compose_tags(
        &[old_tag.clone(), new_tag.clone()],
        &params.diff_tags,
        &plan.probe_tags,
    );
    // The pair's `getBuild` was already fetched, so the probe does not pay for it
    // twice. This is the largest avoidable cost in a metadata-only run: three
    // extra requests out of seven, for answers already in hand.
    let known = vec![
        (old_tag.clone(), build_id(&old_build)),
        (new_tag.clone(), build_id(&new_build)),
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

    // 6. The provenance log. Written even though the run is incomplete: what was
    //    asked, and what it cost, is the part that stays useful.
    let dir = artifact_dir(&plan.out, plan.biz, &old_tag, &new_tag, started);
    let written = emit::write_requests_log(&dir, &log).map_err(|e| {
        apierr(
            &format!("write {}", dir.display()),
            &ApiError::shape("artifact writer", e),
        )
    })?;
    tracing::info!(dir = %dir.display(), records = written, "request log written");

    let summary = Summary::new("capture")
        .set("result", "ok")
        .set("stage", "m1-transport")
        .set("biz", biz.as_str())
        .set("old_tag", &old_tag)
        .set("new_tag", &new_tag)
        .set("package_id", &params.package_id)
        .set("old_build_id", build_id(&old_build))
        .set("new_build_id", build_id(&new_build))
        .set("artifact", dir.as_path())
        .set("requests", log.len() as u64)
        .set("bytes_downloaded", log.bytes_in())
        .set("tags_probed", probe.summary.requested as u64)
        .set("tags_served", probe.summary.served as u64)
        .set("tags_served_outside_diff_tags", probe.summary.not_in_diff_tags_but_served as u64)
        .set("exit_code", ExitCode::Ok.code());

    Ok((summary, ExitCode::Ok))
}

/// Fetch one `getBuild` and return its `data`.
///
/// A `retcode != 0` — `-202` for a tag the API does not serve — is raised as a
/// shape error, which is exit `2`. That is the documented acceptance case: a
/// deliberately bad tag must fail the run, not fall back to something servable.
async fn get_build(
    fetcher: &Fetcher,
    endpoints: &Endpoints<'_>,
    params: &BranchParams,
    tag: &str,
) -> Result<serde_json::Value, Failure> {
    let url = endpoints.get_build_url(params, Some(tag));
    let (_body, v) = fetcher
        .get_json(&url, Kind::Api)
        .await
        .map_err(|e| apierr(&format!("getBuild tag={tag}"), &e))?;
    v.get("data")
        .cloned()
        .filter(|d| !d.is_null())
        .ok_or_else(|| {
            let rc = v.get("retcode").and_then(|r| r.as_i64()).unwrap_or(-1);
            apierr(
                &format!("getBuild tag={tag}"),
                &ApiError::shape(
                    "getBuild",
                    format!("retcode {rc} with no data; this tag is not servable"),
                ),
            )
        })
}

/// `data.build_id`, or `null` — the thing a chunk URL is namespaced by.
fn build_id(data: &serde_json::Value) -> String {
    data.get("build_id")
        .and_then(|b| b.as_str())
        .unwrap_or("none")
        .to_string()
}

/// `--new-tag`, with `latest` (and its absence) resolved against the branch.
fn resolve_new_tag(plan: &CapturePlan, params: &BranchParams) -> Result<String, String> {
    match plan.new_tag.as_deref() {
        None => Ok(params.tag.clone()),
        Some(t) if t.eq_ignore_ascii_case(crate::cli::NEW_TAG_LATEST) => Ok(params.tag.clone()),
        Some(t) => Ok(t.to_string()),
    }
}

/// `--old-tag`, defaulting to `diff_tags[0]` — "current minus one".
///
/// `diff_tags` is a *patch-support* list, not a retention list, so this default
/// is the newest patch the vendor still offers rather than the newest version the
/// API can serve. `--old-tag` is how a caller asks for the latter, and
/// `--probe-tags` is how they find out whether it is there.
fn resolve_old_tag(plan: &CapturePlan, params: &BranchParams) -> Result<String, String> {
    if let Some(t) = plan.old_tag.as_deref() {
        return Ok(t.to_string());
    }
    params
        .diff_tags
        .first()
        .cloned()
        .ok_or_else(|| {
            "the branch lists no diff_tags, so there is no default old version; \
             pass --old-tag with a tag the API serves (see `gircap capture \
             --probe-tags`, or spec section 3.4)"
                .to_string()
        })
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

/// See [`Failure::usage`]; this adds the narration.
fn usage(what: &str, message: &str) -> Failure {
    tracing::error!(subject = what, error = %message, "usage error");
    Failure::usage(message)
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
    use crate::cli::Biz as BizId;

    fn params() -> BranchParams {
        BranchParams {
            package_id: "p".into(),
            password: "w".into(),
            branch: "main".into(),
            tag: "7.1.0".into(),
            diff_tags: vec!["7.0.0".into(), "6.7.0".into()],
        }
    }

    fn plan_with(new_tag: Option<&str>, old_tag: Option<&str>) -> CapturePlan {
        CapturePlan {
            biz: BizId::Hk4eGlobal,
            out: PathBuf::from("artifacts"),
            new_tag: new_tag.map(str::to_string),
            old_tag: old_tag.map(str::to_string),
            per_class: 1,
            max_download_bytes: None,
            require_class: vec![],
            files: vec![],
            audio: crate::config::AudioSelection::Codes(vec!["en-us".into()]),
            manifest_detail: crate::cli::ManifestDetail::Full,
            gzip_threshold: 1_048_576,
            probe_tags: vec![],
            no_chunk_blobs: false,
            jobs: 4,
            retries: 3,
            timeout: std::time::Duration::from_secs(60),
            hyp_base: None,
            sophon_base: None,
            output: OutputFormat::Text,
            argv: vec![],
        }
    }

    /// The default is `diff_tags[0]` — "current minus one" — measured on
    /// 2026-10-14 as 7.1.0 -> 7.0.0.
    #[test]
    fn the_pair_defaults_to_the_branch_head_and_its_first_diff_tag() {
        let p = params();
        assert_eq!(
            resolve_new_tag(&plan_with(None, None), &p).unwrap(),
            "7.1.0"
        );
        assert_eq!(resolve_old_tag(&plan_with(None, None), &p).unwrap(), "7.0.0");
    }

    #[test]
    fn the_latest_sentinel_resolves_against_the_branch_not_the_flag() {
        let p = params();
        assert_eq!(
            resolve_new_tag(&plan_with(Some("latest"), None), &p).unwrap(),
            "7.1.0"
        );
        assert_eq!(
            resolve_new_tag(&plan_with(Some("LATEST"), None), &p).unwrap(),
            "7.1.0",
            "the sentinel is case-insensitive"
        );
        assert_eq!(
            resolve_new_tag(&plan_with(Some("6.6.0"), None), &p).unwrap(),
            "6.6.0",
            "an explicit tag is taken as written; whether it is servable is \
             getBuild's answer, not this function's"
        );
    }

    /// A branch with no `diff_tags` cannot supply a default old version, and
    /// inventing one would mean capturing the wrong version pair.
    #[test]
    fn an_empty_diff_tags_is_an_error_naming_the_remedy() {
        let mut p = params();
        p.diff_tags.clear();
        let e = resolve_old_tag(&plan_with(None, None), &p).unwrap_err();
        assert!(e.contains("--old-tag"), "{e}");
        assert!(e.contains("diff_tags"), "{e}");
        // An explicit tag still works: the caller asked, and getBuild will say.
        assert_eq!(resolve_old_tag(&plan_with(None, Some("5.5.0")), &p).unwrap(), "5.5.0");
    }

    #[test]
    fn build_ids_are_read_or_reported_as_absent() {
        assert_eq!(
            build_id(&serde_json::json!({"build_id": "sMXGW2ll3Fuu"})),
            "sMXGW2ll3Fuu"
        );
        assert_eq!(build_id(&serde_json::json!({})), "none");
        assert_eq!(build_id(&serde_json::json!({"build_id": 7})), "none");
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