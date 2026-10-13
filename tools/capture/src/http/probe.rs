//! `getBuild?tag=` probing: which version tags the API will actually serve.
//!
//! **M1.** `diff_tags` is a patch-support list, not a retention list. Measured
//! 2026-10-14: `6.6.0` and `5.5.0` are absent from `diff_tags` and both served.
//! The difference decides whether an old-version file is reconstructible at all
//! today, without the legacy zip path — and it is the one measurement in this
//! tool that changes a *design conclusion* rather than a number in a table.
//!
//! A probe costs one small JSON request per tag and downloads no blob, so it is
//! cheap enough to run by default and is worth recording every time.
//!
//! **A `retcode -202` here is an answer, not a failure.** It means "this API does
//! not serve that tag", which is the question. Treating it as an error would make
//! the probe unable to report its own most interesting result.

use serde::Serialize;

use crate::biz::BranchParams;
use crate::http::{ApiError, Fetcher, Kind};

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
            self.summary.served,
            self.summary.requested,
            self.summary.not_in_diff_tags_but_served,
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
pub async fn probe_tags(
    fetcher: &Fetcher,
    sophon_base: &str,
    params: &BranchParams,
    tags: &[String],
    diff_tags: &[String],
    known: &[(String, String)],
) -> Result<TagProbe, ApiError> {
    let mut results = Vec::with_capacity(tags.len());
    for tag in tags {
        if let Some((_, build_id)) = known.iter().find(|(t, _)| t == tag) {
            results.push(TagProbeRow {
                tag: tag.clone(),
                in_diff_tags: in_diff_tags(diff_tags, tag),
                http: 200,
                retcode: 0,
                build_id: Some(build_id.clone()),
                tag_echoed: Some(tag.clone()),
                served: true,
                // Recorded so a reader can tell a reused answer from a fetched
                // one. It changes no number in the document.
                already_fetched: true,
            });
            continue;
        }
        let url = params.get_build_url(sophon_base, Some(tag));
        // `Kind::Probe` so a reader can ask "what did the metadata cost?" without
        // matching on URLs; no blob is fetched either way.
        let (http, retcode, build_id, tag_echoed, served) =
            match fetcher.get(&url, Kind::Probe, None).await {
                Ok(f) => {
                    let v: serde_json::Value = serde_json::from_slice(&f.body)
                        .map_err(|e| ApiError::shape(&url, e))?;
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
                    (f.record.status, rc, bid, echo, served)
                }
                // A transport failure is a failure of the probe, not of the
                // tag: a tag that does not exist arrives as a `200` carrying
                // `retcode -202`, so anything landing here means the network
                // broke mid-probe and the row would be a guess.
                Err(e) => return Err(e),
            };
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
            (200, r#"{"data":null,"message":"not found","retcode":-202}"#.into()),
        ]);
        let params = BranchParams {
            package_id: "p".into(),
            password: "w".into(),
            branch: "main".into(),
            tag: "7.1.0".into(),
            diff_tags: vec![],
        };
        let fetcher = Fetcher::new(crate::http::Transport::local(), crate::http::RequestLog::new())
            .unwrap();
        let probe = probe_tags(
            &fetcher,
            &format!("http://127.0.0.1:{port}/api"),
            &params,
            &["7.1.0".into(), "0.0.1".into()],
            &["7.0.0".into()],
            &[],
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
        assert_eq!(probe.results[1].http, 200, "the API answers 200 and then says no");
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
        let fetcher = Fetcher::new(crate::http::Transport::local(), crate::http::RequestLog::new())
            .unwrap();
        let probe = probe_tags(
            &fetcher,
            &format!("http://127.0.0.1:{port}/api"),
            &params,
            &["7.1.0".into(), "6.7.0".into()],
            &["7.0.0".into()],
            &[("7.1.0".into(), "sMXGW2ll3Fuu".into())],
        )
        .await
        .unwrap();

        assert_eq!(hits.load(Ordering::SeqCst), 1, "only the unknown tag was fetched");
        assert!(probe.results[0].served);
        assert_eq!(probe.results[0].build_id.as_deref(), Some("sMXGW2ll3Fuu"));
        assert!(probe.results[0].already_fetched);
        assert!(!probe.results[0].in_diff_tags);
        assert_eq!(probe.results[1].build_id.as_deref(), Some("OTHER"));
        assert!(!probe.results[1].already_fetched);
        assert_eq!(probe.summary.served, 2, "a reused answer still counts as served");
    }

    #[tokio::test]
    async fn every_probe_is_recorded_as_kind_probe() {
        let (port, _) = spawn(vec![(200, r#"{"retcode":0,"data":{"build_id":"b","tag":"t"}}"#.into())]);
        let params = BranchParams {
            package_id: "p".into(),
            password: "w".into(),
            branch: "main".into(),
            tag: "t".into(),
            diff_tags: vec![],
        };
        let log = crate::http::RequestLog::new();
        let fetcher =
            Fetcher::new(crate::http::Transport::local(), log.clone()).unwrap();
        probe_tags(
            &fetcher,
            &format!("http://127.0.0.1:{port}/api"),
            &params,
            &["t".into()],
            &[],
            &[],
        )
        .await
        .unwrap();
        let records = log.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, Kind::Probe);
    }
}