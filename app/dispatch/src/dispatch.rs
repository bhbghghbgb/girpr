//! The query itself: injectable GETs, decrypt, decode and `DispatchInfo`.
//!
//! One call takes a biz and the version from `metadata.rs` (`meta.latest`, not
//! a pre-repair local version), plus the two transport seams. It returns the
//! typed value the later phases consume. There is no HTTP client here, no
//! manifest fetch and no file or marker write: those belong to the fetch and
//! repair phases.

use crate::crypto::decode_base64;
use crate::metadata::load_dispatch_keys_with_pin;
use crate::proto::{DescriptorRow, parse_base_revision, parse_descriptor, parse_dispatch_protobuf};
use crate::url::{build_dispatcher_url, query_version, validate_dispatcher_url};
use crate::{DispatchError, Result};
use girpr_model::Region;
use serde_json::Value;

/// Plain GET of a metadata or dispatcher URL. Implemented by the caller; tests
/// supply recorded/injected bytes and never open a socket.
pub trait BytesFetcher {
    fn get(&self, url: &str) -> Result<Vec<u8>>;
}

impl<F> BytesFetcher for F
where
    F: Fn(&str) -> Result<Vec<u8>>,
{
    fn get(&self, url: &str) -> Result<Vec<u8>> {
        self(url)
    }
}

/// The second GET of `{client_game_res_url}/StandaloneWindows64/base_revision`.
pub trait BaseRevisionFetcher {
    fn get(&self, url: &str) -> Result<String>;
}

impl<F> BaseRevisionFetcher for F
where
    F: Fn(&str) -> Result<String>,
{
    fn get(&self, url: &str) -> Result<String> {
        self(url)
    }
}

/// Which manifest a [`ManifestDescriptor`] mirrors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestKind {
    Res,
    Data,
    Silence,
}

impl ManifestKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Res => "res",
            Self::Data => "data",
            Self::Silence => "silence_data",
        }
    }
}

/// One mirrored manifest: its fetch URL and the MD5/size gate from the
/// descriptor NDJSON, exactly as published in docs/03 §7.4.1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestDescriptor {
    pub kind: ManifestKind,
    pub url: String,
    pub expected_md5: String,
    pub expected_size: u64,
    pub mirror_rel: String,
}

/// The public result of a successful dispatcher query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchInfo {
    pub game_version: String,
    pub channel_name: String,
    pub server_name: String,
    pub codename: String,
    pub res_revision: u64,
    pub data_revision: u64,
    pub silence_revision: u64,
    pub audio_revision: u64,
    pub res_hash: String,
    pub data_hash: String,
    pub silence_hash: String,
    pub audio_hash: String,
    pub client_game_res_url: String,
    pub client_design_data_url: String,
    pub client_silence_data_url: String,
    pub client_audio_assets_url: String,
    pub manifest_digests: [ManifestDescriptor; 3],
}

/// The two inputs that are not values from the metadata leaf.
pub struct DispatchQuery<'a> {
    pub biz: Region,
    /// `meta.latest`, e.g. `7.2.0`. Never the pre-repair local version (B9(a)).
    pub latest: &'a str,
}

/// Split off the numeric `<major>.<minor>` part of a version string.
///
/// The server spells the same version `7.2_live`, so comparing the whole string
/// to a query's `7.2.0` would be a false disagreement.
pub fn minor_version(version: &str) -> Option<(u64, u64)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor_field = parts.next()?;
    let leading: String = minor_field
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let minor = leading.parse::<u64>().ok()?;
    Some((major, minor))
}

/// B9(a): a real minor-version disagreement is a metadata failure.
pub fn check_minor_version(queried: &str, response: &str) -> Result<()> {
    let want = minor_version(queried).ok_or_else(|| {
        DispatchError::Metadata(format!("queried version {queried:?} has no minor number"))
    })?;
    let got = minor_version(response).ok_or_else(|| {
        DispatchError::Metadata(format!("response version {response:?} has no minor number"))
    })?;
    if want != got {
        return Err(DispatchError::Version {
            queried: queried.to_owned(),
            response: response.to_owned(),
        });
    }
    Ok(())
}

/// Codename and channel authority.
///
/// Bilibili is the caveat from `CHANNELS.md`: its dispatcher answers codename
/// `hk4e_cn`, so the codename alone must not reject it. `properties_2=22102`
/// (CN is 22101) or a `cnqd` host is the strict channel assertion.
pub fn check_response_channel(
    biz: Region,
    codename: &str,
    properties_2: u64,
    host: &str,
) -> Result<()> {
    let accepted = match biz {
        Region::Hk4eGlobal => codename == "hk4e_global",
        Region::Hk4eCn => codename == "hk4e_cn",
        Region::Hk4eBilibili => codename == "hk4e_cn" || codename == "hk4e_bilibili",
    };
    if !accepted {
        return Err(DispatchError::Channel(format!(
            "codename {codename:?} does not match {biz}"
        )));
    }
    let (expected_properties_2, host_marker): (u64, Option<&str>) = match biz {
        Region::Hk4eGlobal => (22101, None),
        Region::Hk4eCn => (22101, Some("cngf")),
        Region::Hk4eBilibili => (22102, Some("cnqd")),
    };
    let host_ok = host_marker.is_some_and(|marker| host.contains(marker));
    if properties_2 != expected_properties_2 && !host_ok {
        return Err(DispatchError::Channel(format!(
            "{biz}: properties_2={properties_2}, expected {expected_properties_2}, host {host:?}"
        )));
    }
    Ok(())
}

fn trimmed_url(base: &str, name: &str) -> Result<String> {
    if base.is_empty() {
        return Err(DispatchError::Metadata(format!(
            "missing {name} in dispatcher response"
        )));
    }
    Ok(base.trim_end_matches('/').to_owned())
}

fn descriptor(
    kind: ManifestKind,
    row: DescriptorRow,
    url: String,
    exe_stem: &str,
) -> ManifestDescriptor {
    ManifestDescriptor {
        kind,
        url,
        expected_md5: row.md5,
        expected_size: row.file_size,
        mirror_rel: format!(
            "{exe_stem}_Data/Persistent/{}_versions_persist",
            kind.as_str()
        ),
    }
}

fn envelope_content(body: &[u8]) -> Result<String> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|e| DispatchError::Proto(format!("bad dispatcher envelope JSON: {e}")))?;
    let mut keys = value
        .as_object()
        .ok_or_else(|| DispatchError::Proto("dispatcher envelope is not an object".into()))?
        .keys()
        .collect::<Vec<_>>();
    keys.sort_unstable();
    if keys != ["content", "sign"] {
        return Err(DispatchError::Proto(format!(
            "dispatcher envelope keys are {keys:?}, expected content and sign"
        )));
    }
    value
        .get("content")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| DispatchError::Proto("dispatcher envelope has no content string".into()))
}

/// Full dispatcher query: unwrap keys, build the URL, decrypt, decode, check
/// version/channel, derive the four CDN prefixes and the three descriptors.
pub fn query_dispatch<F, B>(
    params: DispatchQuery<'_>,
    fetcher: &F,
    base_revision: &B,
) -> Result<DispatchInfo>
where
    F: BytesFetcher,
    B: BaseRevisionFetcher,
{
    query_dispatch_with_master_pin(params, fetcher, base_revision, true)
}

/// The offline-test seam behind [`query_dispatch`]. `pin` is always true in
/// the product path; tests with a synthetic master key pass `false`.
pub fn query_dispatch_with_master_pin<F, B>(
    params: DispatchQuery<'_>,
    fetcher: &F,
    base_revision: &B,
    pin: bool,
) -> Result<DispatchInfo>
where
    F: BytesFetcher,
    B: BaseRevisionFetcher,
{
    let keys = load_dispatch_keys_with_pin(params.biz, params.latest, fetcher, pin)?;
    let version = query_version(params.biz, params.latest);
    let url = build_dispatcher_url(&keys.template, keys.host, &version, &keys.seed)?;
    validate_dispatcher_url(&url, params.biz)?;

    let body = fetcher
        .get(&url)
        .map_err(|e| DispatchError::Transport(format!("dispatcher query: {e}")))?;
    let ciphertext = decode_base64(&envelope_content(&body)?)?;
    let plaintext = crate::decrypt_pkcs1_blocks(&keys.key, &ciphertext, keys.block_bytes)?;
    let (response, _opaque) = parse_dispatch_protobuf(&plaintext)?;
    let props = response
        .properties
        .ok_or_else(|| DispatchError::Proto("dispatcher response has no root tag 3".into()))?;
    let res_ver = props.res_version_properties.as_ref().ok_or_else(|| {
        DispatchError::Proto("dispatcher response has no properties tag 22".into())
    })?;

    check_minor_version(params.latest, &props.game_version)?;
    if !res_ver.version.is_empty() {
        check_minor_version(params.latest, &res_ver.version)?;
    }
    check_response_channel(params.biz, &props.codename, props.properties_2, keys.host)?;

    let res_base = trimmed_url(&props.repo_res_version_url, "RepoResVersionURL")?;
    let data_base = trimmed_url(&props.repo_design_data_url, "RepoDesignDataURL")?;
    if res_ver.res_hash.is_empty() || props.data_hash.is_empty() || props.silence_hash.is_empty() {
        return Err(DispatchError::Proto("a revision hash is empty".into()));
    }
    let client_game_res_url = format!(
        "{res_base}/output_{}_{}/client",
        res_ver.res_revision, res_ver.res_hash
    );
    let client_design_data_url = format!(
        "{data_base}/output_{}_{}/client/General",
        props.data_revision, props.data_hash
    );
    let client_silence_data_url = format!(
        "{data_base}/output_{}_{}/client_silence/General",
        props.silence_revision, props.silence_hash
    );

    let base_revision_url = format!("{client_game_res_url}/StandaloneWindows64/base_revision");
    let revision_body = base_revision.get(&base_revision_url).map_err(|e| {
        DispatchError::Transport(format!("base_revision GET {base_revision_url}: {e}"))
    })?;
    let (audio_revision, audio_hash) = parse_base_revision(&revision_body)?;
    let client_audio_assets_url = format!("{res_base}/output_{audio_revision}_{audio_hash}/client");

    let res_row = parse_descriptor(&res_ver.res_descriptor)?;
    let data_row = parse_descriptor(&props.data_descriptor)?;
    let silence_row = parse_descriptor(&props.silence_descriptor)?;
    let res_prefix = format!("{client_game_res_url}/StandaloneWindows64");
    let res_url = format!("{res_prefix}/{}", res_row.remote_name);
    let data_url = format!(
        "{client_design_data_url}/AssetBundles/{}",
        data_row.remote_name
    );
    let silence_url = format!(
        "{client_silence_data_url}/AssetBundles/{}",
        silence_row.remote_name
    );
    let manifest_digests = [
        descriptor(ManifestKind::Res, res_row, res_url, &keys.exe_stem),
        descriptor(ManifestKind::Data, data_row, data_url, &keys.exe_stem),
        descriptor(
            ManifestKind::Silence,
            silence_row,
            silence_url,
            &keys.exe_stem,
        ),
    ];

    Ok(DispatchInfo {
        game_version: props.game_version,
        channel_name: keys.channel_prefix.to_owned(),
        server_name: props.server_name,
        codename: props.codename,
        res_revision: res_ver.res_revision,
        data_revision: props.data_revision,
        silence_revision: props.silence_revision,
        audio_revision,
        res_hash: res_ver.res_hash.clone(),
        data_hash: props.data_hash.clone(),
        silence_hash: props.silence_hash.clone(),
        audio_hash,
        client_game_res_url,
        client_design_data_url,
        client_silence_data_url,
        client_audio_assets_url,
        manifest_digests,
    })
}
