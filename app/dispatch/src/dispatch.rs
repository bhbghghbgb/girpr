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
use rsa::PrivateKey;
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

/// The non-envelope inputs to [`decode_dispatch_response`].
///
/// The live provider reads all of them from the Collapse preset; a persistent
/// record supplies its own key and context while the biz still comes from the
/// CLI. They are values, not fetchers, which is what keeps the decoder pure.
pub struct DispatchDecodeParams<'a> {
    pub biz: Region,
    /// `meta.latest` on the live path; `record.version` for a supplied record.
    pub latest: &'a str,
    /// Dispatcher host used by the strict channel assertion.
    pub host: &'a str,
    /// Prefix published in `DispatchInfo.channel_name`.
    pub channel_prefix: &'a str,
    /// Executable stem used in the three `Persistent/` mirror paths.
    pub exe_stem: &'a str,
    pub key: &'a PrivateKey,
    pub block_bytes: usize,
}

/// Everything the envelope alone establishes, before the second GET.
///
/// A caller may rewrite the CDN URLs (the bridge swaps their authorities), but
/// must compare its digest facts to these values *first*: a record alone can
/// never invent a revision, a hash, a version or a codename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchDecoded {
    pub game_version: String,
    pub server_name: String,
    pub codename: String,
    pub res_revision: u64,
    pub data_revision: u64,
    pub silence_revision: u64,
    pub res_hash: String,
    pub data_hash: String,
    pub silence_hash: String,
    /// Trailing-slash-trimmed CDN bases from the protobuf.
    pub repo_res_version_url: String,
    pub repo_design_data_url: String,
    pub client_game_res_url: String,
    pub client_design_data_url: String,
    pub client_silence_data_url: String,
    pub base_revision_url: String,
    pub channel_name: String,
    pub exe_stem: String,
    /// The `remoteName` of each descriptor, in `manifest_digests` order.
    pub descriptor_remote_names: [String; 3],
    pub manifest_digests: [ManifestDescriptor; 3],
}

/// Decrypt and decode one dispatcher envelope. Pure: no socket, no file, no
/// clock and no exit-code policy — the shared tail of the live query and the
/// record-driven offline path.
pub fn decode_dispatch_response(
    envelope: &[u8],
    params: DispatchDecodeParams<'_>,
) -> Result<DispatchDecoded> {
    let ciphertext = decode_base64(&envelope_content(envelope)?)?;
    let plaintext = crate::decrypt_pkcs1_blocks(params.key, &ciphertext, params.block_bytes)?;
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
    check_response_channel(params.biz, &props.codename, props.properties_2, params.host)?;

    let repo_res_version_url = trimmed_url(&props.repo_res_version_url, "RepoResVersionURL")?;
    let repo_design_data_url = trimmed_url(&props.repo_design_data_url, "RepoDesignDataURL")?;
    if res_ver.res_hash.is_empty() || props.data_hash.is_empty() || props.silence_hash.is_empty() {
        return Err(DispatchError::Proto("a revision hash is empty".into()));
    }
    let client_game_res_url = format!(
        "{repo_res_version_url}/output_{}_{}/client",
        res_ver.res_revision, res_ver.res_hash
    );
    let client_design_data_url = format!(
        "{repo_design_data_url}/output_{}_{}/client/General",
        props.data_revision, props.data_hash
    );
    let client_silence_data_url = format!(
        "{repo_design_data_url}/output_{}_{}/client_silence/General",
        props.silence_revision, props.silence_hash
    );
    let base_revision_url = format!("{client_game_res_url}/StandaloneWindows64/base_revision");

    let res_row = parse_descriptor(&res_ver.res_descriptor)?;
    let data_row = parse_descriptor(&props.data_descriptor)?;
    let silence_row = parse_descriptor(&props.silence_descriptor)?;
    let res_url = format!(
        "{client_game_res_url}/StandaloneWindows64/{}",
        res_row.remote_name
    );
    let data_url = format!(
        "{client_design_data_url}/AssetBundles/{}",
        data_row.remote_name
    );
    let silence_url = format!(
        "{client_silence_data_url}/AssetBundles/{}",
        silence_row.remote_name
    );
    let manifest_digests = [
        descriptor(ManifestKind::Res, res_row.clone(), res_url, params.exe_stem),
        descriptor(
            ManifestKind::Data,
            data_row.clone(),
            data_url,
            params.exe_stem,
        ),
        descriptor(
            ManifestKind::Silence,
            silence_row.clone(),
            silence_url,
            params.exe_stem,
        ),
    ];

    Ok(DispatchDecoded {
        game_version: props.game_version,
        server_name: props.server_name,
        codename: props.codename,
        res_revision: res_ver.res_revision,
        data_revision: props.data_revision,
        silence_revision: props.silence_revision,
        res_hash: res_ver.res_hash.clone(),
        data_hash: props.data_hash.clone(),
        silence_hash: props.silence_hash.clone(),
        repo_res_version_url,
        repo_design_data_url,
        client_game_res_url,
        client_design_data_url,
        client_silence_data_url,
        base_revision_url,
        channel_name: params.channel_prefix.to_owned(),
        exe_stem: params.exe_stem.to_owned(),
        descriptor_remote_names: [
            res_row.remote_name,
            data_row.remote_name,
            silence_row.remote_name,
        ],
        manifest_digests,
    })
}

/// Finish a decoded response with a `"<revision> <hash>"` body. On the live
/// path that is the second GET; a persistent record carries the same bytes in
/// `decoded.base_revision`. Pure.
pub fn dispatch_info_from_decoded(
    decoded: DispatchDecoded,
    base_revision: &str,
) -> Result<DispatchInfo> {
    let (audio_revision, audio_hash) = parse_base_revision(base_revision)?;
    let client_audio_assets_url = format!(
        "{}/output_{audio_revision}_{audio_hash}/client",
        decoded.repo_res_version_url
    );
    Ok(DispatchInfo {
        game_version: decoded.game_version,
        channel_name: decoded.channel_name,
        server_name: decoded.server_name,
        codename: decoded.codename,
        res_revision: decoded.res_revision,
        data_revision: decoded.data_revision,
        silence_revision: decoded.silence_revision,
        audio_revision,
        res_hash: decoded.res_hash,
        data_hash: decoded.data_hash,
        silence_hash: decoded.silence_hash,
        audio_hash,
        client_game_res_url: decoded.client_game_res_url,
        client_design_data_url: decoded.client_design_data_url,
        client_silence_data_url: decoded.client_silence_data_url,
        client_audio_assets_url,
        manifest_digests: decoded.manifest_digests,
    })
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
    let decoded = decode_dispatch_response(
        &body,
        DispatchDecodeParams {
            biz: params.biz,
            latest: params.latest,
            host: keys.host,
            channel_prefix: keys.channel_prefix,
            exe_stem: &keys.exe_stem,
            key: &keys.key,
            block_bytes: keys.block_bytes,
        },
    )?;
    let revision_body = base_revision.get(&decoded.base_revision_url).map_err(|e| {
        DispatchError::Transport(format!(
            "base_revision GET {}: {e}",
            decoded.base_revision_url
        ))
    })?;
    dispatch_info_from_decoded(decoded, &revision_body)
}
