use std::time::Duration;

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use girpr_hyp_wire::{USER_AGENT, deserialize_number_from_string};
// Kept public at this path: the repair facade and its fixtures import the
// classifier from `hyp`, even though the definition lives in the wire leaf.
pub use girpr_hyp_wire::terminal_retcode;

use crate::retry::{BackoffPolicy, Retry, retry};
use girpr_model::region::Region as Biz;
use girpr_model::region::{CHANNEL_ENDPOINT, Host};
use girpr_request_url::build_query_url;

/// How long to wait for a TCP/TLS connection to a HYP or Sophon endpoint.
///
/// A connection that cannot be established must yield to the retry loop instead
/// of stalling until the OS TCP timeout (minutes). Captured requests complete
/// end to end in a few seconds, so ten seconds just for the handshake is
/// generous.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Bounded deadline for a buffered JSON wrapper body read.
///
/// `tokio::time::timeout` wraps the whole `resp.bytes()` future once: this is a
/// **deadline on these small buffered bodies**, not a resetting per-byte idle
/// timer and not a client-wide total request timeout. A body that trickles one
/// byte every 29 s still hits it. That is intended — HYP/Sophon wrapper answers
/// are a few KiB and must resolve inside the retry budget — while large chunk
/// and manifest bodies go through `sophon::stream` and must keep working on a
/// slow link (no blanket total timeout there). The name is kept for continuity;
/// the bound is a fixed window from the start of the read.
const JSON_BODY_IDLE: Duration = Duration::from_secs(30);

/// Minimal HYP client (Starward parity): getGameConfigs / getGameBranches /
/// getBuild (latest + local) / getPatchBuild / getGameDeprecatedFileConfigs.
/// Endpoint shapes follow `HoYoPlayClient` (hyp vs sophon bases, `launcher_id` +
/// `language` query) — see
/// <https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.Core/HoYoPlay/HoYoPlayClient.cs#L108-L113>
/// Request URLs are logged verbatim at debug.
pub struct HypClient {
    client: reqwest::Client,
    hyp_base: reqwest::Url,
    sophon_base: reqwest::Url,
    launcher_id: &'static str,
    game_id: &'static str,
    /// Per-launcher `channel`/`sub_channel` query values, from
    /// [`Biz::channel_tuple`]. Only `getGameDeprecatedFileConfigs` uses them.
    channel: &'static str,
    sub_channel: &'static str,
    /// Retry wait policy: production uses exponential backoff with jitter,
    /// tests use [`BackoffPolicy::none`] so retries are instant.
    backoff: BackoffPolicy,
    /// Deadline for the two buffered JSON body reads. Production uses
    /// [`JSON_BODY_IDLE`]; the field exists so a test can stall a loopback body
    /// and exercise the deadline in milliseconds. It bounds the small buffered
    /// body only — never a total timeout on a streamed download.
    json_body_idle: Duration,
}

/// The Sophon JSON envelope. `data` is optional here: a `retcode:0` body with
/// no data is the API's "nothing here" answer, mapped by the caller below.
type Wrapper<T> = girpr_hyp_wire::Envelope<Option<T>>;

/// The HYP envelope, whose payload is a node object the caller picks apart.
type NodeWrapper = girpr_hyp_wire::Envelope<Option<serde_json::Value>>;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GameIdRef {
    pub id: String,
    pub biz: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GameConfig {
    #[serde(default)]
    pub exe_file_name: String,
    #[serde(default)]
    pub audio_pkg_scan_dir: String,
    #[serde(default)]
    pub audio_pkg_res_dir: String,
    #[serde(default)]
    pub audio_pkg_cache_dir: String,
    #[serde(default)]
    pub default_download_mode: String,
    #[serde(default)]
    pub res_category_dir: String,
    #[serde(default)]
    pub blacklist_dir: String,
    #[serde(default)]
    pub enable_resource_blacklist: bool,
    #[serde(default)]
    pub game: Option<GameIdRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GameBranch {
    #[serde(default)]
    pub main: GameBranchPackage,
    // Deserialized for completeness; no phase reads the pre-download branch.
    #[allow(dead_code)]
    #[serde(default)]
    pub pre_download: Option<GameBranchPackage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GameBranchPackage {
    #[serde(default)]
    pub package_id: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub diff_tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChunkBuild {
    #[serde(default)]
    pub build_id: String,
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub manifests: Vec<ChunkManifestMeta>,
}

/// The identity a response declares for itself, if any.
///
/// An absent or empty field carries no identity, so there is nothing it can
/// disagree with. Callers treat that as "unchecked", not as a match.
fn declared_identity(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.is_empty())
}

/// Return the declared identity when it is present and names something other
/// than the one the request asked for.
///
/// `requested` is the identity the run resolved for the request. An empty
/// `requested` is not an identity either (for example a branch tag the API
/// omitted), so it is not checked.
fn identity_mismatch<'a>(declared: Option<&'a str>, requested: &str) -> Option<&'a str> {
    if requested.is_empty() {
        return None;
    }
    declared_identity(declared).filter(|d| *d != requested)
}

impl GameConfig {
    /// The `game.biz` this response declares, when it disagrees with `biz`.
    ///
    /// The field is optional and may be empty; either way there is no identity
    /// to refuse.
    pub fn biz_mismatch(&self, biz: &str) -> Option<&str> {
        identity_mismatch(self.game.as_ref().map(|g| g.biz.as_str()), biz)
    }
}

impl ChunkBuild {
    /// The `tag` this response declares, when it disagrees with `tag`.
    ///
    /// An empty response tag carries no identity.
    pub fn tag_mismatch(&self, tag: &str) -> Option<&str> {
        identity_mismatch(Some(self.tag.as_str()), tag)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChunkManifestMeta {
    #[serde(default)]
    pub category_id: String,
    // Deserialized for completeness; the manifest is selected by category id.
    #[allow(dead_code)]
    #[serde(default)]
    pub category_name: String,
    #[serde(default)]
    pub matching_field: String,
    #[serde(default)]
    pub manifest: ManifestFile,
    #[serde(default)]
    pub chunk_download: ManifestUrl,
    #[serde(default)]
    pub manifest_download: ManifestUrl,
}

/// `getPatchBuild` response: one build identity plus its patch-manifest
/// descriptors. Every field defaults so an upstream field added later cannot
/// break a client that only knows today's shape.
#[derive(Debug, Clone, Deserialize)]
pub struct PatchBuild {
    #[serde(default)]
    pub build_id: String,
    #[serde(default)]
    pub patch_id: String,
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub manifests: Vec<PatchManifestMeta>,
}

/// One patch-manifest descriptor in a `getPatchBuild` response.
///
/// NOTE(starward-parity): the shape matches [`ChunkManifestMeta`] except for
/// `diff_download` (the patch bundle) in place of `chunk_download`. `stats` is
/// a per-tag object this tool never reads, so it is left out of the DTO and
/// ignored by serde rather than modelled — the same treatment given to the
/// download-descriptor keys this tool does not use.
#[derive(Debug, Clone, Deserialize)]
pub struct PatchManifestMeta {
    #[serde(default)]
    pub category_id: String,
    // Deserialized for completeness; the manifest is selected by category id.
    #[allow(dead_code)]
    #[serde(default)]
    pub category_name: String,
    #[serde(default)]
    pub matching_field: String,
    #[serde(default)]
    pub manifest: ManifestFile,
    #[serde(default)]
    pub diff_download: ManifestUrl,
    #[serde(default)]
    pub manifest_download: ManifestUrl,
}

/// A manifest's sizes as the API reports them.
///
/// Both sizes are hints, not promises. `uncompressed_size` is read (it sizes
/// the decode buffer) and a negative value is clamped to zero where it is used,
/// so a bogus field costs a skipped estimate or reallocation, not a failed
/// build. `compressed_size` is only deserialized; no phase reads it.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ManifestFile {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub checksum: String,
    // Deserialized for completeness; the manifest fetch uses `uncompressed_size`.
    #[allow(dead_code)]
    #[serde(default, deserialize_with = "deserialize_number_from_string")]
    pub compressed_size: i64,
    #[serde(default, deserialize_with = "deserialize_number_from_string")]
    pub uncompressed_size: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ManifestUrl {
    #[serde(default)]
    pub url_prefix: String,
    // Deserialized for completeness; only the prefix is joined with the id.
    #[allow(dead_code)]
    #[serde(default)]
    pub url_suffix: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeprecatedConfig {
    #[serde(default)]
    pub deprecated_files: Vec<DeprecatedFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeprecatedFile {
    #[serde(default)]
    pub name: String,
}

impl HypClient {
    /// The default constructor, kept for callers that do not override the
    /// bases.
    #[allow(dead_code)]
    pub fn new(biz: Biz) -> Result<Self> {
        let (host, _, _) = biz.endpoints();
        let (hyp_base, sophon_base) = production_bases(host);
        Self::new_with_bases(biz, hyp_base, sophon_base, BackoffPolicy::real())
    }

    /// Test/offline constructor: point the HYP + Sophon APIs at a mock
    /// server (e.g. `http://127.0.0.1:<port>/hyp/hyp-connect/api`). Production
    /// code must use [`HypClient::new`].
    pub fn new_with_bases(
        biz: Biz,
        hyp_base: String,
        sophon_base: String,
        backoff: BackoffPolicy,
    ) -> Result<Self> {
        let (_, launcher_id, game_id) = biz.endpoints();
        let (channel, sub_channel, _) = biz.channel_tuple();
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .pool_max_idle_per_host(16)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .context("build http client")?;
        let hyp_base = reqwest::Url::parse(&hyp_base).context("parse HYP base URL")?;
        let sophon_base = reqwest::Url::parse(&sophon_base).context("parse Sophon base URL")?;
        Ok(Self {
            client,
            hyp_base,
            sophon_base,
            launcher_id,
            game_id,
            channel,
            sub_channel,
            backoff,
            json_body_idle: JSON_BODY_IDLE,
        })
    }

    fn hyp_base(&self) -> &str {
        // `Url` adds a trailing slash to an origin-only base; the recorded
        // endpoint spelling appends its own, so drop it here rather than
        // emitting a double slash.
        self.hyp_base.as_str().trim_end_matches('/')
    }

    fn sophon_base(&self) -> &str {
        self.sophon_base.as_str().trim_end_matches('/')
    }

    /// Build a HYP API URL.
    ///
    /// NOTE(starward-parity): `with_channel` appends the **per-launcher**
    /// `channel`/`sub_channel` (`LauncherConfig.Channel` / `.SubChannel`:
    /// CN `1/1`, global `1/0`, bilibili `14/0`), not a fixed pair. Only
    /// `getGameDeprecatedFileConfigs` passes it; `getGameConfigs` and
    /// `getGameBranches` do not.
    /// See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.Core/HoYoPlay/HoYoPlayClient.cs#L106-L126
    /// and https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.Core/HoYoPlay/HoYoPlayClient.cs#L224
    fn hyp_url(&self, api: &str, with_channel: bool) -> String {
        let mut u = format!(
            "{}/{}?launcher_id={}&language=en-us&game_ids[]={}",
            self.hyp_base(),
            api,
            self.launcher_id,
            self.game_id
        );
        if with_channel {
            u.push_str(&format!(
                "&channel={}&sub_channel={}",
                self.channel, self.sub_channel
            ));
        }
        u
    }

    async fn get_node<T: DeserializeOwned>(&self, url: &str, node: &str) -> Result<T> {
        tracing::debug!(api = "hyp", url = %url, "GET");
        let deadline = self.json_body_idle;
        retry(5, &self.backoff, |attempt| async move {
            match self.client.get(url).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    // These JSON routes answer 200 and put failures in `retcode`;
                    // a non-2xx is a broken endpoint, not a usable wrapper. Refuse
                    // it before touching the body (the same check the blob path
                    // makes) so a 503 with `{"retcode":0}` cannot be mistaken for
                    // success. It is final: an identical request cannot fix a
                    // status the server has already decided.
                    if let Err(e) = resp.error_for_status_ref() {
                        return Retry::Fatal(
                            anyhow::Error::new(e)
                                .context(format!("hyp api status {status} url={url}")),
                        );
                    }
                    // A stalled body read must feed the retry loop. The bound is
                    // a deadline on these small buffered bodies (see
                    // `JSON_BODY_IDLE`), not a total request timeout, so large
                    // streamed downloads are untouched.
                    let body = match tokio::time::timeout(deadline, resp.bytes()).await {
                        Ok(Ok(body)) => body,
                        Ok(Err(e)) => {
                            return Retry::Retry(
                                anyhow::Error::from(e).context("read body"),
                            );
                        }
                        Err(_) => {
                            return Retry::Retry(anyhow::anyhow!(
                                "read body timed out after {deadline:?} url={url}"
                            ));
                        }
                    };
                    // `%` fields take their value by value, so the full-body
                    // lossy conversion would run on every response even with
                    // trace off. Render it only when a subscriber is listening.
                    if tracing::enabled!(tracing::Level::TRACE) {
                        tracing::trace!(api = "hyp", url = %url, status = %status, bytes = body.len(), body = %String::from_utf8_lossy(&body), "response");
                    }
                    let mut w: NodeWrapper = match serde_json::from_slice(&body) {
                        Ok(w) => w,
                        Err(e) => {
                            return Retry::Fatal(
                                anyhow::Error::from(e).context("parse hyp wrapper"),
                            );
                        }
                    };
                    if w.retcode != 0 {
                        tracing::debug!(api = "hyp", url = %url, retcode = w.retcode, msg = ?w.message, attempt, "retcode != 0");
                        let err = anyhow::anyhow!(
                            "hyp api retcode={} msg={:?} url={}",
                            w.retcode,
                            w.message,
                            url
                        );
                        if terminal_retcode(w.retcode) {
                            return Retry::Fatal(err);
                        }
                        Retry::Retry(err)
                    } else {
                        // Consume the node out of the owned wrapper: cloning a
                        // potentially large node just to deserialize it costs a
                        // full deep copy that is immediately dropped.
                        let node_val = w
                            .data
                            .as_mut()
                            .and_then(|d| d.as_object_mut())
                            .and_then(|o| o.remove(node))
                            .unwrap_or(serde_json::Value::Null);
                        tracing::debug!(api = "hyp", url = %url, node, attempt, "ok");
                        match serde_json::from_value(node_val) {
                            Ok(v) => Retry::Done(v),
                            Err(e) => Retry::Fatal(
                                anyhow::Error::from(e).context("parse hyp node"),
                            ),
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(api = "hyp", url = %url, attempt, "http error: {}", e);
                    Retry::Retry(anyhow::anyhow!("http {}", e))
                }
            }
        })
        .await
    }

    /// The JSON-wrapper exchange shared by both Sophon routes. `getBuild` is a
    /// GET; `getPatchBuild` is a POST with all its arguments in the query
    /// string (no body). Only the verb differs: the status gate, bounded body
    /// read, `Wrapper<T>` retcode policy and `-202` → [`SophonNotFound`] mapping
    /// are identical, so one implementation owns them and the two callers just
    /// pick a method.
    async fn json_wrapper<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        url: &str,
    ) -> Result<T> {
        tracing::debug!(api = "sophon", url = %url, method = %method, "request");
        let deadline = self.json_body_idle;
        retry(5, &self.backoff, |attempt| {
            // `request` consumes the method. Clone it per attempt so the retry
            // closure stays `FnMut` and every retry sends the same verb.
            let method = method.clone();
            async move {
                match self.client.request(method, url).send().await {
                    Ok(r) => {
                        let status = r.status();
                        // Same route-family rule as `get_node` and the blob path: a
                        // non-2xx here is a broken endpoint, not a wrapper to trust.
                        if let Err(e) = r.error_for_status_ref() {
                            return Retry::Fatal(
                                anyhow::Error::new(e)
                                    .context(format!("sophon api status {status} url={url}")),
                            );
                        }
                        let body = match tokio::time::timeout(deadline, r.bytes()).await {
                            Ok(Ok(body)) => body,
                            Ok(Err(e)) => {
                                return Retry::Retry(anyhow::Error::from(e).context("read body"));
                            }
                            Err(_) => {
                                return Retry::Retry(anyhow::anyhow!(
                                    "read body timed out after {deadline:?} url={url}"
                                ));
                            }
                        };
                        // Same lazy-body rule as the HYP response line above.
                        if tracing::enabled!(tracing::Level::TRACE) {
                            tracing::trace!(api = "sophon", url = %url, status = %status, bytes = body.len(), body = %String::from_utf8_lossy(&body), "response");
                        }
                        // The whole body is buffered, so a parse error is
                        // deterministic for these bytes: five backoffs cannot change
                        // it. Report it as a malformed response on the first attempt.
                        let w: Wrapper<T> = match serde_json::from_slice(&body) {
                            Ok(w) => w,
                            Err(e) => {
                                tracing::debug!(api = "sophon", url = %url, attempt, "parse error: {}", e);
                                return Retry::Fatal(
                                    anyhow::Error::from(e).context("parse sophon wrapper"),
                                );
                            }
                        };
                        if w.retcode != 0 {
                            tracing::debug!(api = "sophon", url = %url, retcode = w.retcode, msg = ?w.message, attempt, "retcode != 0");
                            let err = anyhow::anyhow!(
                                "sophon api retcode={} msg={:?} url={}",
                                w.retcode,
                                w.message,
                                url
                            );
                            if w.retcode == -202 {
                                // Distinct from the other terminal codes: the caller
                                // maps this one to "no local manifest" and falls back,
                                // where an invalid-params code is a hard stop.
                                return Retry::Fatal(SophonNotFound.into());
                            }
                            if terminal_retcode(w.retcode) {
                                return Retry::Fatal(err);
                            }
                            Retry::Retry(err)
                        } else if let Some(d) = w.data {
                            tracing::debug!(api = "sophon", url = %url, attempt, "ok");
                            Retry::Done(d)
                        } else {
                            // `retcode:0` with no data is the same "nothing here"
                            // answer as `-202`; give the caller its not-found marker
                            // instead of spending the retry budget on fixed bytes.
                            Retry::Fatal(SophonNotFound.into())
                        }
                    }
                    Err(e) => {
                        tracing::debug!(api = "sophon", url = %url, attempt, "http error: {}", e);
                        Retry::Retry(anyhow::anyhow!("http {}", e))
                    }
                }
            }
        })
        .await
    }

    // Sophon getBuild: data IS the object (Starward CommonGetAsync without node).
    // Returns retcode -202 when tag unknown -> caller maps to None.
    async fn get_direct<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        self.json_wrapper::<T>(reqwest::Method::GET, url).await
    }

    async fn post_direct<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        self.json_wrapper::<T>(reqwest::Method::POST, url).await
    }

    pub async fn game_config(&self) -> Result<GameConfig> {
        let url = self.hyp_url("getGameConfigs", false);
        let list: Vec<GameConfig> = self.get_node(&url, "launch_configs").await?;
        list.into_iter()
            .next()
            .context("empty launch_configs for this game")
    }

    pub async fn game_branch(&self) -> Result<GameBranch> {
        let url = self.hyp_url("getGameBranches", false);
        let list: Vec<GameBranch> = self.get_node(&url, "game_branches").await?;
        list.into_iter().next().context("empty game_branches")
    }

    pub async fn chunk_build(
        &self,
        pkg: &GameBranchPackage,
        tag: Option<&str>,
    ) -> Result<ChunkBuild> {
        self.get_direct(&sophon_build_url(self.sophon_base(), pkg, tag))
            .await
    }

    /// POST `getPatchBuild`: the patch build a branch package resolves to.
    ///
    /// NOTE(starward-parity): unlike `getBuild`, this route selects nothing by
    /// tag — the server ignores a `tag` query — so only the branch package's
    /// identity is sent, and all of it in the query string (no body).
    pub async fn get_patch_build(&self, pkg: &GameBranchPackage) -> Result<PatchBuild> {
        self.post_direct(&patch_build_url(self.sophon_base(), pkg))
            .await
    }

    pub async fn deprecated_files(&self) -> Result<Vec<String>> {
        let url = self.hyp_url(CHANNEL_ENDPOINT, true);
        let list: Vec<DeprecatedConfig> = self.get_node(&url, "deprecated_file_configs").await?;
        Ok(list
            .into_iter()
            .flat_map(|c| c.deprecated_files.into_iter().map(|f| f.name))
            .filter(|s| !s.is_empty())
            .collect())
    }

    pub fn http(&self) -> reqwest::Client {
        self.client.clone()
    }
}

#[derive(Debug)]
pub struct SophonNotFound;
impl std::fmt::Display for SophonNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sophon build not found (-202)")
    }
}
impl std::error::Error for SophonNotFound {}

/// Build the Sophon `getBuild` URL. `tag = None` asks for the latest build;
/// `Some(tag)` asks for that exact version (the server answers `retcode -202`
/// for a tag it no longer serves, which the caller treats as "no local build").
fn sophon_build_url(base: &str, pkg: &GameBranchPackage, tag: Option<&str>) -> String {
    let mut pairs = vec![
        ("branch", pkg.branch.as_str()),
        ("package_id", pkg.package_id.as_str()),
        ("password", pkg.password.as_str()),
    ];
    if let Some(t) = tag {
        pairs.push(("tag", t));
    }
    build_query_url(base, "getBuild", &pairs)
}

/// Build the Sophon `getPatchBuild` URL. Query-only, like `getBuild`, but with
/// no tag: the server ignores one, so sending `pkg.tag` would imply a version
/// selection this endpoint does not make. Percent-encoding is the same as for
/// `getBuild`, so a package id or password with spaces/special chars survives.
fn patch_build_url(base: &str, pkg: &GameBranchPackage) -> String {
    build_query_url(
        base,
        "getPatchBuild",
        &[
            ("branch", pkg.branch.as_str()),
            ("package_id", pkg.package_id.as_str()),
            ("password", pkg.password.as_str()),
        ],
    )
}

/// The only `default_download_mode` this tool can patch. GI reports
/// `DOWNLOAD_MODE_CHUNK`; the other two (`DOWNLOAD_MODE_FILE`,
/// `DOWNLOAD_MODE_LDIFF`) need the 7z/hdiff paths that v1 does not implement.
/// See `Starward.Core/HoYoPlay/GameConfig.cs:DownloadMode`.
pub use girpr_model::region::DOWNLOAD_MODE_CHUNK;

/// Production API bases for a host key (`mihoyo` vs overseas).
/// Split out so [`HypClient::new`] and the repair pipeline share one mapping.
/// The base strings come from `girpr-model`; the key-to-family mapping stays
/// here, so this binary keeps its own resolve policy for the host key.
pub fn production_bases(host: &str) -> (String, String) {
    let family = match host {
        "mihoyo" => Host::Mhy,
        _ => Host::Hyv,
    };
    (
        family.hyp_base().to_string(),
        family.sophon_base().to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_http::{serve_bodies, serve_with_status};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// The measured `retcode`s are terminal; everything else is not. `0` is the
    /// important negative: it is a *success*, and `getGameBranches` returns it
    /// with an empty node for a game id it does not know, which is a real answer
    /// the caller handles rather than a failure to retry.
    #[test]
    fn the_measured_terminal_retcodes_are_terminal_and_nothing_else_is() {
        for r in [-202, -201, -1001] {
            assert!(terminal_retcode(r), "{r} is invalid-params/not-found");
        }
        for r in [0, -1, -200, -2000] {
            assert!(!terminal_retcode(r), "{r} must not be treated as terminal");
        }
    }

    #[test]
    fn wrapper_node_parse() {
        let body = r#"{"retcode":0,"message":"OK","data":{"game_branches":[{"main":{"package_id":"p","branch":"main","password":"","tag":"5.0.0","diff_tags":[]}}]}}"#;
        let w: NodeWrapper = serde_json::from_slice(body.as_bytes()).unwrap();
        let v: Vec<GameBranch> =
            serde_json::from_value(w.data.unwrap().get("game_branches").cloned().unwrap()).unwrap();
        assert_eq!(v[0].main.tag, "5.0.0");
    }

    #[derive(Debug, Deserialize)]
    struct Size {
        #[serde(deserialize_with = "deserialize_number_from_string")]
        n: i64,
    }

    fn parse(json: &str) -> std::result::Result<Size, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn number_or_string_sizes_keep_the_full_i64_range() {
        let a = parse(r#"{"n":12}"#).unwrap();
        let b = parse(r#"{"n":"34"}"#).unwrap();
        assert_eq!((a.n, b.n), (12, 34));

        // The largest representable size is accepted in both shapes.
        let max_num = parse(r#"{"n":9223372036854775807}"#).unwrap();
        let max_str = parse(r#"{"n":"9223372036854775807"}"#).unwrap();
        assert_eq!((max_num.n, max_str.n), (i64::MAX, i64::MAX));

        // One past the top of the range, and a string that is not a number,
        // are rejected rather than wrapped into a negative size. The exact
        // serde error wording is the library's, not an output contract.
        assert!(parse(r#"{"n":9223372036854775808}"#).is_err());
        assert!(parse(r#"{"n":18446744073709551615}"#).is_err());
        assert!(parse(r#"{"n":"9223372036854775808"}"#).is_err());
        assert!(parse(r#"{"n":"abc"}"#).is_err());
    }

    /// A negative size is accepted rather than rejected: it is a hint, and the
    /// consumers clamp it to zero where they use it.
    #[test]
    fn negative_sizes_are_kept_for_the_consumer_to_clamp() {
        let num = parse(r#"{"n":-1}"#).unwrap();
        let string = parse(r#"{"n":"-1"}"#).unwrap();
        assert_eq!((num.n, string.n), (-1, -1));
    }

    fn client_for(biz: Biz) -> HypClient {
        HypClient::new_with_bases(
            biz,
            "http://hyp.test/api".into(),
            "http://sophon.test/api".into(),
            BackoffPolicy::none(),
        )
        .unwrap()
    }

    /// A loopback origin that announces a longer body than it writes and then
    /// holds the connection open. A JSON client can only finish `resp.bytes()`
    /// by hitting its deadline. Each connection gets its own task so retries
    /// open fresh sockets while an earlier stalled one is still held; the task
    /// ends when the timed-out client closes its side, so no sleep or manual
    /// shutdown is needed.
    async fn stalled_body_origin() -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let uri = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let accepted = Arc::clone(&hits);
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let hits = Arc::clone(&accepted);
                tokio::spawn(async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    let mut request = [0u8; 1024];
                    let _ = socket.read(&mut request).await;
                    // Announce 16 body bytes but write two. `resp.bytes()` waits
                    // for the remaining bytes, which never arrive.
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 16\r\nConnection: close\r\n\r\nok",
                        )
                        .await;
                    let _ = socket.flush().await;
                    // A read on a quiet connection is the stall: it returns only
                    // when the timed-out client drops the body and closes.
                    let mut probe = [0u8; 1];
                    let _ = socket.read(&mut probe).await;
                });
            }
        });
        (uri, hits)
    }

    /// A loopback origin that never completes its body but keeps it *arriving*:
    /// it announces a 4 KiB body, then sends one byte every `gap` — far inside
    /// the injected deadline — and stops after `cap` bytes. A resetting per-byte
    /// idle timer would keep pushing its deadline out while these arrive; only a
    /// fixed window from the start of the read still fires. Each connection gets
    /// its own task so retries open fresh sockets. A task exits as soon as the
    /// timed-out client closes its socket (the next write fails), with `cap` as
    /// a backstop so no task outlives the test.
    async fn dribbling_body_origin(gap: Duration, cap: usize) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let uri = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let accepted = Arc::clone(&hits);
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let hits = Arc::clone(&accepted);
                tokio::spawn(async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    let mut request = [0u8; 1024];
                    let _ = socket.read(&mut request).await;
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 4096\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                    let _ = socket.flush().await;
                    for _ in 0..cap {
                        tokio::time::sleep(gap).await;
                        // One byte per gap, each far inside the deadline. A
                        // per-byte idle reset would never reach zero here.
                        if socket.write_all(b"x").await.is_err() {
                            break;
                        }
                        let _ = socket.flush().await;
                    }
                    // Quiet from here; a fixed deadline fired long before cap.
                    let mut probe = [0u8; 1];
                    let _ = socket.read(&mut probe).await;
                });
            }
        });
        (uri, hits)
    }

    /// A body that never completes must hit the injected deadline in both JSON
    /// paths. The origin announces 16 body bytes, sends two, then goes quiet,
    /// so `resp.bytes()` can only finish by hitting the bound. That proves the
    /// buffered read is bounded; pinning the non-resetting property needs a
    /// separate fixture that keeps arriving in sub-deadline gaps (the fixed
    /// window itself is specified on `JSON_BODY_IDLE` above). The failure is
    /// transport-class, so it is retried with the zero-wait test policy and
    /// finally reported with the deadline in the error, not as a parse failure
    /// or a silent success.
    #[tokio::test]
    async fn both_json_paths_time_out_a_stalled_body() {
        let deadline = Duration::from_millis(60);
        let (uri, hits) = stalled_body_origin().await;
        let mut client = client_for(Biz::Hk4eGlobal);
        client.json_body_idle = deadline;

        let node_err = tokio::time::timeout(Duration::from_secs(5), async {
            client
                .get_node::<Vec<GameConfig>>(&uri, "launch_configs")
                .await
        })
        .await
        .expect("the stalled body must hit the injected deadline")
        .expect_err("a body that never completes cannot succeed");
        assert!(
            format!("{node_err:#}").contains("read body timed out"),
            "the deadline must be the reported cause: {node_err:#}"
        );

        let direct_err = tokio::time::timeout(Duration::from_secs(5), async {
            client.get_direct::<ChunkBuild>(&uri).await
        })
        .await
        .expect("the stalled body must hit the injected deadline")
        .expect_err("a body that never completes cannot succeed");
        assert!(
            format!("{direct_err:#}").contains("read body timed out"),
            "the deadline must be the reported cause: {direct_err:#}"
        );
        assert!(
            hits.load(Ordering::SeqCst) >= 1,
            "the loopback origin must have seen a request"
        );
    }

    /// `JSON_BODY_IDLE` is a fixed deadline on the buffered body, not a
    /// per-byte resetting idle timer. The origin above goes quiet after two
    /// bytes, so it proves only that the read is bounded — either deadline
    /// semantic would satisfy it. This fixture is the one that distinguishes
    /// them: a byte arrives every 20 ms (five times inside the 100 ms injected
    /// deadline), so a resetting idle timer would never fire while they keep
    /// coming. Both JSON paths must still report the fixed deadline, and the
    /// call must finish inside the outer guard rather than tracking the
    /// origin's multi-second dribble. No total timeout is added to blob streams.
    #[tokio::test]
    async fn a_sub_deadline_byte_dribble_still_hits_the_fixed_body_deadline() {
        let deadline = Duration::from_millis(100);
        // 20 ms gaps and 400 bytes (8 s of dribbling) give a resetting timer no
        // gap to expire in and outlast the 6 s outer guard; a fixed window fires
        // after ~5 * 100 ms of retries and closes each socket early.
        let (uri, hits) = dribbling_body_origin(Duration::from_millis(20), 400).await;
        let mut client = client_for(Biz::Hk4eGlobal);
        client.json_body_idle = deadline;

        let node_err = tokio::time::timeout(Duration::from_secs(6), async {
            client
                .get_node::<Vec<GameConfig>>(&uri, "launch_configs")
                .await
        })
        .await
        .expect("a continuous sub-deadline dribble must still hit the fixed deadline")
        .expect_err("an incomplete body cannot succeed");
        assert!(
            format!("{node_err:#}").contains("read body timed out"),
            "the fixed deadline must be the reported cause: {node_err:#}"
        );

        let direct_err = tokio::time::timeout(Duration::from_secs(6), async {
            client.get_direct::<ChunkBuild>(&uri).await
        })
        .await
        .expect("a continuous sub-deadline dribble must still hit the fixed deadline")
        .expect_err("an incomplete body cannot succeed");
        assert!(
            format!("{direct_err:#}").contains("read body timed out"),
            "the fixed deadline must be the reported cause: {direct_err:#}"
        );

        assert!(
            hits.load(Ordering::SeqCst) >= 2,
            "both JSON paths must have reached the origin on fresh connections"
        );
    }

    /// A retryable retcode on the first call and a real node on the second: the
    /// caller must see the second answer, and the server both requests.
    #[tokio::test]
    async fn get_node_retries_a_retryable_retcode_and_takes_the_next_answer() {
        let server = serve_bodies(vec![
            r#"{"retcode":-1,"message":"busy","data":null}"#,
            r#"{"retcode":0,"message":"OK","data":{"launch_configs":[{"exe_file_name":"GenshinImpact.exe"}]}}"#,
        ])
        .await;
        let list: Vec<GameConfig> = client_for(Biz::Hk4eGlobal)
            .get_node(&server.uri(), "launch_configs")
            .await
            .expect("the second attempt answers");
        assert_eq!(server.hits().await, 2);
        assert_eq!(list[0].exe_file_name, "GenshinImpact.exe");
    }

    /// An invalid-params retcode is answered identically every time, so the
    /// first answer must end the call.
    #[tokio::test]
    async fn get_node_stops_on_a_terminal_retcode() {
        let server = serve_bodies(vec![
            r#"{"retcode":-1001,"message":"invalid params","data":null}"#,
        ])
        .await;
        let err = client_for(Biz::Hk4eGlobal)
            .get_node::<Vec<GameConfig>>(&server.uri(), "launch_configs")
            .await
            .expect_err("invalid params is final");
        assert_eq!(
            server.hits().await,
            1,
            "a terminal retcode must not be retried"
        );
        let chain = format!("{err:#}");
        assert!(chain.contains("-1001"), "{chain}");
        assert!(
            !chain.contains("after 5 attempts"),
            "a fatal answer must not be dressed up as exhaustion: {chain}"
        );
    }

    /// `get_direct` uses the same retry rule: a transient failure is retried,
    /// and the successful body is returned as the node itself.
    #[tokio::test]
    async fn get_direct_retries_a_retryable_retcode_and_takes_the_next_answer() {
        let server = serve_bodies(vec![
            r#"{"retcode":-1,"message":"busy","data":null}"#,
            r#"{"retcode":0,"message":"OK","data":{"build_id":"build_test"}}"#,
        ])
        .await;
        let build: ChunkBuild = client_for(Biz::Hk4eGlobal)
            .get_direct(&server.uri())
            .await
            .expect("the second attempt answers");
        assert_eq!(server.hits().await, 2);
        assert_eq!(build.build_id, "build_test");
    }

    /// `-202` is terminal *and* has its own meaning: the caller must get
    /// [`SophonNotFound`] after one request, not a retried generic error.
    #[tokio::test]
    async fn get_direct_stops_on_a_fatal_retcode() {
        let server = serve_bodies(vec![
            r#"{"retcode":-202,"message":"not found","data":null}"#,
        ])
        .await;
        let err = client_for(Biz::Hk4eGlobal)
            .get_direct::<ChunkBuild>(&server.uri())
            .await
            .expect_err("an unknown tag is final");
        assert_eq!(
            server.hits().await,
            1,
            "a fatal retcode must not be retried"
        );
        assert!(
            err.downcast_ref::<SophonNotFound>().is_some(),
            "the caller needs the not-found marker: {err:#}"
        );
    }

    /// A non-2xx status is decided before the body: a `503` carrying a wrapper
    /// that used to look like success must not be accepted, in either JSON path.
    #[tokio::test]
    async fn both_json_paths_refuse_a_non_2xx_before_reading_the_body() {
        let ok_node = r#"{"retcode":0,"message":"OK","data":{"launch_configs":[{"exe_file_name":"GenshinImpact.exe"}]}}"#;
        let node_server = serve_with_status(503, vec![ok_node]).await;
        let node_err = client_for(Biz::Hk4eGlobal)
            .get_node::<Vec<GameConfig>>(&node_server.uri(), "launch_configs")
            .await
            .expect_err("a 503 is not a successful wrapper");
        assert_eq!(node_server.hits().await, 1, "a decided status is final");
        assert!(format!("{node_err:#}").contains("503"), "{node_err:#}");

        let ok_build = r#"{"retcode":0,"message":"OK","data":{"build_id":"build_test"}}"#;
        let direct_server = serve_with_status(503, vec![ok_build]).await;
        let direct_err = client_for(Biz::Hk4eGlobal)
            .get_direct::<ChunkBuild>(&direct_server.uri())
            .await
            .expect_err("a 503 is not a successful wrapper");
        assert_eq!(direct_server.hits().await, 1, "a decided status is final");
        assert!(format!("{direct_err:#}").contains("503"), "{direct_err:#}");
    }

    /// Malformed wrapper bytes are deterministic, so `get_direct` must report
    /// them on the first attempt instead of spending five backoffs.
    #[tokio::test]
    async fn get_direct_treats_a_malformed_body_as_fatal() {
        let server = serve_bodies(vec!["<html>not json</html>"]).await;
        let err = client_for(Biz::Hk4eGlobal)
            .get_direct::<ChunkBuild>(&server.uri())
            .await
            .expect_err("a malformed wrapper is fatal");
        assert_eq!(
            server.hits().await,
            1,
            "identical bytes cannot parse better"
        );
        let chain = format!("{err:#}");
        assert!(chain.contains("parse sophon wrapper"), "{chain}");
        assert!(!chain.contains("after 5 attempts"), "{chain}");
    }

    /// `retcode:0` with no data is a clean not-found, not five retries ending
    /// in a generic "empty data" error.
    #[tokio::test]
    async fn get_direct_maps_data_null_to_a_clean_not_found() {
        let server = serve_bodies(vec![r#"{"retcode":0,"message":"OK","data":null}"#]).await;
        let err = client_for(Biz::Hk4eGlobal)
            .get_direct::<ChunkBuild>(&server.uri())
            .await
            .expect_err("no data is a not-found");
        assert_eq!(server.hits().await, 1, "the answer is final");
        assert!(
            err.downcast_ref::<SophonNotFound>().is_some(),
            "the caller needs the not-found marker: {err:#}"
        );
    }

    /// The channel query is per-launcher (`LauncherConfig.Channel/SubChannel`),
    /// not a fixed pair: bilibili must not be sent the global `1/0`.
    #[test]
    fn hyp_url_carries_the_per_launcher_channel() {
        for (biz, launcher, game, ch, sub) in [
            (Biz::Hk4eCn, "jGHBHlcOq1", "1Z8W5NHUQb", "1", "1"),
            (Biz::Hk4eGlobal, "VYTpXlbWo8", "gopR6Cufr3", "1", "0"),
            (Biz::Hk4eBilibili, "umfgRO5gh5", "T2S0Gz4Dr2", "14", "0"),
        ] {
            let c = client_for(biz);
            let url = c.hyp_url(CHANNEL_ENDPOINT, true);
            assert_eq!(
                url,
                format!(
                    "http://hyp.test/api/getGameDeprecatedFileConfigs?launcher_id={launcher}&language=en-us&game_ids[]={game}&channel={ch}&sub_channel={sub}"
                ),
                "{biz:?}"
            );
            // Channel-less APIs must not carry the params at all.
            let plain = c.hyp_url("getGameConfigs", false);
            assert!(!plain.contains("channel="), "{plain}");
            assert!(!plain.contains("sub_channel="), "{plain}");
        }
    }

    #[test]
    fn sophon_build_url_joins_branch_and_omits_tag_for_latest() {
        let pkg = GameBranchPackage {
            package_id: "pkg id".into(),
            branch: "main".into(),
            password: "p@ss/word".into(),
            tag: "5.1.0".into(),
            diff_tags: vec![],
        };
        let base = "http://sophon.test/api";
        assert_eq!(
            sophon_build_url(base, &pkg, None),
            "http://sophon.test/api/getBuild?branch=main&package_id=pkg%20id&password=p%40ss%2Fword"
        );
        assert_eq!(
            sophon_build_url(base, &pkg, Some("5.0.0")),
            "http://sophon.test/api/getBuild?branch=main&package_id=pkg%20id&password=p%40ss%2Fword&tag=5.0.0"
        );
    }

    /// `getPatchBuild` takes branch/package/password in the query and no tag.
    /// The server ignores `tag` here, so the client must not send one —
    /// including a `pkg.tag` that happens to be populated.
    #[test]
    fn patch_build_url_is_query_only_and_has_no_tag() {
        let pkg = GameBranchPackage {
            package_id: "pkg id".into(),
            branch: "main".into(),
            password: "p@ss/word".into(),
            tag: "5.1.0".into(),
            diff_tags: vec![],
        };
        let url = patch_build_url("http://sophon.test/api", &pkg);
        assert_eq!(
            url,
            "http://sophon.test/api/getPatchBuild?branch=main&package_id=pkg%20id&password=p%40ss%2Fword"
        );
        assert!(!url.contains("tag="), "{url}");
    }

    /// Patch-manifest sizes arrive as JSON strings as well as numbers; the
    /// download descriptors' unused keys and the per-tag `stats` object are
    /// unknown to the DTO and must not fail the parse.
    #[test]
    fn patch_build_deserialises_string_sizes_and_ignores_unused_keys() {
        let body = r#"{
            "build_id":"b","patch_id":"p","tag":"5.1.0",
            "manifests":[{
                "category_id":"1","category_name":"res","matching_field":"",
                "manifest":{"id":"m","checksum":"c","compressed_size":"12","uncompressed_size":34},
                "diff_download":{"encryption":"none","password":"","compression":"zstd","url_prefix":"https://diff/","url_suffix":"?x=1"},
                "manifest_download":{"url_prefix":"https://manifest/"},
                "stats":{"5.0.0":{"count":1}}
            }]
        }"#;
        let build: PatchBuild = serde_json::from_str(body).unwrap();
        assert_eq!(build.build_id, "b");
        assert_eq!(build.patch_id, "p");
        assert_eq!(build.tag, "5.1.0");
        assert_eq!(build.manifests.len(), 1);
        let meta = &build.manifests[0];
        assert_eq!(meta.category_id, "1");
        assert_eq!(meta.manifest.compressed_size, 12);
        assert_eq!(meta.manifest.uncompressed_size, 34);
        assert_eq!(meta.diff_download.url_prefix, "https://diff/");
        assert_eq!(meta.manifest_download.url_prefix, "https://manifest/");
    }

    /// `get_patch_build` goes through the shared wrapper rules but with the
    /// POST verb; the whole request is the query string, with no body or tag.
    #[tokio::test]
    async fn get_patch_build_posts_the_query_and_decodes_the_wrapper() {
        use girpr_loopback_mock::{Reply, ScriptedServer};

        let server = ScriptedServer::start(vec![Reply::new(
            200,
            r#"{"retcode":0,"message":"OK","data":{"build_id":"b","patch_id":"p","tag":"5.1.0","manifests":[]}}"#,
        )]);
        let client = HypClient::new_with_bases(
            Biz::Hk4eGlobal,
            "http://hyp.test/api".into(),
            server.uri(),
            BackoffPolicy::none(),
        )
        .unwrap();
        let pkg = GameBranchPackage {
            package_id: "pkg id".into(),
            branch: "main".into(),
            password: "p@ss/word".into(),
            tag: "5.1.0".into(),
            diff_tags: vec![],
        };

        let build = client.get_patch_build(&pkg).await.unwrap();
        assert_eq!(build.build_id, "b");

        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(
            requests[0].target,
            "/getPatchBuild?branch=main&package_id=pkg%20id&password=p%40ss%2Fword"
        );
    }

    /// Only a present, non-empty identity that names something else is a
    /// mismatch; absence and an empty value carry nothing to check, and an
    /// empty request has no identity to compare against.
    #[test]
    fn identity_mismatch_checks_only_a_present_different_value() {
        assert_eq!(identity_mismatch(None, "5.1.0"), None);
        assert_eq!(identity_mismatch(Some(""), "5.1.0"), None);
        assert_eq!(identity_mismatch(Some("5.1.0"), "5.1.0"), None);
        assert_eq!(identity_mismatch(Some("5.0.0"), "5.1.0"), Some("5.0.0"));
        assert_eq!(identity_mismatch(Some("5.0.0"), ""), None);
    }

    fn config_with_game(game: Option<GameIdRef>) -> GameConfig {
        GameConfig {
            exe_file_name: String::new(),
            audio_pkg_scan_dir: String::new(),
            audio_pkg_res_dir: String::new(),
            audio_pkg_cache_dir: String::new(),
            default_download_mode: String::new(),
            res_category_dir: String::new(),
            blacklist_dir: String::new(),
            enable_resource_blacklist: false,
            game,
        }
    }

    #[test]
    fn game_config_biz_mismatch_is_reported_only_when_declared() {
        let absent = config_with_game(None);
        assert_eq!(absent.biz_mismatch("hk4e_global"), None);

        let empty = config_with_game(Some(GameIdRef {
            id: "gopR6Cufr3".into(),
            biz: String::new(),
        }));
        assert_eq!(empty.biz_mismatch("hk4e_global"), None);

        let matching = config_with_game(Some(GameIdRef {
            id: "gopR6Cufr3".into(),
            biz: "hk4e_global".into(),
        }));
        assert_eq!(matching.biz_mismatch("hk4e_global"), None);

        let different = config_with_game(Some(GameIdRef {
            id: "1Z8W5NHUQb".into(),
            biz: "hk4e_cn".into(),
        }));
        assert_eq!(
            different.biz_mismatch("hk4e_global"),
            Some("hk4e_cn"),
            "a present biz for another launcher is a mismatch"
        );
    }

    #[test]
    fn chunk_build_tag_mismatch_is_reported_only_when_declared() {
        let build = |tag: &str| ChunkBuild {
            build_id: "b".into(),
            tag: tag.into(),
            manifests: vec![],
        };
        assert_eq!(build("").tag_mismatch("5.1.0"), None);
        assert_eq!(build("5.1.0").tag_mismatch("5.1.0"), None);
        assert_eq!(build("5.0.0").tag_mismatch("5.1.0"), Some("5.0.0"));
        assert_eq!(build("5.0.0").tag_mismatch(""), None);
    }
}
