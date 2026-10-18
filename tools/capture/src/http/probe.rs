//! `getBuild?tag=` probing: which version tags the API will actually serve, and
//! what the API does with a request it will not serve.
//!
//! `diff_tags` is a patch-support list, not a retention list. Measured
//! 2026-10-14: `6.6.0` and `5.5.0` are absent from `diff_tags` and both served.
//! The difference decides whether an old-version file is reconstructible at all
//! today, without the legacy zip path - and it is the one measurement in this
//! tool that changes a *design conclusion* rather than a number in a table.
//!
//! A probe costs one small JSON request per tag and downloads no blob, so it is
//! cheap enough to run by default and is worth recording every time.
//!
//! **A `retcode -202` here is an answer, not a failure.** It means "this API does
//! not serve that tag", which is the question. Treating it as an error would make
//! the probe unable to report its own most interesting result.
//!
//! ## The two route families disagree about failure
//!
//! [`ErrorShapes`] records that disagreement as bundle content rather than leaving
//! it in a review. The blob routes answer `404` with an AliyunOSS `NoSuchKey`
//! body; the JSON routes answer `200` and put the failure in `retcode`. A tool
//! that replays a bundle has to answer both correctly, and without a document
//! recording them it has to invent them — and then its claim that "the API 404s
//! things" is unfounded. One of those answers is **not** a failure at all
//! (`getGameBranches` with an unknown game id answers `retcode 0` with an empty
//! node), which is the specific mistake this document exists to prevent.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::biz::BranchParams;
use crate::http::{ApiError, Fetcher, Kind, Observation};

/// The probe report: `api/tag_probe.json`.
#[derive(Debug, Clone, Serialize)]
pub struct TagProbe {
    /// The bundle schema version.
    pub schema_version: u32,
    /// The tags asked about, in the order they were asked.
    pub requested: Vec<String>,
    /// One row per tag, in the same order.
    pub results: Vec<TagProbeRow>,
    /// The counts a reader wants first.
    pub summary: ProbeSummary,
}

/// One tag's answer.
#[derive(Debug, Clone, Serialize)]
pub struct TagProbeRow {
    /// The tag asked about.
    pub tag: String,
    /// Whether the branch's `diff_tags` lists it.
    pub in_diff_tags: bool,
    /// HTTP status. `200` even for an unserved tag.
    pub http: u16,
    /// The API's own code. `-202` means not served.
    pub retcode: i64,
    /// `data.build_id`, when served.
    pub build_id: Option<String>,
    /// `data.tag` as the API echoed it — it may differ from what was asked.
    pub tag_echoed: Option<String>,
    /// The load-bearing fact: did the API produce a build for this tag?
    pub served: bool,
    /// Whether the answer was reused from a `getBuild` this run already made.
    ///
    /// Purely provenance: it changes no number in the document, and it is what
    /// makes the request log reconcilable with the probe — a row that says
    /// `true` must *not* have a matching `kind: "probe"` entry.
    pub already_fetched: bool,
}

/// Whether `diff_tags` lists `tag`.
fn in_diff_tags(diff_tags: &[String], tag: &str) -> bool {
    diff_tags.iter().any(|d| d == tag)
}

/// Headline counts.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ProbeSummary {
    /// How many tags were asked about.
    pub requested: usize,
    /// How many were served.
    pub served: usize,
    /// How many of the served ones were **not** in `diff_tags`.
    ///
    /// This is the number that quantifies "patch-support list, not a retention
    /// list". Zero would mean `diff_tags` really is everything the API has.
    pub not_in_diff_tags_but_served: usize,
}

impl TagProbe {
    /// The tags that were served.
    pub fn served(&self) -> Vec<&str> {
        self.results
            .iter()
            .filter(|r| r.served)
            .map(|r| r.tag.as_str())
            .collect()
    }

    /// A one-line human summary for the run's stderr narration.
    pub fn describe(&self) -> String {
        format!(
            "{}/{} tags served, {} outside diff_tags",
            self.summary.served, self.summary.requested, self.summary.not_in_diff_tags_but_served,
        )
    }
}

/// Ask about `tags`, in order.
///
/// `diff_tags` is used only to label the rows; it never decides which tags are
/// asked about. The caller composes the list — the pair plus anything
/// `--probe-tags` added — and this function records which of them the branch
/// claims to patch.
///
/// `known` is the tag → `build_id` map for tags whose `getBuild` has already been
/// fetched this run. Those rows are filled in from what was already paid for
/// rather than re-requested: a probe costs one JSON request per tag, the pair is
/// always in the list, and re-asking it would be the single largest avoidable
/// cost in a metadata-only run. The document still answers the full question,
/// because a row that was not fetched and a row that was filled in are
/// indistinguishable to a reader — which is correct, since both are the same
/// fact.
/// What one tag's `getBuild` probe found.
///
/// Five values with five names. A tuple would be read positionally at the one
/// site that builds it and the one site that consumes it, and nothing would say
/// which `Option<String>` is the build id and which is the echoed tag.
type Answered = (u16, i64, Option<String>, Option<String>, bool);

pub async fn probe_tags(
    fetcher: &Fetcher,
    sophon_base: &str,
    params: &BranchParams,
    tags: &[String],
    diff_tags: &[String],
    known: &[(String, String)],
    jobs: usize,
) -> Result<TagProbe, ApiError> {
    // Tags already answered by the pair's own `getBuild` cost nothing, and the
    // rest used to be fetched strictly one after another. They do not depend on
    // each other, so they go together; `diff_tags` is unbounded by contract and
    // the real run asks about seven.
    //
    // The document is still assembled in `tags` order, which is what
    // `requested` and every reader's row order assume. Which probe answered
    // first is not allowed to be observable.
    let wanted: Vec<(usize, String)> = tags
        .iter()
        .enumerate()
        .filter(|(_, tag)| !known.iter().any(|(t, _)| t == *tag))
        .map(|(i, tag)| (i, params.get_build_url(sophon_base, Some(tag))))
        .collect();
    let fetched = fetcher
        .map_concurrently(&wanted, jobs, |fetcher, i, (_i, url)| async move {
            // `Kind::Probe` so a reader can ask "what did the metadata cost?"
            // without matching on URLs; no blob is fetched either way.
            match fetcher.get(&url, Kind::Probe, None).await {
                Ok(f) => {
                    let v: serde_json::Value = match serde_json::from_slice(&f.body) {
                        Ok(v) => v,
                        Err(e) => return (i, Err(ApiError::shape(&url, e).to_string())),
                    };
                    let rc = v.get("retcode").and_then(|r| r.as_i64()).unwrap_or(-1);
                    let bid = v
                        .pointer("/data/build_id")
                        .and_then(|b| b.as_str())
                        .map(str::to_string);
                    let echo = v
                        .pointer("/data/tag")
                        .and_then(|t| t.as_str())
                        .map(str::to_string);
                    let served = rc == 0 && bid.is_some();
                    (i, Ok((f.status, rc, bid, echo, served)))
                }
                // A transport failure is a failure of the probe, not of the
                // tag: a tag that does not exist arrives as a `200` carrying
                // `retcode -202`, so anything landing here means the network
                // broke mid-probe and the row would be a guess.
                Err(e) => (i, Err(e.to_string())),
            }
        })
        .await;
    // Input order, so `fetched[k]` answers `wanted[k]` — and an error is
    // reported against its own tag rather than against whichever probe happened
    // to fail first.
    let mut answered: BTreeMap<usize, Answered> = BTreeMap::new();
    for (r, (i, _url)) in fetched.into_iter().zip(&wanted) {
        match r {
            Ok(answer) => {
                answered.insert(*i, answer);
            }
            Err(e) => {
                return Err(ApiError::shape(&tags[*i], e));
            }
        }
    }

    let mut results = Vec::with_capacity(tags.len());
    for (i, tag) in tags.iter().enumerate() {
        if let Some((http, retcode, build_id, tag_echoed, served)) = answered.remove(&i) {
            results.push(TagProbeRow {
                tag: tag.clone(),
                in_diff_tags: in_diff_tags(diff_tags, tag),
                http,
                retcode,
                build_id,
                tag_echoed,
                served,
                already_fetched: false,
            });
            continue;
        }
        // Reused rather than re-requested: a probe costs one JSON request per
        // tag, the pair is always in the list, and re-asking it would be the
        // single largest avoidable cost in a metadata-only run. The document
        // still answers the full question, because a row that was not fetched
        // and a row that was filled in are indistinguishable to a reader —
        // which is correct, since both are the same fact.
        let build_id = known
            .iter()
            .find(|(t, _)| t == tag)
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        results.push(TagProbeRow {
            tag: tag.clone(),
            in_diff_tags: in_diff_tags(diff_tags, tag),
            http: 200,
            retcode: 0,
            build_id: Some(build_id),
            tag_echoed: Some(tag.clone()),
            served: true,
            // Recorded so a reader can tell a reused answer from a fetched
            // one. It changes no number in the document.
            already_fetched: true,
        });
    }

    Ok(finish(TagProbe {
        schema_version: crate::SCHEMA_VERSION,
        requested: tags.to_vec(),
        results,
        summary: ProbeSummary::default(),
    }))
}

/// Fill in the counts. Separate from `probe_tags` so a document assembled from a
/// stored list — or a test fixture — gets the same arithmetic.
pub fn finish(mut probe: TagProbe) -> TagProbe {
    probe.summary = ProbeSummary {
        requested: probe.results.len(),
        served: probe.results.iter().filter(|r| r.served).count(),
        not_in_diff_tags_but_served: probe
            .results
            .iter()
            .filter(|r| r.served && !r.in_diff_tags)
            .count(),
    };
    probe
}

/// Compose the tag list: the branch's `diff_tags`, then the pair, then any extras.
///
/// Order is fixed rather than set-derived because the document is read by people:
/// the pair first, because that is what the run is about, then the branch's list,
/// then whatever `--probe-tags` added. Duplicates are dropped, keeping the first.
pub fn compose_tags(pair: &[String], diff_tags: &[String], extra: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(pair.len() + diff_tags.len() + extra.len());
    for t in pair.iter().chain(diff_tags).chain(extra) {
        if !out.contains(t) {
            out.push(t.clone());
        }
    }
    out
}

/// Which family of route a shape came from.
///
/// The two families are the reason this document exists, so the family is a field
/// rather than something a reader infers from the status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// `autopatchhk.yuanshen.com` and friends: a missing object is a `404` with an
    /// `application/xml` body. Never a `200` carrying a code.
    Blob,
    /// `sg-hyp-api` / `sg-downloader-api`: the status is always `200` and the
    /// answer is in `retcode`.
    Json,
}

/// The negative-response report: `api/error_shapes.json`.
///
/// Every entry is a request this tool made and the answer the origin gave, both
/// recorded verbatim. Nothing here is synthesized: a replayer reading this
/// document can reproduce the status, the content-type and the body without
/// having to guess any of the three.
///
/// `Deserialize` because the document is **required**, not optional: `verify`
/// reads it back through this type, so a bundle whose shapes are not the shape
/// this tool writes fails to open rather than failing later and somewhere else.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorShapes {
    /// The bundle schema version.
    pub schema_version: u32,
    /// One entry per probe, in a fixed order so two captures diff cleanly.
    pub shapes: Vec<ErrorShape>,
}

/// One measured negative response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorShape {
    /// Stable identifier, e.g. `blob_missing`. A consumer asserts on this, never
    /// on the URL, which carries a build id and changes every capture.
    pub id: String,
    /// Which route family answered.
    pub family: Family,
    /// What was asked for, in words.
    pub request: String,
    /// The URL that was actually requested.
    pub url: String,
    /// The status, as received.
    pub status: u16,
    /// `Content-Type`, as received. `null` when the origin sent none.
    pub content_type: Option<String>,
    /// The body's `retcode`, when it is a JSON envelope. `null` for blobs.
    pub retcode: Option<i64>,
    /// Whether the origin is treating this as a failure.
    ///
    /// **This is `false` for `getgamebranches_bad_game_id`**, which answers
    /// `retcode 0` with an empty `game_branches`: a success with nothing in it.
    /// The field is here so a reader scanning for "what does a bad request look
    /// like" finds that case and sees it is not one, instead of inferring a
    /// failure from the shape.
    pub error: bool,
    /// The body, byte-exact as received.
    pub body: String,
    /// Names in the body that change on every request, and so cannot be replayed
    /// byte-for-byte.
    ///
    /// The AliyunOSS error envelope carries `RequestId`, `HostId` and `Key`;
    /// `Key` echoes the requested path, so the **body length** varies with it too.
    /// The JSON envelopes carry none — `retcode` and `message` are stable — so
    /// their list is empty and a replayer can be byte-exact.
    pub volatile_fields: Vec<String>,
    /// What the retry policy says about this status.
    ///
    /// **Derived from [`crate::http::RetryPolicy::should_retry_status`], which is
    /// the point.** A reader scanning the document for "what does a bad request
    /// look like, and would we try again" must be reading the policy rather than a
    /// constant — every probe answers `200` or `404` and neither is in the retry
    /// set, so a hardcoded `false` would be indistinguishable from a correct
    /// derivation on every input this tool can produce, and wrong the moment an
    /// origin throttled with a `429`.
    ///
    /// **It states the policy, not what this run did.** The probe path uses
    /// [`crate::http::Fetcher::observe`], which never retries, so a `true` here
    /// means "the fetcher would have retried this" and not "this was retried". The
    /// two are the same question only while probes are single-attempt, which is
    /// what `observe` is for.
    pub retryable: bool,
}

/// One row of `api/error_shapes.json`, from one observation.
///
/// **A function, because the row is a claim about policy and policy must be
/// testable.** This was built inline in a closure inside
/// [`probe_error_shapes`], which meant [`ErrorShape::retryable`] — the one field
/// that states what the retry policy would do — could only be checked against the
/// statuses the mock happens to answer. Those are `200` and `404`, neither of
/// which is in the retry set, so the `true` branch had no test at all: a
/// hardcoded `false` and a correct derivation were indistinguishable.
///
/// Extracted so `retryable_follows_the_policy_on_both_sides` can hand it a `429`.
fn shape_from(
    id: &str,
    family: Family,
    request: String,
    url: String,
    error: bool,
    o: &Observation,
) -> ErrorShape {
    ErrorShape {
        id: id.to_string(),
        family,
        request,
        url,
        status: o.status,
        content_type: o.content_type.clone(),
        retcode: o.retcode(),
        error,
        body: o.text(),
        volatile_fields: if family == Family::Blob {
            vec![
                "RequestId".to_string(),
                "HostId".to_string(),
                "Key".to_string(),
                // `Key` echoes the requested path, so the body's
                // length is a function of the URL rather than of the
                // shape.
                "body length (follows Key)".to_string(),
            ]
        } else {
            Vec::new()
        },
        // Derived, not asserted. See `ErrorShape::retryable`: this states what
        // `RetryPolicy` would do with the status, so a reader does not have to
        // reimplement the policy to know whether a shape is worth retrying.
        retryable: crate::http::RetryPolicy::should_retry_status(o.status),
    }
}

impl ErrorShapes {
    /// A one-line human summary for the run's stderr narration.
    pub fn describe(&self) -> String {
        let blob = self
            .shapes
            .iter()
            .filter(|s| s.family == Family::Blob)
            .count();
        let json = self.shapes.len() - blob;
        format!(
            "{} shapes recorded: {blob} blob (404), {json} json (200+retcode)",
            self.shapes.len()
        )
    }
}

/// What the negative probes need to build their URLs.
///
/// The blob probes are "the real prefix, plus an id that does not exist" and "a
/// real id, under the other build's prefix". Both halves are needed, so they are
/// named fields rather than positional arguments — a probe that silently swapped
/// the real id for the bogus one would still report a `404` and prove nothing.
#[derive(Debug, Clone, Copy)]
pub struct ErrorProbes<'a> {
    /// `…/manifests/<series>/<build_id>` for the new build.
    pub manifest_prefix: &'a str,
    /// `…/chunks/<series>/<build_id>` for the new build.
    pub chunk_prefix: &'a str,
    /// The old build's manifest prefix.
    pub other_manifest_prefix: &'a str,
    /// The old build's chunk prefix.
    pub other_chunk_prefix: &'a str,
    /// A chunk id the **new build declares and the old one does not**.
    ///
    /// New-build-exclusive, not merely real, and the distinction is the whole
    /// probe. A shared chunk is one id under two addresses and the old prefix
    /// *does* serve it, so asking for it there returns `200` — which would record
    /// "a valid id under the other build's prefix is not a 404", the exact
    /// opposite of the claim, from a probe that looks like it worked.
    pub new_only_chunk_id: &'a str,
}

/// An id that cannot exist. Fixed rather than random so a captured bundle's URLs
/// are stable across runs and two captures diff cleanly.
const BOGUS_ID: &str = "0000000000000000_00000000000000000000000000000000";

/// Ask the API what it does with requests it will not serve, and record it.
///
/// **Never fails the capture.** A negative response is a successful observation,
/// and the only thing that can go wrong is the network — in which case there is
/// nothing to record and the error propagates. Nothing here asserts that the
/// origin answered the way it answered last time: if a future origin changes its
/// shape, the document changes with it, and that is the document being honest
/// rather than a capture failing over a difference nobody asked about.
pub async fn probe_error_shapes(
    fetcher: &Fetcher,
    endpoints: &crate::biz::Endpoints<'_>,
    biz: &crate::biz::Biz,
    params: &BranchParams,
    probes: &ErrorProbes<'_>,
    jobs: usize,
) -> Result<ErrorShapes, ApiError> {
    // Same shape, three ways: the request this tool would have made, with
    // exactly one thing wrong. Built from the good URL rather than assembled by
    // hand, so a probe cannot drift into a spelling the tool never sends.
    //
    // The bogus values are deliberately generic (`null` / `undefined`) rather
    // than naming this tool: a request the origin logs must not identify the
    // client behind it.
    let good_build = endpoints.get_build_url(params, None);
    let bad_package = with_param(&good_build, "package_id", "undefined");
    let unknown_tag = endpoints.get_build_url(params, Some("null"));
    let good_configs = endpoints.hyp_url(biz, crate::biz::HypEndpoint::GameConfigs, "en-us");
    let good_branches = endpoints.hyp_url(biz, crate::biz::HypEndpoint::GameBranches, "en-us");
    let bad_game = "undefined";

    let wanted: Vec<(&str, Family, String, String, bool)> = vec![
        (
            "blob_missing",
            Family::Blob,
            "a manifest id under the new build's prefix that the build does not have".into(),
            format!(
                "{}/{}",
                probes.manifest_prefix.trim_end_matches('/'),
                BOGUS_ID
            ),
            true,
        ),
        (
            "chunk_missing",
            Family::Blob,
            "a chunk id under the new build's prefix that the build does not have".into(),
            format!("{}/{}", probes.chunk_prefix.trim_end_matches('/'), BOGUS_ID),
            true,
        ),
        (
            "blob_wrong_build",
            Family::Blob,
            "a chunk id that exists, requested under the *other* build's prefix".into(),
            format!(
                "{}/{}",
                probes.other_chunk_prefix.trim_end_matches('/'),
                probes.new_only_chunk_id
            ),
            true,
        ),
        (
            "getbuild_unknown_tag",
            Family::Json,
            "getBuild for a tag the API does not serve".into(),
            unknown_tag,
            true,
        ),
        (
            "getbuild_bad_package",
            Family::Json,
            "getBuild with a package_id the API does not know".into(),
            bad_package,
            true,
        ),
        (
            "getgameconfigs_bad_game_id",
            Family::Json,
            "getGameConfigs with a game_ids[] the API does not know".into(),
            with_param(&good_configs, "game_ids[]", bad_game),
            true,
        ),
        (
            "getgamebranches_bad_game_id",
            Family::Json,
            "getGameBranches with a game_ids[] the API does not know — measured to \
             answer retcode 0 with an empty game_branches, which is a success"
                .into(),
            with_param(&good_branches, "game_ids[]", bad_game),
            // The whole point of recording it: this is not an error.
            false,
        ),
    ];

    // A probe that never reached the origin is not a shape, so the failure
    // propagates — and it propagates against its own URL, because input order
    // means `observed[k]` is `wanted[k]`'s and no other.
    let observed = fetcher
        .map_concurrently(&wanted, jobs, |fetcher, i, (id, family, request, url, error)| async move {
            let o = match fetcher.observe(&url, Kind::Probe).await {
                Ok(o) => o,
                Err(e) => return (i, Err(e.to_string())),
            };
            (i, Ok(shape_from(id, family, request, url, error, &o)))
        })
        .await;
    let mut shapes = Vec::with_capacity(wanted.len());
    for (r, w) in observed.into_iter().zip(&wanted) {
        shapes.push(r.map_err(|e| ApiError::shape(&w.3, e))?);
    }

    Ok(ErrorShapes {
        schema_version: crate::SCHEMA_VERSION,
        shapes,
    })
}

/// `url` with one query parameter's value replaced.
///
/// `key` is matched literally, brackets included: the measured spelling of
/// `game_ids[]` keeps them, and matching on a decoded form would silently not
/// find the parameter.
///
/// **The replacement value is percent-encoded, by [`crate::biz::enc`].** Splicing
/// `value` in raw would make this a second URL grammar living beside
/// [`crate::biz::push_pair`] — the one every real URL in this tool goes through.
/// The two would agree only while every value stays `"undefined"`: a value
/// containing `&`, `=`, `+` or a space would split the query into a different set
/// of parameters and the probe would ask the wrong question while still looking
/// like a probe.
///
/// The key stays literal for the same reason it is matched literally: `push_pair`
/// appends keys unencoded too, so the two now agree on both halves. Encoding is a
/// no-op for the values actually used, so **no recorded URL changes** — see
/// `a_value_needing_encoding_is_encoded_and_todays_is_not`.
fn with_param(url: &str, key: &str, value: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let prefix = format!("{key}=");
    let parts: Vec<String> = query
        .split('&')
        .map(|p| {
            if p.starts_with(&prefix) {
                format!("{prefix}{}", crate::biz::enc(value))
            } else {
                p.to_string()
            }
        })
        .collect();
    format!("{base}?{}", parts.join("&"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(tag: &str, in_diff_tags: bool, retcode: i64, build: Option<&str>) -> TagProbeRow {
        TagProbeRow {
            tag: tag.to_string(),
            in_diff_tags,
            http: 200,
            retcode,
            build_id: build.map(str::to_string),
            tag_echoed: build.map(|_| tag.to_string()),
            served: retcode == 0 && build.is_some(),
            already_fetched: false,
        }
    }

    #[test]
    fn the_composed_list_puts_the_pair_first_and_deduplicates() {
        let got = compose_tags(
            &["7.1.0".into(), "7.0.0".into()],
            &["7.0.0".into(), "6.7.0".into()],
            &["6.6.0".into(), "7.1.0".into()],
        );
        assert_eq!(got, vec!["7.1.0", "7.0.0", "6.7.0", "6.6.0"]);
    }

    #[test]
    fn retryable_follows_the_policy_on_both_sides() {
        // Both branches, driven through the real `shape_from`. The `true` side is
        // the one that matters and the one no healthy capture reaches: it needs a
        // status that *is* retryable, which means an origin that throttles a
        // probe.
        let obs = |status: u16| Observation {
            status,
            content_type: Some("application/json".into()),
            body: br#"{"retcode":-1,"message":"slow down"}"#.to_vec(),
        };
        let shape = |status: u16| {
            shape_from(
                "probe",
                Family::Json,
                "a request".into(),
                "http://example.invalid/x".into(),
                true,
                &obs(status),
            )
        };

        // The policy's own answer, on both sides of it.
        for status in [408_u16, 429, 500, 503, 599] {
            assert!(
                shape(status).retryable,
                "{status} is in the retry set, so the document must say so"
            );
        }
        for status in [200_u16, 400, 403, 404, 410] {
            assert!(
                !shape(status).retryable,
                "{status} is not in the retry set, so the document must not claim it is"
            );
        }

        // And the field tracks the function rather than agreeing with it by
        // chance: a status the policy grows later is reflected with no edit here.
        assert_eq!(
            shape(429).retryable,
            crate::http::RetryPolicy::should_retry_status(429)
        );
        assert_eq!(
            shape(404).retryable,
            crate::http::RetryPolicy::should_retry_status(404)
        );
    }

    #[test]
    fn a_value_needing_encoding_is_encoded_and_todays_is_not() {
        // The second half of the claim is the one that matters for the artifact:
        // if encoding changed the URLs the probes send today, every recorded
        // `error_shapes.json` URL would be a fiction.
        let good = "https://api.example/getBuild?branch=main&package_id=ScSYQBFhu9&tag=7.1.0";

        // Unchanged: what the probes actually send today.
        for today in ["undefined", "0", "ScSYQBFhu9", "en-us"] {
            assert_eq!(
                with_param(good, "package_id", today),
                good.replace("ScSYQBFhu9", today),
                "{today:?} must not be altered"
            );
        }

        // Changed: a value that would otherwise reshape the query.
        let nasty = with_param(good, "package_id", "a&tag=evil");
        assert!(
            nasty.contains("package_id=a%26tag%3Devil"),
            "an ampersand in the value must not become another parameter: {nasty}"
        );
        assert_eq!(
            nasty.matches('&').count(),
            good.matches('&').count(),
            "the query must keep exactly as many parameters as it had: {nasty}"
        );
        assert_eq!(
            nasty.split('?').nth(1).unwrap().split('&').count(),
            good.split('?').nth(1).unwrap().split('&').count(),
            "and one value is still one parameter"
        );

        // The key is matched and kept literally, brackets and all — the measured
        // spelling of `game_ids[]` must survive, and `push_pair` does the same.
        let cfg = "https://api.example/getGameConfigs?game_ids[]=gopR6Cufr3&lang=en-us";
        assert_eq!(
            with_param(cfg, "game_ids[]", "undefined"),
            "https://api.example/getGameConfigs?game_ids[]=undefined&lang=en-us",
            "the bracketed key is matched literally and not encoded"
        );

        // And a URL with no query is returned untouched rather than gaining one.
        assert_eq!(with_param("https://x.example/y", "k", "v"), "https://x.example/y");
    }

    #[test]
    fn a_shape_carries_the_observation_it_was_built_from() {
        // The extraction must not have lost a field. Everything a reader of
        // `api/error_shapes.json` relies on is spelled out here.
        let o = Observation {
            status: 404,
            content_type: Some("application/xml".into()),
            body: b"<?xml version=\"1.0\"?><Error><Code>NoSuchKey</Code></Error>".to_vec(),
        };
        let s = shape_from(
            "blob_missing",
            Family::Blob,
            "a chunk that is not there".into(),
            "http://example.invalid/blob".into(),
            true,
            &o,
        );
        assert_eq!(s.id, "blob_missing");
        assert_eq!(s.family, Family::Blob);
        assert_eq!(s.status, 404);
        assert_eq!(s.content_type.as_deref(), Some("application/xml"));
        assert!(s.error);
        assert!(s.body.contains("NoSuchKey"), "the body is byte-exact: {}", s.body);
        assert_eq!(
            s.volatile_fields,
            vec!["RequestId", "HostId", "Key", "body length (follows Key)"],
            "the blob envelope's volatile names, because `Key` follows the path"
        );
        assert!(s.retcode.is_none(), "an XML envelope has no retcode");
    }

    /// The measured shape of the finding: `diff_tags` is a patch-support list and
    /// serving more tags than it names is the *interesting* case, so the counter
    /// that reports it must not be derivable from `served` alone.
    #[test]
    fn the_summary_separates_served_from_outside_diff_tags() {
        let p = finish(TagProbe {
            schema_version: 1,
            requested: vec![],
            results: vec![
                row("7.1.0", false, 0, Some("sMXGW2ll3Fuu")),
                row("7.0.0", true, 0, Some("3XnPviMfoILi")),
                row("6.7.0", true, 0, Some("E2zvp18oyzw6")),
                row("6.6.0", false, 0, Some("L7CJe4FP1z71")),
                row("0.0.1", false, -202, None),
            ],
            summary: ProbeSummary::default(),
        });
        assert_eq!(p.summary.requested, 5);
        assert_eq!(p.summary.served, 4);
        assert_eq!(p.summary.not_in_diff_tags_but_served, 2);
        assert_eq!(p.served(), vec!["7.1.0", "7.0.0", "6.7.0", "6.6.0"]);
    }

    /// A `-202` is an answer. Without a `build_id` the tag is unserved whatever
    /// the retcode says, because the URL that matters is the one to fetch from.
    #[test]
    fn served_requires_a_build_id_not_just_a_zero_retcode() {
        let p = finish(TagProbe {
            schema_version: 1,
            requested: vec!["x".into()],
            results: vec![row("x", false, 0, None)],
            summary: ProbeSummary::default(),
        });
        assert_eq!(p.summary.served, 0);
    }

    #[test]
    fn an_empty_probe_is_zeroed_not_panicking() {
        let p = finish(TagProbe {
            schema_version: 1,
            requested: vec![],
            results: vec![],
            summary: ProbeSummary::default(),
        });
        assert_eq!(p.summary.requested, 0);
        assert_eq!(p.summary.served, 0);
        assert!(p.served().is_empty());
        assert!(p.describe().contains("0/0"));
    }

    #[test]
    fn the_document_is_the_documented_json_shape() {
        let p = finish(TagProbe {
            schema_version: 1,
            requested: vec!["6.6.0".into(), "0.0.1".into()],
            results: vec![
                row("6.6.0", false, 0, Some("L7CJe4FP1z71")),
                row("0.0.1", false, -202, None),
            ],
            summary: ProbeSummary::default(),
        });
        let v: serde_json::Value = serde_json::to_value(&p).unwrap();
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["requested"][0], "6.6.0");
        assert_eq!(v["results"][0]["tag"], "6.6.0");
        assert_eq!(v["results"][0]["in_diff_tags"], false);
        assert_eq!(v["results"][0]["http"], 200);
        assert_eq!(v["results"][0]["retcode"], 0);
        assert_eq!(v["results"][0]["build_id"], "L7CJe4FP1z71");
        assert_eq!(v["results"][0]["served"], true);
        assert_eq!(v["results"][1]["retcode"], -202);
        assert_eq!(v["results"][1]["build_id"], serde_json::Value::Null);
        assert_eq!(v["results"][1]["served"], false);
        assert_eq!(v["summary"]["requested"], 2);
        assert_eq!(v["summary"]["served"], 1);
        assert_eq!(v["summary"]["not_in_diff_tags_but_served"], 1);
    }

    // ------------------------------------------------------ against a socket --

    /// `probe_tags` is the one place a `retcode != 0` is a result rather than an
    /// error, so it is tested against a real response rather than a literal.
    mod local_server {
        use super::*;
        use std::io::{BufRead, BufReader, Write};
        use std::net::{TcpListener, TcpStream};
        use std::sync::atomic::{AtomicU32, Ordering};

        pub fn spawn(statuses: Vec<(u16, String)>) -> (u16, Arc<AtomicU32>) {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().unwrap().port();
            let hits = Arc::new(AtomicU32::new(0));
            let h = hits.clone();
            std::thread::spawn(move || {
                for (status, body) in statuses {
                    let Ok((stream, _)) = listener.accept() else {
                        return;
                    };
                    h.fetch_add(1, Ordering::SeqCst);
                    serve(stream, status, &body);
                }
            });
            (port, hits)
        }

        fn serve(mut stream: TcpStream, status: u16, body: &str) {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                    break;
                }
            }
            let head = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body.as_bytes());
            let _ = stream.flush();
        }
    }

    use local_server::*;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    #[tokio::test]
    async fn an_unserved_tag_is_recorded_as_an_answer_not_an_error() {
        let (port, hits) = spawn(vec![
            (
                200,
                r#"{"retcode":0,"data":{"build_id":"sMXGW2ll3Fuu","tag":"7.1.0"}}"#.into(),
            ),
            (
                200,
                r#"{"data":null,"message":"not found","retcode":-202}"#.into(),
            ),
        ]);
        let params = BranchParams {
            package_id: "p".into(),
            password: "w".into(),
            branch: "main".into(),
            tag: "7.1.0".into(),
            diff_tags: vec![],
        };
        let fetcher = Fetcher::new(
            crate::http::Transport::local(),
            crate::http::RequestLog::new(),
        )
        .unwrap();
        let probe = probe_tags(
            &fetcher,
            &format!("http://127.0.0.1:{port}/api"),
            &params,
            &["7.1.0".into(), "0.0.1".into()],
            &["7.0.0".into()],
            &[],
            4,
        )
        .await
        .expect("a -202 is an answer, not a failure");

        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert!(probe.results[0].served);
        assert_eq!(probe.results[0].build_id.as_deref(), Some("sMXGW2ll3Fuu"));
        assert_eq!(probe.results[0].tag_echoed.as_deref(), Some("7.1.0"));
        assert!(!probe.results[0].in_diff_tags);
        assert!(!probe.results[1].served);
        assert_eq!(probe.results[1].retcode, -202);
        assert_eq!(
            probe.results[1].http, 200,
            "the API answers 200 and then says no"
        );
        assert_eq!(probe.summary.served, 1);
        assert_eq!(probe.summary.not_in_diff_tags_but_served, 1);
    }

    #[tokio::test]
    async fn a_tag_already_fetched_is_answered_from_the_run_not_the_network() {
        // One scripted response. If the known tag were re-requested this would
        // be consumed by the wrong tag and the server would have nothing left
        // for the second.
        let (port, hits) = spawn(vec![(
            200,
            r#"{"retcode":0,"data":{"build_id":"OTHER","tag":"6.7.0"}}"#.into(),
        )]);
        let params = BranchParams {
            package_id: "p".into(),
            password: "w".into(),
            branch: "main".into(),
            tag: "7.1.0".into(),
            diff_tags: vec![],
        };
        let fetcher = Fetcher::new(
            crate::http::Transport::local(),
            crate::http::RequestLog::new(),
        )
        .unwrap();
        let probe = probe_tags(
            &fetcher,
            &format!("http://127.0.0.1:{port}/api"),
            &params,
            &["7.1.0".into(), "6.7.0".into()],
            &["7.0.0".into()],
            &[("7.1.0".into(), "sMXGW2ll3Fuu".into())],
            4,
        )
        .await
        .unwrap();

        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "only the unknown tag was fetched"
        );
        assert!(probe.results[0].served);
        assert_eq!(probe.results[0].build_id.as_deref(), Some("sMXGW2ll3Fuu"));
        assert!(probe.results[0].already_fetched);
        assert!(!probe.results[0].in_diff_tags);
        assert_eq!(probe.results[1].build_id.as_deref(), Some("OTHER"));
        assert!(!probe.results[1].already_fetched);
        assert_eq!(
            probe.summary.served, 2,
            "a reused answer still counts as served"
        );
    }

    #[tokio::test]
    async fn every_probe_is_recorded_as_kind_probe() {
        let (port, _) = spawn(vec![(
            200,
            r#"{"retcode":0,"data":{"build_id":"b","tag":"t"}}"#.into(),
        )]);
        let params = BranchParams {
            package_id: "p".into(),
            password: "w".into(),
            branch: "main".into(),
            tag: "t".into(),
            diff_tags: vec![],
        };
        let log = crate::http::RequestLog::new();
        let fetcher = Fetcher::new(crate::http::Transport::local(), log.clone()).unwrap();
        probe_tags(
            &fetcher,
            &format!("http://127.0.0.1:{port}/api"),
            &params,
            &["t".into()],
            &[],
            &[],
            4,
        )
        .await
        .unwrap();
        let records = log.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, Kind::Probe);
    }
}
