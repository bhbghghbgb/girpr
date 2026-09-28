use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::Biz;

/// Minimal HoYoPlay client (Starward parity): getGameConfigs / getGameBranches /
/// getBuild (latest + local) / getGameDeprecatedFileConfigs.
/// Endpoint shapes follow `HoYoPlayClient` (hyp vs sophon bases, `launcher_id` +
/// `language` query) — see
/// https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.Core/HoYoPlay/HoYoPlayClient.cs#L108-L113
/// Request URLs are logged verbatim at debug.
pub struct HypClient {
    client: reqwest::Client,
    hyp_base: String,
    sophon_base: String,
    launcher_id: &'static str,
    game_id: &'static str,
    /// Per-launcher `channel`/`sub_channel` query values, from
    /// [`Biz::channel_tuple`]. Only `getGameDeprecatedFileConfigs` uses them.
    channel: &'static str,
    sub_channel: &'static str,
}

#[derive(Debug, Deserialize)]
struct Wrapper<T> {
    retcode: i64,
    message: Option<String>,
    data: Option<T>,
}

#[derive(Debug, Deserialize)]
struct NodeWrapper {
    retcode: i64,
    message: Option<String>,
    data: Option<serde_json::Value>,
}

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

#[derive(Debug, Clone, Deserialize)]
pub struct ChunkManifestMeta {
    #[serde(default)]
    pub category_id: String,
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

fn num_from_string<'de, D>(deserializer: D) -> std::result::Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    struct V;
    impl Visitor<'_> for V {
        type Value = i64;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("int or string int")
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<i64, E> {
            Ok(v)
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<i64, E> {
            Ok(v as i64)
        }
        fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<i64, E> {
            v.parse::<i64>().map_err(E::custom)
        }
        fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<i64, E> {
            v.parse::<i64>().map_err(E::custom)
        }
    }
    deserializer.deserialize_any(V)
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ManifestFile {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub checksum: String,
    #[serde(default, deserialize_with = "num_from_string")]
    pub compressed_size: i64,
    #[serde(default, deserialize_with = "num_from_string")]
    pub uncompressed_size: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ManifestUrl {
    #[serde(default)]
    pub url_prefix: String,
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
    pub fn new(biz: Biz) -> Result<Self> {
        let (host, _, _) = biz.endpoints();
        let (hyp_base, sophon_base) = production_bases(host);
        Self::new_with_bases(biz, hyp_base, sophon_base)
    }

    /// Test/offline constructor: point the HoYoPlay + Sophon APIs at a mock
    /// server (e.g. `http://127.0.0.1:<port>/hyp/hyp-connect/api`). Production
    /// code must use [`HypClient::new`].
    pub fn new_with_bases(biz: Biz, hyp_base: String, sophon_base: String) -> Result<Self> {
        let (_, launcher_id, game_id) = biz.endpoints();
        let (channel, sub_channel, _) = biz.channel_tuple();
        let client = reqwest::Client::builder()
            .user_agent("UnityPlayer/2019.4.40f1 (UnityWebRequest/1.0, libcurl/7.80.0-DEV)")
            .pool_max_idle_per_host(16)
            .build()
            .context("build http client")?;
        Ok(Self {
            client,
            hyp_base,
            sophon_base,
            launcher_id,
            game_id,
            channel,
            sub_channel,
        })
    }

    fn hyp_base(&self) -> String {
        self.hyp_base.clone()
    }

    fn sophon_base(&self) -> String {
        self.sophon_base.clone()
    }

    /// Build a HoYoPlay API URL.
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
        let mut last_err = anyhow::anyhow!("no attempts");
        for attempt in 1..=5 {
            let r = self.client.get(url).send().await;
            match r {
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.bytes().await.context("read body")?;
                    tracing::trace!(api = "hyp", url = %url, status = %status, bytes = body.len(), body = %String::from_utf8_lossy(&body), "response");
                    let w: NodeWrapper =
                        serde_json::from_slice(&body).context("parse hyp wrapper")?;
                    if w.retcode != 0 {
                        tracing::debug!(api = "hyp", url = %url, retcode = w.retcode, msg = ?w.message, attempt, "retcode != 0");
                        last_err = anyhow::anyhow!(
                            "hyp api retcode={} msg={:?} url={}",
                            w.retcode,
                            w.message,
                            url
                        );
                    } else {
                        let node_val = w
                            .data
                            .as_ref()
                            .and_then(|d| d.get(node))
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        tracing::debug!(api = "hyp", url = %url, node, attempt, "ok");
                        let v: T = serde_json::from_value(node_val).context("parse hyp node")?;
                        return Ok(v);
                    }
                }
                Err(e) => {
                    tracing::debug!(api = "hyp", url = %url, attempt, "http error: {}", e);
                    last_err = anyhow::anyhow!("http {}", e);
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(attempt)).await;
        }
        Err(last_err)
    }

    async fn get_direct<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        // Sophon getBuild: data IS the object (Starward CommonGetAsync without node).
        // Returns retcode -202 when tag unknown -> caller maps to None.
        tracing::debug!(api = "sophon", url = %url, "GET");
        let mut last_err = anyhow::anyhow!("no attempts");
        for attempt in 1..=5 {
            let resp = self.client.get(url).send().await;
            match resp {
                Ok(r) => {
                    let status = r.status();
                    let body = r.bytes().await.context("read body")?;
                    tracing::trace!(api = "sophon", url = %url, status = %status, bytes = body.len(), body = %String::from_utf8_lossy(&body), "response");
                    let w: Wrapper<T> = match serde_json::from_slice(&body) {
                        Ok(w) => w,
                        Err(e) => {
                            tracing::debug!(api = "sophon", url = %url, attempt, "parse error: {}", e);
                            last_err = anyhow::anyhow!("parse sophon wrapper: {}", e);
                            tokio::time::sleep(std::time::Duration::from_secs(attempt)).await;
                            continue;
                        }
                    };
                    if w.retcode != 0 {
                        tracing::debug!(api = "sophon", url = %url, retcode = w.retcode, msg = ?w.message, attempt, "retcode != 0");
                        last_err = anyhow::anyhow!(
                            "sophon api retcode={} msg={:?} url={}",
                            w.retcode,
                            w.message,
                            url
                        );
                        if w.retcode == -202 {
                            return Err(SophonNotFound.into());
                        }
                    } else if let Some(d) = w.data {
                        tracing::debug!(api = "sophon", url = %url, attempt, "ok");
                        return Ok(d);
                    } else {
                        last_err = anyhow::anyhow!("sophon api empty data url={}", url);
                    }
                }
                Err(e) => {
                    tracing::debug!(api = "sophon", url = %url, attempt, "http error: {}", e);
                    last_err = anyhow::anyhow!("http {}", e);
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(attempt)).await;
        }
        Err(last_err)
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
        self.get_direct(&sophon_build_url(&self.sophon_base(), pkg, tag))
            .await
    }

    pub async fn deprecated_files(&self) -> Result<Vec<String>> {
        let url = self.hyp_url("getGameDeprecatedFileConfigs", true);
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
    let mut url = format!(
        "{base}/getBuild?branch={}&package_id={}&password={}",
        urlencode(&pkg.branch),
        urlencode(&pkg.package_id),
        urlencode(&pkg.password)
    );
    if let Some(t) = tag {
        url.push_str(&format!("&tag={}", urlencode(t)));
    }
    url
}

/// The only `default_download_mode` this tool can patch. Genshin reports
/// `DOWNLOAD_MODE_CHUNK`; the other two (`DOWNLOAD_MODE_FILE`,
/// `DOWNLOAD_MODE_LDIFF`) need the 7z/hdiff paths that v1 does not implement.
/// See `Starward.Core/HoYoPlay/GameConfig.cs:DownloadMode`.
pub const DOWNLOAD_MODE_CHUNK: &str = "DOWNLOAD_MODE_CHUNK";

/// Production API bases for a host key (`mihoyo` vs overseas).
/// Split out so [`HypClient::new`] and tests share one mapping.
pub fn production_bases(host: &str) -> (String, String) {
    match host {
        "mihoyo" => (
            "https://hyp-api.mihoyo.com/hyp/hyp-connect/api".to_string(),
            "https://downloader-api.mihoyo.com/downloader/sophon_chunk/api".to_string(),
        ),
        _ => (
            "https://sg-hyp-api.hoyoverse.com/hyp/hyp-connect/api".to_string(),
            "https://sg-downloader-api.hoyoverse.com/downloader/sophon_chunk/api".to_string(),
        ),
    }
}

fn urlencode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-.~_".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{:02X}", b));
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapper_node_parse() {
        let body = r#"{"retcode":0,"message":"OK","data":{"game_branches":[{"main":{"package_id":"p","branch":"main","password":"","tag":"5.0.0","diff_tags":[]}}]}}"#;
        let w: NodeWrapper = serde_json::from_slice(body.as_bytes()).unwrap();
        let v: Vec<GameBranch> =
            serde_json::from_value(w.data.unwrap().get("game_branches").cloned().unwrap()).unwrap();
        assert_eq!(v[0].main.tag, "5.0.0");
    }

    #[test]
    fn num_from_string_both_shapes() {
        #[derive(Deserialize)]
        struct S {
            #[serde(deserialize_with = "num_from_string")]
            n: i64,
        }
        let a: S = serde_json::from_str(r#"{"n":12}"#).unwrap();
        let b: S = serde_json::from_str(r#"{"n":"34"}"#).unwrap();
        assert_eq!((a.n, b.n), (12, 34));
    }

    fn client_for(biz: Biz) -> HypClient {
        HypClient::new_with_bases(biz, "http://hyp.test/api".into(), "http://sophon.test/api".into())
            .unwrap()
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
            let url = c.hyp_url("getGameDeprecatedFileConfigs", true);
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
}
