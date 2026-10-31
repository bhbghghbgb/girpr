//! The four HoYoPlay endpoints and Sophon's `getBuild`.
//!
//! **M2.** Every body is captured **byte-exact** rather than re-serialized, so a
//! reader can `diff` a captured file against a fresh request and see nothing. The
//! typed views in [`types`] are what the run *acts on*; they are never what gets
//! written, because a typed round-trip through this API's string-encoded numbers
//! and its non-ASCII category names is one lossy step away from the truth.
//!
//! `retcode != 0` is exit `2` with the URL named, and that check lives in
//! [`crate::http::Fetcher::get_json`] so no endpoint can be read without it.
//! A `200` carrying `retcode -202` is how this API says *no such tag*, and for
//! `getBuild` that is a hard failure: a capture that quietly patched 7.1.0 to
//! 7.1.0 would be worse than no capture at all.
//!
//! ## Order
//!
//! `getGameBranches` has to come first — it carries the `package_id` and
//! `password` that every `getBuild` URL is built from — which means the tag
//! selectors are resolved *between* the two groups. [`HypClient::capture_all`]
//! hides that ordering rather than leaving each caller to get it right.

use crate::biz::{Biz, BranchParams, Endpoints, HypEndpoint};
use crate::config::{NewTagSelector, OldTagSelector, VersionPair};
use crate::http::{ApiError, Fetcher, Kind, RequestLog};

pub mod types;

pub use types::{
    BuildManifest, Category, DOWNLOAD_MODE_CHUNK, DeprecatedFile, DeprecatedFiles, Download,
    GameBranch, GameBranches, GameConfigs, GamePackage, GamePackages, GetBuild, IntOrStr,
    LaunchConfig, LegacyArchive, LegacyAudio, LegacyMain, LegacyRelease, ManifestRef, Stats,
    Wrapper,
};

/// The language every HoYoPlay endpoint is asked in.
///
/// `gircap` captures bytes, not locales, and the responses are byte-exact either
/// way. `en-us` is the international launcher's own default, so a capture made
/// with `--biz hk4e_global` is a capture of what that launcher actually asks for.
pub const CAPTURE_LANGUAGE: &str = "en-us";

/// One captured response: the bytes as served, plus the request that got them.
#[derive(Debug, Clone)]
pub struct CapturedBody {
    /// The endpoint name, e.g. `getGameConfigs`.
    pub endpoint: &'static str,
    /// The file name under `api/`, e.g. `getBuild.7.1.0.json`.
    pub file: String,
    /// The request's sequence number, for filling in `stored_as`.
    pub seq: u64,
    /// The body, byte-exact.
    pub bytes: Vec<u8>,
}

impl CapturedBody {
    /// Parse the body as `T`.
    ///
    /// Separate from the fetch on purpose: the run must be able to keep the raw
    /// bytes even when the typed view is rejected, because a body this tool
    /// cannot understand is still evidence about what the API said.
    pub fn parse<T: serde::de::DeserializeOwned>(&self) -> Result<T, ApiError> {
        serde_json::from_slice(&self.bytes).map_err(|e| ApiError::shape(self.endpoint, e))
    }

    /// The `data` payload.
    pub fn data<T: serde::de::DeserializeOwned>(&self) -> Result<T, ApiError> {
        self.parse::<Wrapper<T>>().map(|w| w.data)
    }

    /// The top-level `retcode`, as the API sent it.
    ///
    /// Read from the raw bytes rather than through a typed view, because A1 is
    /// the assertion that the response was *well-formed enough to answer* — a
    /// view that failed to deserialise is exactly the case where the retcode
    /// still has to be reported.
    pub fn retcode(&self) -> i64 {
        serde_json::from_slice::<EnvelopeOnly>(&self.bytes)
            .map(|e| e.retcode)
            .unwrap_or(i64::MIN)
    }
}

/// Just the envelope, for [`CapturedBody::retcode`].
#[derive(serde::Deserialize)]
struct EnvelopeOnly {
    retcode: i64,
}

/// All six bodies of a capture, plus the derived pair.
#[derive(Debug, Clone)]
pub struct Captured {
    /// `api/getGameConfigs.json`.
    pub configs: CapturedBody,
    /// `api/getGameBranches.json`.
    pub branches: CapturedBody,
    /// `api/getGameDeprecatedFileConfigs.json`.
    pub deprecated: CapturedBody,
    /// `api/getGamePackages.json`. Evidence only.
    pub packages: CapturedBody,
    /// `api/getBuild.<new>.json`.
    pub build_new: CapturedBody,
    /// `api/getBuild.<old>.json`.
    pub build_old: CapturedBody,
    /// The resolved pair, with both build ids attached.
    pub versions: VersionPair,
}

impl Captured {
    /// Every body, in the order they were fetched.
    pub fn bodies(&self) -> [&CapturedBody; 6] {
        [
            &self.configs,
            &self.branches,
            &self.deprecated,
            &self.packages,
            &self.build_new,
            &self.build_old,
        ]
    }

    /// Every body, mutable, in the order they were fetched.
    ///
    /// Exists so [`attach_seq`] can be called without a caller naming six field
    /// names and risking one of them being missed. The order matches
    /// [`Captured::bodies`] because the only caller that cares — the log
    /// lookup — matches on the endpoint inside the URL, not on position.
    pub fn bodies_mut(&mut self) -> [&mut CapturedBody; 6] {
        [
            &mut self.configs,
            &mut self.branches,
            &mut self.deprecated,
            &mut self.packages,
            &mut self.build_new,
            &mut self.build_old,
        ]
    }

    /// The launcher configuration.
    pub fn game_configs(&self) -> Result<GameConfigs, ApiError> {
        self.configs.data()
    }

    /// The branch parameters, re-derived from the captured body.
    ///
    /// Re-parsed rather than carried over from the fetch so that a bundle's
    /// claim about its own downloader parameters can be checked against the bytes
    /// stored beside it. They come from the same body, so this is cheap.
    pub fn branch_params(&self) -> Result<BranchParams, ApiError> {
        BranchParams::from_branches(&self.branches.bytes)
    }

    /// The new build.
    pub fn new_build(&self) -> Result<GetBuild, ApiError> {
        self.build_new.data()
    }
    /// The old build.
    pub fn old_build(&self) -> Result<GetBuild, ApiError> {
        self.build_old.data()
    }

    /// Assert the launcher is on chunk delivery, and that the two builds agree
    /// with the pair they were fetched for.
    ///
    /// The tag echo is checked because a `getBuild` that answers with a *different*
    /// tag than was asked for is the one case where the whole capture would be
    /// silently about the wrong version pair: every URL in the bundle would be
    /// right and the bundle would be wrong.
    pub fn validate(&self) -> Result<(), ApiError> {
        let configs = self.game_configs()?;
        configs.require_chunk_mode()?;
        // Fail here rather than at the fixture write: the scan directory is a
        // path the launcher supplies and the fixture joins under `game/`. This
        // runs before a manifest or a blob is fetched, so an escaping value
        // costs no network work and no artifact directory.
        configs.audio_pkg_scan_dir()?;

        let new = self.new_build()?;
        if new.tag != self.versions.new.tag {
            return Err(ApiError::shape(
                self.build_new.endpoint,
                format!(
                    "asked for tag {:?} and the build echoes {:?}; the capture would be \
                     about the wrong version pair",
                    self.versions.new.tag, new.tag
                ),
            ));
        }
        let old = self.old_build()?;
        if old.tag != self.versions.old.tag {
            return Err(ApiError::shape(
                self.build_old.endpoint,
                format!(
                    "asked for tag {:?} and the build echoes {:?}",
                    self.versions.old.tag, old.tag
                ),
            ));
        }

        // Storage is asserted once, here, before M3 fetches a single blob. A
        // compression value this tool cannot decode would otherwise surface as
        // garbage far downstream, with nothing to point at.
        for (which, build) in [("new", &new), ("old", &old)] {
            for m in &build.manifests {
                m.manifest_download
                    .require_zstd_plaintext(&format!("{which} manifest_download"))?;
                m.chunk_download
                    .require_zstd_plaintext(&format!("{which} chunk_download"))?;
            }
        }
        Ok(())
    }

    /// How stale the legacy package endpoint is, as `(major_versions, patches)`.
    ///
    /// Recorded because the whole reason the legacy path is irrelevant is a claim
    /// about this endpoint being frozen, and a bundle should be able to show it.
    pub fn legacy_staleness(&self) -> Option<(u64, Vec<String>)> {
        self.packages
            .data::<GamePackages>()
            .ok()?
            .staleness(&self.versions.new.tag)
    }
}

/// Issues the metadata requests for one capture.
#[derive(Debug)]
pub struct HypClient<'a> {
    endpoints: Endpoints<'a>,
    biz: Biz,
    fetcher: &'a Fetcher,
}

impl<'a> HypClient<'a> {
    /// Build a client over an existing fetcher, so the request log is shared.
    pub fn new(endpoints: Endpoints<'a>, biz: Biz, fetcher: &'a Fetcher) -> Self {
        HypClient {
            endpoints,
            biz,
            fetcher,
        }
    }

    /// The log every request through this client is recorded in.
    pub fn log(&self) -> &RequestLog {
        self.fetcher.log()
    }

    /// Fetch one HoYoPlay endpoint, byte-exact.
    pub async fn endpoint(&self, endpoint: HypEndpoint) -> Result<CapturedBody, ApiError> {
        let url = self
            .endpoints
            .hyp_url(&self.biz, endpoint, CAPTURE_LANGUAGE);
        self.fetch(&url, endpoint.path(), format!("{}.json", endpoint.path()))
            .await
    }

    /// Fetch one `getBuild`, byte-exact.
    pub async fn get_build(
        &self,
        params: &BranchParams,
        tag: &str,
    ) -> Result<CapturedBody, ApiError> {
        let url = self.endpoints.get_build_url(params, Some(tag));
        self.fetch(&url, "getBuild", format!("getBuild.{tag}.json"))
            .await
    }

    async fn fetch(
        &self,
        url: &str,
        endpoint: &'static str,
        file: String,
    ) -> Result<CapturedBody, ApiError> {
        let (bytes, _value) = self.fetcher.get_json(url, Kind::Api).await?;
        tracing::debug!(endpoint, file, bytes = bytes.len(), "captured");
        Ok(CapturedBody {
            endpoint,
            file,
            seq: 0, // filled in by the caller from the request log
            bytes,
        })
    }

    /// Fetch all six bodies and resolve the version pair.
    ///
    /// **Six bodies, two round trips.** A sequential `.await` per body would be
    /// the entire network cost of a `--no-chunk-blobs` metadata run. The
    /// dependency is narrower than the sequence implied: four of the six URLs
    /// URLs depend only on `biz` and the capture language, so all four go at
    /// once, and of the remaining two — `getBuild` for each tag — neither
    /// depends on the other.
    ///
    /// Only `getGameBranches` is a real dependency, because it supplies the
    /// `package_id` and `password` that every `getBuild` URL needs. That is
    /// also the reason this is one function rather than six: a caller that
    /// issued a `getBuild` before it would build a URL with no package in it,
    /// and that is now the *only* ordering that matters.
    ///
    /// `jobs` bounds each wave. It is the capture's own `--jobs`, because there
    /// is no other number here that means anything to an operator.
    pub async fn capture_all(
        &self,
        new: NewTagSelector<'_>,
        old: OldTagSelector<'_>,
        jobs: usize,
    ) -> Result<Captured, ApiError> {
        // The four bodies whose URL needs nothing but `biz`. Also the answer key:
        // `map_concurrently` returns input order, so the reads below are
        // positional and each one is named by the `PLAIN` entry above it.
        const PLAIN: [&str; 4] = [
            "getGameConfigs",
            "getGameBranches",
            "getGameDeprecatedFileConfigs",
            "getGamePackages",
        ];
        const PLAIN_ENDPOINTS: [HypEndpoint; 4] = [
            HypEndpoint::GameConfigs,
            HypEndpoint::GameBranches,
            HypEndpoint::GameDeprecatedFileConfigs,
            HypEndpoint::GamePackages,
        ];
        let plain: Vec<(String, &'static str)> = PLAIN
            .iter()
            .zip(PLAIN_ENDPOINTS)
            .map(|(path, e)| {
                (
                    self.endpoints.hyp_url(&self.biz, e, CAPTURE_LANGUAGE),
                    *path,
                )
            })
            .collect();

        let mut got = self
            .fetcher
            .map_concurrently(&plain, jobs, |fetcher, i, (url, path)| async move {
                let fetched = fetcher.get_json(&url, Kind::Api).await;
                tracing::debug!(endpoint = path, file = format!("{path}.json"), "captured");
                let bytes = match fetched {
                    Ok((bytes, _value)) => bytes,
                    Err(e) => return (i, Err(e.to_string())),
                };
                let body = CapturedBody {
                    endpoint: path,
                    file: format!("{path}.json"),
                    seq: 0, // filled in by the caller from the request log
                    bytes,
                };
                (i, Ok(body))
            })
            .await
            .into_iter();

        let err = |what: &str, e: String| ApiError::shape(what, e);
        let configs = got
            .next()
            .expect("four bodies")
            .map_err(|e| err("getGameConfigs", e))?;
        let branches = got
            .next()
            .expect("four bodies")
            .map_err(|e| err("getGameBranches", e))?;
        let deprecated = got
            .next()
            .expect("four bodies")
            .map_err(|e| err("getGameDeprecatedFileConfigs", e))?;
        let packages = got
            .next()
            .expect("four bodies")
            .map_err(|e| err("getGamePackages", e))?;

        let params = BranchParams::from_branches(&branches.bytes)?;

        let mut versions = VersionPair::select(&params, new, old)
            .map_err(|m| ApiError::shape("version pair", m))?;

        // The two `getBuild`s depend on `params` and on nothing each other. Their URLs
        // are built here rather than inside the task, because the task is
        // `'static` and `params` is neither.
        let tags = [versions.new.tag.clone(), versions.old.tag.clone()];
        let build_urls: Vec<String> = tags
            .iter()
            .map(|t| self.endpoints.get_build_url(&params, Some(t)))
            .collect();
        let mut got = self
            .fetcher
            .map_concurrently(&build_urls, jobs, |fetcher, i, url| async move {
                match fetcher.get_json(&url, Kind::Api).await {
                    Ok((bytes, _v)) => (i, Ok(bytes)),
                    Err(e) => (i, Err(e.to_string())),
                }
            })
            .await
            .into_iter();
        let body = |tag: &str, r: Result<Vec<u8>, String>| -> Result<CapturedBody, ApiError> {
            r.map(|bytes| CapturedBody {
                endpoint: "getBuild",
                file: format!("getBuild.{tag}.json"),
                seq: 0,
                bytes,
            })
            .map_err(|e| ApiError::shape("getBuild", e))
        };
        let build_new = body(&tags[0], got.next().expect("two builds"))?;
        let build_old = body(&tags[1], got.next().expect("two builds"))?;

        versions.attach_build_ids(
            &build_new.data::<GetBuild>()?.build_id,
            &build_old.data::<GetBuild>()?.build_id,
        );

        Ok(Captured {
            configs,
            branches,
            deprecated,
            packages,
            build_new,
            build_old,
            versions,
        })
    }
}

/// Fill in each body's request sequence number from the log.
///
/// Done after the fact and by lookup rather than threaded through, because the
/// sequence number is the fetcher's business and the capture's is to know where
/// its bytes ended up. Without this, [`CapturedBody::seq`]'s "filled in by the
/// caller" would be a promise nobody keeps: all six bodies would sit at `0`,
/// `log.set_stored_as(0, ..)` would find nothing to mark, and every `api` row in
/// `requests.ndjson` would record a null `stored_as` for a body sitting on disk
/// under `api/`. The manifest rows are correct either way, because
/// `manifest::mark_stored` receives a real `seq` from the fetch.
pub fn attach_seq(log: &RequestLog, bodies: &mut [&mut CapturedBody]) {
    for b in bodies.iter_mut() {
        // The endpoint name appears in the URL path, with or without a trailing
        // slash and with or without a query after it. Matched on the last
        // successful row so a retried request contributes one body, not two.
        //
        // **The tag has to be part of the match, or the two `getBuild` bodies
        // claim the same row.** `getBuild.7.0.0.json` and `getBuild.7.1.0.json`
        // both want a row whose URL contains `getBuild`, and there is a third —
        // the tag probe's `getBuild` for a tag the API does not serve, which
        // also answers HTTP 200 with a `retcode` in the body. So a
        // name-only match reversed over the log gives *both* build bodies the
        // probe's sequence number, the second `set_stored_as` overwrites the
        // first, and the bundle ends up with five of six bodies marked and a
        // probe row marked instead of a build row.
        //
        // The tag comes from the body's own file name, which the fetch built as
        // `getBuild.<tag>.json`, and the URL carries it as `?tag=<tag>`. Matching
        // both is what makes the assignment one-to-one.
        let tag = b
            .file
            .strip_prefix(&format!("{}.", b.endpoint))
            .and_then(|rest| rest.strip_suffix(".json"))
            .map(|t| format!("tag={t}"));
        let hit = log
            .records_matching(|r| {
                r.url.contains(b.endpoint)
                    && r.status == 200
                    && tag.as_deref().is_none_or(|t| r.url.contains(t))
            })
            .pop();
        b.seq = hit.map(|r| r.seq).unwrap_or(0);
    }
}

/// Render a document for writing under `api/`.
pub fn render_json(value: &impl serde::Serialize) -> Result<String, ApiError> {
    serde_json::to_string_pretty(value).map_err(|e| ApiError::shape("serialize", e))
}

#[cfg(test)]
mod attach_seq_tests {
    use super::*;
    use crate::http::RequestRecord;
    use std::collections::BTreeMap;

    fn body(endpoint: &'static str, file: &str) -> CapturedBody {
        CapturedBody {
            endpoint,
            file: file.to_string(),
            seq: 0,
            bytes: Vec::new(),
        }
    }

    /// One row per request, in the order the log recorded them.
    ///
    /// The record is spelled out field by field. A `Default`-derived one would
    /// follow every default a future edit adds, and this test is about *which
    /// row each body claims* — a fixture that could drift is a fixture that
    /// stops testing that.
    fn log_with(urls: &[&str]) -> RequestLog {
        let log = RequestLog::new();
        for (i, u) in urls.iter().enumerate() {
            log.push(RequestRecord {
                seq: i as u64 + 1,
                ts: "2026-01-02T03:04:05.000Z".to_string(),
                kind: crate::http::Kind::Api,
                method: "GET",
                url: (*u).to_string(),
                range: None,
                status: 200,
                final_url: (*u).to_string(),
                redirected: false,
                elapsed_ms: 1,
                req_headers: BTreeMap::new(),
                resp_headers: BTreeMap::new(),
                bytes_in: 0,
                stored_as: None,
                attempts: 1,
            });
        }
        log
    }

    #[test]
    fn two_bodies_for_one_endpoint_get_different_rows() {
        // The near-miss this function exists to avoid. `getBuild` is fetched
        // twice - once per build - and both bodies want a row whose URL contains
        // `getBuild`. Matching on the name alone and reversing over the log gives
        // both the *last* matching row, so one body overwrites the other and the
        // bundle records five of six.
        let log = log_with(&[
            "https://h/api/getGameConfigs?x=1",
            "https://s/api/getBuild?tag=7.1.0",
            "https://s/api/getBuild?tag=7.0.0",
        ]);
        let mut b = [
            body("getGameConfigs", "getGameConfigs.json"),
            body("getBuild", "getBuild.7.1.0.json"),
            body("getBuild", "getBuild.7.0.0.json"),
        ];
        let mut view: Vec<&mut CapturedBody> = b.iter_mut().collect();
        attach_seq(&log, &mut view);

        assert_eq!(b[0].seq, 1, "the only getGameConfigs row");
        assert_eq!(b[1].seq, 2, "the 7.1.0 build, not the later 7.0.0 one");
        assert_eq!(b[2].seq, 3);
        assert_ne!(
            b[1].seq, b[2].seq,
            "two bodies sharing one row is the bug this guards"
        );
    }

    #[test]
    fn a_body_never_takes_a_row_from_a_tag_it_did_not_ask_for() {
        // The tag probe asks for a tag the API does not serve. It answers HTTP
        // 200 with a `retcode` in the body, so `status == 200` does not exclude
        // it - and it is the *last* `getBuild` row in the log.
        let log = log_with(&[
            "https://s/api/getBuild?tag=7.1.0",
            "https://s/api/getBuild?tag=7.0.0",
            "https://s/api/getBuild?tag=6.7.0",
        ]);
        let mut b = [
            body("getBuild", "getBuild.7.1.0.json"),
            body("getBuild", "getBuild.7.0.0.json"),
        ];
        let mut view: Vec<&mut CapturedBody> = b.iter_mut().collect();
        attach_seq(&log, &mut view);
        assert_eq!(
            (b[0].seq, b[1].seq),
            (1, 2),
            "neither body takes the probe's row"
        );
    }

    #[test]
    fn a_body_with_no_matching_row_keeps_seq_zero() {
        // Zero is what `set_stored_as(0, ..)` looks up and does not find, so a
        // miss is visibly a miss rather than silently marking some other row.
        let log = log_with(&["https://h/api/getGameConfigs?x=1"]);
        let mut b = [body("getGameConfigs", "getGameConfigs.json")];
        let mut view: Vec<&mut CapturedBody> = b.iter_mut().collect();
        let mut c = body("getBuild", "getBuild.7.1.0.json");
        let mut view2: Vec<&mut CapturedBody> = std::iter::once(&mut c).collect();
        attach_seq(&log, &mut view);
        attach_seq(&log, &mut view2);
        assert_eq!(b[0].seq, 1);
        assert_eq!(
            c.seq, 0,
            "no getBuild row exists, so the body stays at zero"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biz::BranchParams;
    use crate::http::{RequestLog, Transport};
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};

    /// A scripted API, served over loopback. Own code, no borrowing.
    struct Mock {
        port: u16,
        seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    fn branch_body() -> String {
        r#"{"retcode":0,"message":"OK","data":{"game_branches":[{"game":{"id":"gopR6Cufr3","biz":"hk4e_global"},"main":{"package_id":"ScSYQBFhu9","branch":"main","password":"bDL4JUHL625x","tag":"7.1.0","diff_tags":["7.0.0","6.7.0"],"categories":[]}}]}}"#.into()
    }

    fn build_body(tag: &str, id: &str) -> String {
        format!(
            r#"{{"retcode":0,"message":"OK","data":{{"build_id":"{id}","tag":"{tag}","manifests":[{{"category_id":"10016","category_name":"g","manifest":{{"id":"manifest_aaaaaaaaaaaaaaaa_{id}","checksum":"{id}","compressed_size":"1","uncompressed_size":"2"}},"chunk_download":{{"encryption":0,"password":"","compression":1,"url_prefix":"https://c/{id}","url_suffix":""}},"manifest_download":{{"encryption":0,"password":"","compression":1,"url_prefix":"https://m/{id}","url_suffix":""}},"matching_field":"game","stats":{{"file_count":"1","chunk_count":"1"}}}}]}}}}"#
        )
    }

    /// Answers by path, so the order the run uses is visible in the assertions.
    fn spawn(routes: Vec<(&'static str, String)>) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        std::thread::spawn(move || {
            for (route, body) in routes {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                serve(stream, route, &body, &s);
            }
        });
        Mock { port, seen }
    }

    fn serve(
        mut stream: TcpStream,
        route: &str,
        body: &str,
        seen: &std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).ok();
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                break;
            }
        }
        seen.lock().unwrap().push(line.trim().to_string());
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body.as_bytes());
        let _ = stream.flush();
        assert!(
            line.contains(route),
            "the run requested {} but the script had {route}",
            line.trim()
        );
    }

    fn params() -> BranchParams {
        BranchParams::from_branches(branch_body().as_bytes()).unwrap()
    }

    fn mock_captured() -> Captured {
        let mut captured = Captured {
            configs: CapturedBody {
                endpoint: "getGameConfigs",
                file: "getGameConfigs.json".into(),
                seq: 0,
                bytes: br#"{"retcode":0,"data":{"launch_configs":[{"default_download_mode":"DOWNLOAD_MODE_CHUNK","audio_pkg_scan_dir":"GenshinImpact_Data/Persistent/audio_lang_14"}]}}"#.to_vec(),
            },
            branches: CapturedBody {
                endpoint: "getGameBranches",
                file: "getGameBranches.json".into(),
                seq: 0,
                bytes: branch_body().into_bytes(),
            },
            deprecated: CapturedBody {
                endpoint: "getGameDeprecatedFileConfigs",
                file: "getGameDeprecatedFileConfigs.json".into(),
                seq: 0,
                bytes: br#"{"retcode":0,"data":{"deprecated_file_configs":[]}}"#.to_vec(),
            },
            packages: CapturedBody {
                endpoint: "getGamePackages",
                file: "getGamePackages.json".into(),
                seq: 0,
                bytes: br#"{"retcode":0,"data":{"game_packages":[{"main":{"major":{"version":"5.5.0"},"patches":[]}}]}}"#.to_vec(),
            },
            build_new: CapturedBody {
                endpoint: "getBuild",
                file: "getBuild.7.1.0.json".into(),
                seq: 0,
                bytes: build_body("7.1.0", "newBUILD").into_bytes(),
            },
            build_old: CapturedBody {
                endpoint: "getBuild",
                file: "getBuild.7.0.0.json".into(),
                seq: 0,
                bytes: build_body("7.0.0", "oldBUILD").into_bytes(),
            },
            versions: VersionPair::select(
                &params(),
                NewTagSelector::BranchHead,
                OldTagSelector::FirstDiffTag,
            )
            .unwrap(),
        };
        captured.versions.attach_build_ids("newBUILD", "oldBUILD");
        captured
    }

    // ------------------------------------------------------------- the bodies

    /// The six documents, byte-exact and named the way a reader expects.
    #[test]
    fn a_capture_is_six_bodies_named_by_endpoint() {
        let c = mock_captured();
        let files: Vec<&str> = c.bodies().iter().map(|b| b.file.as_str()).collect();
        assert_eq!(
            files,
            vec![
                "getGameConfigs.json",
                "getGameBranches.json",
                "getGameDeprecatedFileConfigs.json",
                "getGamePackages.json",
                "getBuild.7.1.0.json",
                "getBuild.7.0.0.json",
            ]
        );
        for b in c.bodies() {
            assert!(!b.bytes.is_empty(), "{} is empty", b.file);
            // Every one re-parses as its wrapper: that is the acceptance clause.
            let w: Wrapper<serde_json::Value> = b.parse().expect("re-parses");
            assert_eq!(w.retcode, 0, "{}", b.file);
        }
    }

    /// Byte-exact means byte-exact: the stored bytes are the response, not a
    /// re-serialization of a typed view of it.
    #[test]
    fn the_stored_bytes_are_the_response_not_a_reserialization() {
        let c = mock_captured();
        let raw = br#"{"retcode":0,"data":{"launch_configs":[{"default_download_mode":"DOWNLOAD_MODE_CHUNK","audio_pkg_scan_dir":"GenshinImpact_Data/Persistent/audio_lang_14"}]}}"#;
        assert_eq!(c.configs.bytes, raw);
        // And a typed round-trip would *not* reproduce them: the whitespace is
        // the server's, and nothing here preserves it.
        let typed = c.game_configs().unwrap();
        assert_eq!(
            render_json(&typed).unwrap(),
            serde_json::to_string_pretty(&typed).unwrap()
        );
    }

    #[test]
    fn the_pair_is_resolved_from_the_captured_branch_not_carried_in() {
        let c = mock_captured();
        // Re-derived from the stored bytes.
        assert_eq!(c.branch_params().unwrap().package_id, "ScSYQBFhu9");
        assert_eq!(c.versions.new.tag, "7.1.0");
        assert_eq!(c.versions.old.tag, "7.0.0");
        assert_eq!(c.versions.new.build_id, "newBUILD");
    }

    // ------------------------------------------------------------- validation

    #[test]
    fn a_good_capture_validates() {
        mock_captured()
            .validate()
            .expect("the mock is self-consistent");
    }

    /// The one failure mode that would make every URL in the bundle right and the
    /// bundle wrong.
    #[test]
    fn a_build_that_echoes_another_tag_is_refused() {
        let mut c = mock_captured();
        c.build_new.bytes = build_body("7.0.0", "newBUILD").into_bytes();
        let e = c.validate().unwrap_err();
        assert!(e.to_string().contains("7.1.0"), "{e}");
        assert!(e.to_string().contains("7.0.0"), "{e}");
    }

    #[test]
    fn a_non_chunk_launcher_is_refused_before_a_blob_is_fetched() {
        let mut c = mock_captured();
        c.configs.bytes = br#"{"retcode":0,"data":{"launch_configs":[{"default_download_mode":"DOWNLOAD_MODE_LDIFF"}]}}"#.to_vec();
        let e = c.validate().unwrap_err();
        assert!(e.to_string().contains("DOWNLOAD_MODE_LDIFF"), "{e}");
        assert!(e.to_string().contains("DOWNLOAD_MODE_CHUNK"), "{e}");
    }

    #[test]
    fn an_undecodable_storage_declaration_is_refused() {
        let mut c = mock_captured();
        let s = String::from_utf8(c.build_new.bytes.clone()).unwrap();
        c.build_new.bytes = s
            .replacen("\"compression\":1", "\"compression\":0", 1)
            .into_bytes();
        let e = c.validate().unwrap_err();
        assert!(e.to_string().contains("compression"), "{e}");
    }

    #[test]
    fn legacy_staleness_is_derived_from_the_captured_legacy_body() {
        assert_eq!(mock_captured().legacy_staleness(), Some((2, vec![])));
    }

    // ------------------------------------------------------------- the client

    #[tokio::test]
    async fn capture_all_issues_the_six_requests_in_the_only_order_that_works() {
        let mock = spawn(vec![
            ("getGameConfigs", r#"{"retcode":0,"data":{"launch_configs":[{"default_download_mode":"DOWNLOAD_MODE_CHUNK","audio_pkg_scan_dir":"x"}]}}"#.into()),
            ("getGameBranches", branch_body()),
            ("getGameDeprecatedFileConfigs", r#"{"retcode":0,"data":{"deprecated_file_configs":[]}}"#.into()),
            ("getGamePackages", r#"{"retcode":0,"data":{"game_packages":[]}}"#.into()),
            ("getBuild", build_body("7.1.0", "newBUILD")),
            ("getBuild", build_body("7.0.0", "oldBUILD")),
        ]);
        let fetcher = Fetcher::new(Transport::local(), RequestLog::new()).unwrap();
        let hyp_base = format!("http://127.0.0.1:{}/hyp", mock.port);
        let sophon_base = format!("http://127.0.0.1:{}/sophon", mock.port);
        let client = HypClient::new(
            Endpoints::new(&hyp_base, &sophon_base),
            Biz::resolve(crate::cli::Biz::Hk4eGlobal),
            &fetcher,
        );
        let captured = client
            .capture_all(NewTagSelector::BranchHead, OldTagSelector::FirstDiffTag, 4)
            .await
            .expect("all six answered");

        assert_eq!(captured.bodies().len(), 6);
        assert_eq!(captured.versions.new.tag, "7.1.0");
        assert_eq!(captured.versions.new.build_id, "newBUILD");
        assert_eq!(captured.versions.old.build_id, "oldBUILD");

        // The order is the point: getBuild cannot be built before the branch is
        // known, and the mock asserts each route as it is served.
        let seen = mock.seen.lock().unwrap().clone();
        assert!(seen[0].contains("getGameConfigs"), "{seen:?}");
        assert!(seen[1].contains("getGameBranches"), "{seen:?}");
        assert!(seen[3].contains("getGamePackages"), "{seen:?}");
        assert!(seen[4].contains("tag=7.1.0"), "{seen:?}");
        assert!(seen[5].contains("tag=7.0.0"), "{seen:?}");
        assert_eq!(fetcher.log().len(), 6);
    }

    /// The channel tuple is only on one endpoint, and it is the per-launcher pair
    /// — a cn or bili capture sent `1/0` would be filtered against the wrong list.
    #[tokio::test]
    async fn only_the_deprecated_request_carries_the_channel_tuple() {
        let mock = spawn(vec![
            (
                "getGameConfigs",
                r#"{"retcode":0,"data":{"launch_configs":[]}}"#.into(),
            ),
            ("getGameBranches", branch_body()),
            (
                "getGameDeprecatedFileConfigs",
                r#"{"retcode":0,"data":{"deprecated_file_configs":[]}}"#.into(),
            ),
            (
                "getGamePackages",
                r#"{"retcode":0,"data":{"game_packages":[]}}"#.into(),
            ),
            ("getBuild", build_body("7.1.0", "n")),
            ("getBuild", build_body("7.0.0", "o")),
        ]);
        let fetcher = Fetcher::new(Transport::local(), RequestLog::new()).unwrap();
        let hyp_base = format!("http://127.0.0.1:{}/hyp", mock.port);
        let sophon_base = format!("http://127.0.0.1:{}/sophon", mock.port);
        let client = HypClient::new(
            Endpoints::new(&hyp_base, &sophon_base),
            Biz::resolve(crate::cli::Biz::Hk4eGlobal),
            &fetcher,
        );
        client
            .capture_all(NewTagSelector::BranchHead, OldTagSelector::FirstDiffTag, 4)
            .await
            .unwrap();
        let seen = mock.seen.lock().unwrap().clone();
        assert!(
            seen[2].contains("channel=1") && seen[2].contains("sub_channel=0"),
            "{:?}",
            seen[2]
        );
        for i in [0, 1, 3] {
            assert!(
                !seen[i].contains("channel="),
                "request {i} carried it: {}",
                seen[i]
            );
        }
    }

    /// An unservable tag is exit 2, named, with the URL in the message.
    #[tokio::test]
    async fn an_unservable_tag_fails_the_capture_by_name() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for _ in 0..6 {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                // Keyed on the request line rather than on the accept index:
                // `capture_all` issues four requests and then two, so an
                // answer queue keyed on arrival order is a serial test of a
                // concurrent client, and it fails for a reason that has
                // nothing to do with the unservable tag it is checking.
                let request_line = read_request_line(&stream);
                let body = if request_line.contains("getGameBranches") {
                    branch_body()
                } else if request_line.contains("getGameConfigs") {
                    r#"{"retcode":0,"data":{"launch_configs":[]}}"#.into()
                } else if request_line.contains("GameDeprecatedFileConfigs") {
                    r#"{"retcode":0,"data":{"deprecated_file_configs":[]}}"#.into()
                } else if request_line.contains("getGamePackages") {
                    r#"{"retcode":0,"data":{"game_packages":[]}}"#.into()
                } else if request_line.contains("tag=0.0.1") {
                    // The unservable old tag answers HTTP 200 and then says no.
                    r#"{"data":null,"message":"not found","retcode":-202}"#.to_string()
                } else {
                    build_body("7.1.0", "n")
                };
                serve_raw(stream, &body);
            }
        });
        let fetcher = Fetcher::new(Transport::local(), RequestLog::new()).unwrap();
        let hyp_base = format!("http://127.0.0.1:{port}/hyp");
        let sophon_base = format!("http://127.0.0.1:{port}/sophon");
        let client = HypClient::new(
            Endpoints::new(&hyp_base, &sophon_base),
            Biz::resolve(crate::cli::Biz::Hk4eGlobal),
            &fetcher,
        );
        let e = client
            .capture_all(
                NewTagSelector::BranchHead,
                OldTagSelector::Explicit("0.0.1"),
                4,
            )
            .await
            .expect_err("an unservable old tag is fatal");
        assert_eq!(e.exit_code(), crate::exit::ExitCode::Network);
        assert!(e.to_string().contains("tag=0.0.1"), "{e}");
        assert!(e.to_string().contains("-202"), "{e}");
    }

    /// Read a connection's request line and headers, returning the request line.
    ///
    /// Separate from [`serve_raw`] so a mock can decide **what** to answer
    /// before it answers it. The headers are consumed here, so `serve_raw`'s
    /// skip loop then hits end-of-stream and falls straight through.
    fn read_request_line(stream: &TcpStream) -> String {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                break;
            }
            if request_line.is_empty() {
                request_line = h.trim_end().to_string();
            }
        }
        request_line
    }

    /// Answer one connection with `body`, without reading it first.
    ///
    /// Deliberately does not consume the request. A caller that dispatches on the
    /// request line calls `read_request_line` itself; reading here as well would
    /// block, because the client has already sent its request and is now waiting
    /// for this response.
    fn serve_raw(mut stream: TcpStream, body: &str) {
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body.as_bytes());
        let _ = stream.flush();
    }

    // ----------------------------------------------------------- sequence ids

    #[test]
    fn each_body_finds_the_request_that_produced_it() {
        let log = RequestLog::new();
        for (url, tag) in [
            ("https://x/getGameConfigs", "api"),
            ("https://y/getBuild", "api"),
        ] {
            let seq = log.reserve();
            log.push(crate::http::RequestRecord {
                seq,
                ts: crate::http::now_rfc3339(),
                kind: Kind::Api,
                method: "GET",
                url: url.to_string(),
                range: None,
                status: 200,
                final_url: url.to_string(),
                redirected: false,
                elapsed_ms: 1,
                req_headers: Default::default(),
                resp_headers: Default::default(),
                bytes_in: 1,
                stored_as: None,
                attempts: 1,
            });
            let _ = tag;
        }
        let mut a = CapturedBody {
            endpoint: "getBuild",
            file: "f".into(),
            seq: 0,
            bytes: vec![],
        };
        let mut b = CapturedBody {
            endpoint: "getGameConfigs",
            file: "f".into(),
            seq: 0,
            bytes: vec![],
        };
        attach_seq(&log, &mut [&mut a, &mut b]);
        assert_eq!(a.seq, 2);
        assert_eq!(b.seq, 1);
    }

    #[test]
    fn documents_render_as_pretty_json_and_are_stable() {
        let a = render_json(&serde_json::json!({"b":1,"a":2})).unwrap();
        assert_eq!(
            a,
            serde_json::to_string_pretty(&serde_json::json!({"b":1,"a":2})).unwrap()
        );
        assert!(a.contains('\n'), "a document is not one long line");
    }
}
