use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::Biz;

/// Minimal HoYoPlay client (Starward parity): getGameConfigs / getGameBranches /
/// getBuild (latest + local) / getGameDeprecatedFileConfigs.
pub struct HypClient {
    client: reqwest::Client,
    host: &'static str,
    launcher_id: &'static str,
    game_id: &'static str,
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

#[derive(Debug, Clone, Deserialize)]
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

impl Default for ManifestFile {
    fn default() -> Self {
        Self {
            id: String::new(),
            checksum: String::new(),
            compressed_size: 0,
            uncompressed_size: 0,
        }
    }
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
        let (host, launcher_id, game_id) = biz.endpoints();
        let client = reqwest::Client::builder()
            .user_agent("UnityPlayer/2019.4.40f1 (UnityWebRequest/1.0, libcurl/7.80.0-DEV)")
            .pool_max_idle_per_host(16)
            .build()
            .context("build http client")?;
        Ok(Self {
            client,
            host,
            launcher_id,
            game_id,
        })
    }

    fn hyp_base(&self) -> String {
        match self.host {
            "mihoyo" => "https://hyp-api.mihoyo.com/hyp/hyp-connect/api".to_string(),
            _ => "https://sg-hyp-api.hoyoverse.com/hyp/hyp-connect/api".to_string(),
        }
    }

    fn sophon_base(&self) -> String {
        match self.host {
            "mihoyo" => "https://downloader-api.mihoyo.com/downloader/sophon_chunk/api".to_string(),
            _ => "https://sg-downloader-api.hoyoverse.com/downloader/sophon_chunk/api".to_string(),
        }
    }

    fn hyp_url(&self, api: &str, with_channel: bool) -> String {
        let mut u = format!(
            "{}/{}?launcher_id={}&language=en-us&game_ids[]={}",
            self.hyp_base(),
            api,
            self.launcher_id,
            self.game_id
        );
        if with_channel {
            u.push_str("&channel=1&sub_channel=0");
        }
        u
    }

    async fn get_node<T: DeserializeOwned>(&self, url: &str, node: &str) -> Result<T> {
        tracing::debug!(api = "hyp", url = %redact(url), "GET");
        let mut last_err = anyhow::anyhow!("no attempts");
        for attempt in 1..=5 {
            let r = self.client.get(url).send().await;
            match r {
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.bytes().await.context("read body")?;
                    tracing::trace!(api = "hyp", url = %redact(url), status = %status, bytes = body.len(), body = %String::from_utf8_lossy(&body), "response");
                    let w: NodeWrapper =
                        serde_json::from_slice(&body).context("parse hyp wrapper")?;
                    if w.retcode != 0 {
                        tracing::debug!(api = "hyp", url = %redact(url), retcode = w.retcode, msg = ?w.message, attempt, "retcode != 0");
                        last_err = anyhow::anyhow!(
                            "hyp api retcode={} msg={:?} url={}",
                            w.retcode,
                            w.message,
                            redact(url)
                        );
                    } else {
                        let node_val = w
                            .data
                            .as_ref()
                            .and_then(|d| d.get(node))
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        tracing::debug!(api = "hyp", url = %redact(url), node, attempt, "ok");
                        let v: T = serde_json::from_value(node_val).context("parse hyp node")?;
                        return Ok(v);
                    }
                }
                Err(e) => {
                    tracing::debug!(api = "hyp", url = %redact(url), attempt, "http error: {}", e);
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
        tracing::debug!(api = "sophon", url = %redact(url), "GET");
        let mut last_err = anyhow::anyhow!("no attempts");
        for attempt in 1..=5 {
            let resp = self.client.get(url).send().await;
            match resp {
                Ok(r) => {
                    let status = r.status();
                    let body = r.bytes().await.context("read body")?;
                    tracing::trace!(api = "sophon", url = %redact(url), status = %status, bytes = body.len(), body = %String::from_utf8_lossy(&body), "response");
                    let w: Wrapper<T> = match serde_json::from_slice(&body) {
                        Ok(w) => w,
                        Err(e) => {
                            tracing::debug!(api = "sophon", url = %redact(url), attempt, "parse error: {}", e);
                            last_err = anyhow::anyhow!("parse sophon wrapper: {}", e);
                            tokio::time::sleep(std::time::Duration::from_secs(attempt)).await;
                            continue;
                        }
                    };
                    if w.retcode != 0 {
                        tracing::debug!(api = "sophon", url = %redact(url), retcode = w.retcode, msg = ?w.message, attempt, "retcode != 0");
                        last_err = anyhow::anyhow!(
                            "sophon api retcode={} msg={:?} url={}",
                            w.retcode,
                            w.message,
                            redact(url)
                        );
                        if w.retcode == -202 {
                            return Err(SophonNotFound.into());
                        }
                    } else if let Some(d) = w.data {
                        tracing::debug!(api = "sophon", url = %redact(url), attempt, "ok");
                        return Ok(d);
                    } else {
                        last_err = anyhow::anyhow!("sophon api empty data url={}", redact(url));
                    }
                }
                Err(e) => {
                    tracing::debug!(api = "sophon", url = %redact(url), attempt, "http error: {}", e);
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
        let mut url = format!(
            "{}/getBuild?branch={}&package_id={}&password={}",
            self.sophon_base(),
            urlencode(&pkg.branch),
            urlencode(&pkg.package_id),
            urlencode(&pkg.password)
        );
        if let Some(t) = tag {
            url.push_str(&format!("&tag={}", urlencode(t)));
        }
        self.get_direct(&url).await
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

/// Redact `password=...` query values (sophon getBuild URLs carry the branch password).
fn redact(url: &str) -> String {
    let mut out = url.to_string();
    let mut i = 0;
    while let Some(pos) = out[i..].find("password=") {
        let start = i + pos + "password=".len();
        let end = out[start..]
            .find('&')
            .map(|e| start + e)
            .unwrap_or(out.len());
        out.replace_range(start..end, "***");
        i = start + 3;
    }
    out
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
    fn redact_hides_password() {
        let u = "https://x/getBuild?branch=main&package_id=p&password=secret123&tag=7.0";
        let r = redact(u);
        assert!(!r.contains("secret123"));
        assert!(r.contains("password=***"));
        assert!(r.contains("tag=7.0"));
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
}
