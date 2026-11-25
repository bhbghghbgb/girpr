//! The plan half's persistent decisions: query, verify, plan, guard.
//!
//! This module owns the product-side wiring around three acyclic leaves:
//! `girpr-dispatch` (the query), `girpr-fetch::persistent` (the required
//! manifest fetches and gates) and `girpr-persistent-plan` (the pure plan and
//! the C7 scope predicates). Nothing here writes a marker or a mirror, spends
//! a hotfix byte, or decides a new exit code: every failure is mapped onto the
//! existing metadata/usage codes at the raise site.
//!
//! The whole module is inert unless `--with-persistent` was passed. An
//! unflagged run never constructs a transport, never calls this provider and
//! never grows a `SUMMARY` key.

use std::collections::HashMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result as AnyhowResult, anyhow, bail};
use girpr_digest::{decode_hex, md5_hex, sha256_hex};
use girpr_dispatch::{
    BaseRevisionFetcher, BytesFetcher, DispatchDecodeParams, DispatchDecoded, DispatchError,
    DispatchInfo, DispatchQuery, channel_prefix, decode_dispatch_response, dispatch_host,
    dispatch_info_from_decoded, parse_base_revision, private_key_block_bytes,
    private_key_fingerprint, private_key_from_bytes, query_dispatch,
};
use girpr_fetch::persistent::{fetch_all_manifests, parse_manifest};
use girpr_model::Region;
use girpr_persistent_plan::{
    BaseTree, Folder, ManifestInput, MarkerSet, PersistentPlan, ScopeDecision, asset_rel,
    base_res_version_hash_marker, build_plan, render_markers, scope_decision,
};
use reqwest::Client;
use serde::Deserialize;
use tokio::runtime::Handle;

use super::spawn_blocking_traced;
use crate::config::{RunCtx, RunFailure};
use crate::retry::BackoffPolicy;
use crate::util;

/// A test-only replacement for the dispatcher query.
///
/// The product path always leaves this `None` and calls the real provider. A
/// test supplies a closure that returns either a canned [`DispatchInfo`] whose
/// descriptor URLs name its loopback origin, or an error string modelling a
/// decoder/transport failure. It is a value on `RunCtx` rather than global
/// state, so a run's behaviour is fully determined by its own configuration.
#[derive(Clone)]
pub struct DispatchOverride(
    Arc<dyn Fn(Region, &str) -> std::result::Result<DispatchInfo, String> + Send + Sync>,
);

impl DispatchOverride {
    pub fn new(
        f: impl Fn(Region, &str) -> std::result::Result<DispatchInfo, String> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(f))
    }

    fn query(&self, biz: Region, latest: &str) -> std::result::Result<DispatchInfo, String> {
        (self.0)(biz, latest)
    }
}

impl fmt::Debug for DispatchOverride {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DispatchOverride(…)")
    }
}

/// Every dispatcher-side failure is a metadata failure with the pinned log
/// prefix, whatever layer produced it (key unwrap, transport, decrypt, decode,
/// version/codename disagreement, or the injected test seam).
pub fn dispatch_failure(e: impl fmt::Display) -> RunFailure {
    RunFailure::metadata(anyhow!("dispatcher query failed: {e}"))
}

// ---------------------------------------------------------------------------
// Out-of-band dispatcher inputs: --dispatch-key + --dispatch-record.
// ---------------------------------------------------------------------------

/// The record schema version this binary accepts. The capture tool owns the
/// writer; a different value is a usage error at the flag, never a fallback.
pub const DISPATCH_RECORD_SCHEMA_VERSION: u64 = 3;
const DISPATCH_KEY_BLOCK_BYTES: usize = 256;

/// The bundle record whose digest facts are checked against the envelope.
///
/// `dispatch_url` and `key_file` are accepted for the writer's convenience but
/// are never used to fetch or to load a key: the key always comes from
/// `--dispatch-key`, and no Collapse metadata is fetched on this path.
#[derive(Debug, Clone, Deserialize)]
pub struct DispatchRecord {
    pub schema_version: u64,
    pub version: String,
    pub query_version: String,
    #[serde(default)]
    pub dispatch_url: Option<String>,
    pub key_fingerprint: String,
    #[serde(default)]
    pub key_file: Option<String>,
    pub seed: String,
    pub envelope: String,
    pub envelope_md5: String,
    pub envelope_sha256: String,
    /// Byte length of the byte-exact envelope body, as recorded by the writer.
    pub ciphertext_bytes: u64,
    pub decoded: DecodedRecord,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DecodedRecord {
    pub game_version: String,
    #[serde(default)]
    pub channel_name: Option<String>,
    pub codename: String,
    pub res_revision: u64,
    pub data_revision: u64,
    pub silence_revision: u64,
    pub res_hash: String,
    pub data_hash: String,
    pub silence_hash: String,
    pub base_revision: String,
    /// CDN bases may be rewritten; only their paths are verified.
    #[serde(default)]
    pub repo_res_version_url: Option<String>,
    #[serde(default)]
    pub repo_design_data_url: Option<String>,
    pub descriptors: Vec<RecordDescriptor>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecordDescriptor {
    pub kind: String,
    pub remote_name: String,
    pub url: String,
    pub md5: String,
    pub file_size: u64,
}

fn is_hex(value: &str, bytes: usize) -> bool {
    value.len() == bytes * 2 && decode_hex(value).is_ok()
}

/// Reject `..`, absolute paths, backslashes, drives and empty components.
///
/// Both the CLI and the loader call this before an envelope path is joined to
/// anything, so a record can never name a file outside its own bundle tree.
fn bundle_relative_problem(path: &str) -> Option<&'static str> {
    if path.is_empty() {
        return Some("empty");
    }
    if path.contains('\\') {
        return Some("backslash");
    }
    if path.contains(':') {
        return Some("drive or colon");
    }
    if path.starts_with('/') {
        return Some("absolute");
    }
    if !Path::new(path)
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return Some("parent or current-directory component");
    }
    None
}

impl DispatchRecord {
    /// Everything checkable without reading the key or the envelope. The CLI
    /// calls this at `into_ctx` (exit 1); the loader calls it again (exit 2).
    pub fn validate(&self) -> AnyhowResult<()> {
        if self.schema_version != DISPATCH_RECORD_SCHEMA_VERSION {
            bail!(
                "dispatch record schema_version {} != {DISPATCH_RECORD_SCHEMA_VERSION}",
                self.schema_version
            );
        }
        if self.version.is_empty() || self.query_version.is_empty() {
            bail!("dispatch record has an empty version or query_version");
        }
        if let Some(url) = &self.dispatch_url {
            if url.is_empty() {
                bail!("dispatch record dispatch_url is empty");
            }
        }
        if let Some(path) = &self.key_file {
            if let Some(problem) = bundle_relative_problem(path) {
                bail!("dispatch record key_file path is unsafe ({problem}): {path:?}");
            }
        }
        if !is_hex(&self.key_fingerprint, 32) {
            bail!("dispatch record key_fingerprint is not 32 bytes of hex");
        }
        if !is_hex(&self.seed, 8) {
            bail!("dispatch record seed is not 8 bytes of hex");
        }
        if let Some(problem) = bundle_relative_problem(&self.envelope) {
            bail!(
                "dispatch record envelope path is unsafe ({problem}): {:?}",
                self.envelope
            );
        }
        if !is_hex(&self.envelope_md5, 16) || !is_hex(&self.envelope_sha256, 32) {
            bail!("dispatch record envelope_md5/envelope_sha256 is not hex of the right length");
        }
        if self.ciphertext_bytes == 0 {
            bail!("dispatch record ciphertext_bytes is zero");
        }
        self.decoded.validate()
    }
}

impl DecodedRecord {
    fn validate(&self) -> AnyhowResult<()> {
        if self.game_version.is_empty() || self.codename.is_empty() {
            bail!("dispatch record decoded game_version/codename is empty");
        }
        for (name, hash) in [
            ("res_hash", &self.res_hash),
            ("data_hash", &self.data_hash),
            ("silence_hash", &self.silence_hash),
        ] {
            if hash.is_empty() {
                bail!("dispatch record decoded {name} is empty");
            }
        }
        parse_base_revision(&self.base_revision)
            .map_err(|e| anyhow!("dispatch record decoded base_revision: {e}"))?;
        if let Some(url) = &self.repo_res_version_url {
            if url.is_empty() {
                bail!("dispatch record decoded repo_res_version_url is empty");
            }
        }
        if let Some(url) = &self.repo_design_data_url {
            if url.is_empty() {
                bail!("dispatch record decoded repo_design_data_url is empty");
            }
        }
        if self.descriptors.len() != 3 {
            bail!(
                "dispatch record decoded has {} descriptors, expected 3",
                self.descriptors.len()
            );
        }
        let mut kinds = Vec::new();
        for row in &self.descriptors {
            if row.remote_name.is_empty() || row.url.is_empty() {
                bail!("dispatch record descriptor has an empty remote_name or url");
            }
            if !is_hex(&row.md5, 16) {
                bail!(
                    "dispatch record descriptor {} md5 is not 32 hex chars",
                    row.kind
                );
            }
            if row.file_size == 0 {
                bail!("dispatch record descriptor {} file_size is zero", row.kind);
            }
            if !["res", "data", "silence_data"].contains(&row.kind.as_str()) {
                bail!("dispatch record has unknown descriptor kind {:?}", row.kind);
            }
            kinds.push(row.kind.as_str());
        }
        kinds.sort_unstable();
        if kinds != ["data", "res", "silence_data"] {
            bail!("dispatch record descriptors must name res, data and silence_data once each");
        }
        Ok(())
    }
}

/// Parse and size-check the two flag bytes without ever logging their content.
pub fn validate_dispatch_key_file(bytes: &[u8]) -> AnyhowResult<()> {
    let key = private_key_from_bytes(bytes).context("dispatch key")?;
    let block = private_key_block_bytes(&key);
    if block != DISPATCH_KEY_BLOCK_BYTES {
        bail!(
            "dispatch key must be RSA-2048 ({DISPATCH_KEY_BLOCK_BYTES}-byte blocks), got {block}"
        );
    }
    private_key_fingerprint(&key).context("dispatch key fingerprint")?;
    Ok(())
}

/// JSON/schema/path checks for `--dispatch-record` at the flag boundary.
pub fn validate_dispatch_record_file(bytes: &[u8]) -> AnyhowResult<()> {
    let record: DispatchRecord =
        serde_json::from_slice(bytes).context("dispatch record is not JSON")?;
    record.validate()
}

/// Mirror names come from `GameExecutableName` on the live path. A record does
/// not carry one, so the biz decides it — never an editable record field.
fn exe_stem_for(biz: Region) -> &'static str {
    match biz {
        Region::Hk4eGlobal => "GenshinImpact",
        Region::Hk4eCn | Region::Hk4eBilibili => "YuanShen",
    }
}

/// A record's envelope is bundle-relative. Accept both a scratch copy that
/// keeps the `dispatch/` directory and one whose record sits at the bundle
/// root; the path itself was already checked for traversal.
fn resolve_envelope(record_path: &Path, envelope: &str) -> AnyhowResult<PathBuf> {
    let parent = record_path.parent().unwrap_or_else(|| Path::new("."));
    let beside = parent.join(envelope);
    if beside.is_file() {
        return Ok(beside);
    }
    if let Some(root) = parent.parent() {
        let at_root = root.join(envelope);
        if at_root.is_file() {
            return Ok(at_root);
        }
    }
    bail!("dispatch envelope {envelope:?} is missing beside the record")
}

/// Scheme-and-authority-free URL tail. The bridge may rewrite the authority;
/// changing the *path* would name a different dispatcher fact.
fn url_path(url: &str) -> &str {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    after_scheme.find('/').map_or("/", |i| &after_scheme[i..])
}

fn check_text(bad: &mut Vec<String>, name: &str, want: &str, got: &str) {
    if want != got {
        bad.push(format!("{name}: record {want:?} != envelope {got:?}"));
    }
}

fn check_number(bad: &mut Vec<String>, name: &str, want: u64, got: u64) {
    if want != got {
        bad.push(format!("{name}: record {want} != envelope {got}"));
    }
}

/// Prove that every digest fact the record claims came from the decrypted
/// envelope. URLs are the one exception: their authorities may be rewritten.
fn verify_record_facts(record: &DispatchRecord, decoded: &DispatchDecoded) -> AnyhowResult<()> {
    let d = &record.decoded;
    let mut bad = Vec::new();
    check_text(
        &mut bad,
        "game_version",
        &d.game_version,
        &decoded.game_version,
    );
    check_text(&mut bad, "codename", &d.codename, &decoded.codename);
    if let Some(channel) = &d.channel_name {
        check_text(&mut bad, "channel_name", channel, &decoded.channel_name);
    }
    check_number(
        &mut bad,
        "res_revision",
        d.res_revision,
        decoded.res_revision,
    );
    check_number(
        &mut bad,
        "data_revision",
        d.data_revision,
        decoded.data_revision,
    );
    check_number(
        &mut bad,
        "silence_revision",
        d.silence_revision,
        decoded.silence_revision,
    );
    check_text(&mut bad, "res_hash", &d.res_hash, &decoded.res_hash);
    check_text(&mut bad, "data_hash", &d.data_hash, &decoded.data_hash);
    check_text(
        &mut bad,
        "silence_hash",
        &d.silence_hash,
        &decoded.silence_hash,
    );

    for row in &d.descriptors {
        let index = decoded
            .manifest_digests
            .iter()
            .position(|m| m.kind.as_str() == row.kind)
            .ok_or_else(|| anyhow!("record descriptor kind {:?} is unknown", row.kind))?;
        let actual = &decoded.manifest_digests[index];
        check_text(&mut bad, "descriptor md5", &row.md5, &actual.expected_md5);
        check_number(
            &mut bad,
            "descriptor file_size",
            row.file_size,
            actual.expected_size,
        );
        check_text(
            &mut bad,
            "descriptor remote_name",
            &row.remote_name,
            &decoded.descriptor_remote_names[index],
        );
        if url_path(&row.url) != url_path(&actual.url) {
            bad.push(format!(
                "descriptor {} URL path {:?} != envelope {:?}",
                row.kind, row.url, actual.url
            ));
        }
    }
    for (name, want, got) in [
        (
            "repo_res_version_url",
            d.repo_res_version_url.as_deref(),
            Some(decoded.repo_res_version_url.as_str()),
        ),
        (
            "repo_design_data_url",
            d.repo_design_data_url.as_deref(),
            Some(decoded.repo_design_data_url.as_str()),
        ),
    ] {
        if let Some(want) = want {
            let got = got.expect("decoded CDN base");
            if url_path(want) != url_path(got) {
                bad.push(format!("{name} path {want:?} != envelope {got:?}"));
            }
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        bail!(
            "record facts disagree with the decrypted envelope: {}",
            bad.join("; ")
        )
    }
}

/// Apply the record's (possibly rewritten) CDN URLs *after* verification.
fn apply_record_urls(record: &DispatchRecord, decoded: &mut DispatchDecoded) {
    if let Some(url) = &record.decoded.repo_res_version_url {
        let base = url.trim_end_matches('/').to_owned();
        decoded.repo_res_version_url = base.clone();
        decoded.client_game_res_url = format!(
            "{base}/output_{}_{}/client",
            decoded.res_revision, decoded.res_hash
        );
    }
    if let Some(url) = &record.decoded.repo_design_data_url {
        let base = url.trim_end_matches('/').to_owned();
        decoded.repo_design_data_url = base.clone();
        decoded.client_design_data_url = format!(
            "{base}/output_{}_{}/client/General",
            decoded.data_revision, decoded.data_hash
        );
        decoded.client_silence_data_url = format!(
            "{base}/output_{}_{}/client_silence/General",
            decoded.silence_revision, decoded.silence_hash
        );
    }
    for row in &record.decoded.descriptors {
        if let Some(index) = decoded
            .manifest_digests
            .iter()
            .position(|m| m.kind.as_str() == row.kind)
        {
            decoded.manifest_digests[index].url = row.url.clone();
        }
    }
}

/// Rebuild a `DispatchInfo` entirely from the supplied key and record. No
/// Collapse metadata, master key, seed table or dispatcher GET participates.
fn dispatch_from_files(
    key_path: &Path,
    record_path: &Path,
    biz: Region,
) -> AnyhowResult<DispatchInfo> {
    let key_bytes = std::fs::read(key_path)
        .with_context(|| format!("read dispatch key {}", key_path.display()))?;
    let key = private_key_from_bytes(&key_bytes).context("dispatch key")?;
    let block_bytes = private_key_block_bytes(&key);
    if block_bytes != DISPATCH_KEY_BLOCK_BYTES {
        bail!("dispatch key is {block_bytes} bytes, expected RSA-2048");
    }
    // Log the path and the fingerprint only — never a byte of the key itself.
    let fingerprint = private_key_fingerprint(&key).context("dispatch key fingerprint")?;
    tracing::info!(
        path = %key_path.display(),
        fingerprint = %fingerprint,
        "persistent dispatch key loaded"
    );

    let record_bytes = std::fs::read(record_path)
        .with_context(|| format!("read dispatch record {}", record_path.display()))?;
    let record: DispatchRecord =
        serde_json::from_slice(&record_bytes).context("dispatch record is not JSON")?;
    record.validate()?;
    if !record.key_fingerprint.eq_ignore_ascii_case(&fingerprint) {
        bail!(
            "record key_fingerprint {} != key fingerprint {fingerprint}",
            record.key_fingerprint
        );
    }
    tracing::info!(
        path = %record_path.display(),
        version = %record.version,
        "persistent dispatch record loaded"
    );

    let envelope_path = resolve_envelope(record_path, &record.envelope)?;
    let envelope = std::fs::read(&envelope_path)
        .with_context(|| format!("read dispatch envelope {}", envelope_path.display()))?;
    if envelope.len() as u64 != record.ciphertext_bytes {
        bail!(
            "dispatch envelope is {} bytes, record says {}",
            envelope.len(),
            record.ciphertext_bytes
        );
    }
    let md5 = md5_hex(&envelope);
    if !md5.eq_ignore_ascii_case(&record.envelope_md5) {
        bail!(
            "dispatch envelope md5 {md5} != record {}",
            record.envelope_md5
        );
    }
    let sha256 = sha256_hex(&envelope);
    if !sha256.eq_ignore_ascii_case(&record.envelope_sha256) {
        bail!(
            "dispatch envelope sha256 {sha256} != record {}",
            record.envelope_sha256
        );
    }

    let mut decoded = decode_dispatch_response(
        &envelope,
        DispatchDecodeParams {
            biz,
            latest: &record.version,
            host: dispatch_host(biz),
            channel_prefix: channel_prefix(biz),
            exe_stem: exe_stem_for(biz),
            key: &key,
            block_bytes,
        },
    )
    .context("decode dispatch envelope")?;
    verify_record_facts(&record, &decoded)?;
    apply_record_urls(&record, &mut decoded);
    dispatch_info_from_decoded(decoded, &record.decoded.base_revision)
        .context("dispatch base_revision")
}

// ---------------------------------------------------------------------------
// The provider's transport: injected into the synchronous decoder leaf.
// ---------------------------------------------------------------------------

/// A plain GET, used for both the metadata/dispatcher URLs and the second
/// `base_revision` GET. The decoder leaf is synchronous by design; the product
/// drives these futures on the runtime handle captured before the blocking
/// task is spawned, so no nested runtime is ever built.
struct HttpTransport {
    client: Client,
    handle: Handle,
}

impl HttpTransport {
    fn bytes(&self, url: &str) -> std::result::Result<Vec<u8>, DispatchError> {
        let client = self.client.clone();
        let url = url.to_owned();
        self.handle.block_on(async move {
            let response = client
                .get(&url)
                .send()
                .await
                .map_err(|e| DispatchError::Transport(format!("GET {url}: {e}")))?;
            let status = response.status();
            if !status.is_success() {
                return Err(DispatchError::Transport(format!(
                    "GET {url}: status {status}"
                )));
            }
            response
                .bytes()
                .await
                .map(|b| b.to_vec())
                .map_err(|e| DispatchError::Transport(format!("read body {url}: {e}")))
        })
    }
}

impl BytesFetcher for HttpTransport {
    fn get(&self, url: &str) -> std::result::Result<Vec<u8>, DispatchError> {
        self.bytes(url)
    }
}

impl BaseRevisionFetcher for HttpTransport {
    fn get(&self, url: &str) -> std::result::Result<String, DispatchError> {
        let bytes = self.bytes(url)?;
        String::from_utf8(bytes)
            .map_err(|e| DispatchError::Proto(format!("base_revision is not UTF-8: {e}")))
    }
}

/// Query `query_cur_region` with `meta.latest` (never the local version).
///
/// The query itself is synchronous, so it runs on the blocking pool with the
/// caller's tracing dispatcher installed. The override is checked first (the
/// unit-test seam); the key+record pair is an override too — it fetches no
/// Collapse metadata at all. An unflagged run never reaches this function.
pub async fn query(
    ctx: &RunCtx,
    latest: &str,
    http: &Client,
) -> std::result::Result<DispatchInfo, RunFailure> {
    let handle = Handle::current();
    let biz = ctx.biz;
    let latest = latest.to_owned();
    if let Some(override_) = ctx.persistent_dispatch_override.clone() {
        let query =
            spawn_blocking_traced(move || override_.query(biz, &latest).map_err(dispatch_failure))
                .await;
        return query.map_err(|e| dispatch_failure(anyhow!("query task failed: {e}")))?;
    }
    if let (Some(key_path), Some(record_path)) = (
        ctx.dispatch_key_path.clone(),
        ctx.dispatch_record_path.clone(),
    ) {
        let query = spawn_blocking_traced(move || {
            dispatch_from_files(&key_path, &record_path, biz).map_err(dispatch_failure)
        })
        .await;
        return query.map_err(|e| dispatch_failure(anyhow!("query task failed: {e}")))?;
    }
    let client = http.clone();
    let query = spawn_blocking_traced(move || {
        let transport = HttpTransport { client, handle };
        query_dispatch(
            DispatchQuery {
                biz,
                latest: &latest,
            },
            &transport,
            &transport,
        )
        .map_err(dispatch_failure)
    })
    .await;
    query.map_err(|e| dispatch_failure(anyhow!("query task failed: {e}")))?
}

// ---------------------------------------------------------------------------
// Required manifests: verify against the descriptors before parsing.
// ---------------------------------------------------------------------------

/// Fetch all three dispatcher descriptors and parse their NDJSON bodies.
///
/// A failure here is fatal (`RunFailure::metadata`), never the best-effort
/// degradation the base manifest fan-out uses. The raw bytes stay in memory:
/// mirroring them belongs to the execute/post half (commit 8).
pub async fn fetch_manifests(
    http: &Client,
    backoff: &BackoffPolicy,
    dispatch: &DispatchInfo,
) -> std::result::Result<Vec<ManifestInput>, RunFailure> {
    let fetched = fetch_all_manifests(http, backoff, &dispatch.manifest_digests)
        .await
        .map_err(|e| RunFailure::metadata(e.context("persistent manifest")))?;
    fetched
        .into_iter()
        .map(|f| {
            let rows = parse_manifest(&f.descriptor, &f.raw)
                .map_err(|e| RunFailure::metadata(e.context("persistent manifest")))?;
            Ok(ManifestInput {
                descriptor: f.descriptor,
                raw: f.raw,
                rows,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The pure plan, wrapped with the two product-side inputs and scope guards.
// ---------------------------------------------------------------------------

/// The scope outcome the post phase needs: a suppressed run still repairs
/// in-scope files but publishes no marker block and no mirror (C8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suppression {
    Proceed,
    Suppressed,
}

/// Everything the execute and post halves read about the persistent tier.
/// The dispatcher values and rendered marker bytes are carried here so
/// commits 7/8 can publish from the same decisions the plan made; the plan
/// half itself only lists them.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct PersistentState {
    pub dispatch: DispatchInfo,
    pub plan: PersistentPlan,
    /// The exact marker bytes, rendered once here so the C7(d) guard and the
    /// later publish cannot disagree about which paths are install-wide.
    pub markers: Vec<MarkerSet>,
    pub suppression: Suppression,
    /// The `SUMMARY persistent=` word: read-only modes only list, purge-only
    /// does no work, and a suppressed scope withholds the revision claim.
    pub status: &'static str,
    /// C16: `true` when `res_versions_streaming` was absent, so no
    /// `base_res_version_hash` marker was rendered.
    pub base_hash_omitted: bool,
}

/// The unfiltered Sophon `rel -> md5` map: the force rule must compare against
/// the whole base manifest, even under a scope that hides that rel.
pub fn base_md5_map(
    per_manifest: &[(String, Vec<crate::sophon::SophonChunkFile>)],
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for (_, files) in per_manifest {
        for f in files {
            if f.is_folder || f.file.is_empty() {
                continue;
            }
            if let Ok(rel) = util::clean_rel(&f.file) {
                map.insert(rel, f.md5.clone());
            }
        }
    }
    map
}

/// Read the base marker's source, if it exists. Only `NotFound` is the C16
/// omission; an unreadable file is a metadata failure rather than a silently
/// wrong hash.
pub fn read_base_hash_source(
    game_dir: &Path,
    exe_file_name: &str,
) -> std::result::Result<Option<Vec<u8>>, RunFailure> {
    let data_dir = girpr_persistent_plan::data_dir_name(exe_file_name)
        .map_err(|e| RunFailure::metadata(anyhow!("persistent plan: {e}")))?;
    let rel = format!("{data_dir}/StreamingAssets/res_versions_streaming");
    let path = util::contained_join(game_dir, &rel)
        .with_context(|| format!("resolve {rel}"))
        .map_err(RunFailure::metadata)?;
    match std::fs::read(&path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(RunFailure::metadata(anyhow!("read {rel}: {e}"))),
    }
}

/// Re-read the base hash source after the repair loop and rebuild the one
/// marker whose bytes come from it.
///
/// The plan half renders `base_res_version_hash` from the bytes that were on
/// disk *before* the base repair ran. That repair can create or correct
/// `StreamingAssets/res_versions_streaming`, so publishing or checking the
/// plan-time marker would claim a revision for bytes the game no longer has —
/// the exact case where the game wipes its Persistent state. Post and check
/// both call this one helper, so they cannot disagree about the marker block.
///
/// Returns the marker block to use and whether the source is genuinely absent
/// (C16). A marker rendered here replaces the stale one at the same, pinned
/// position; an absent source drops it entirely, and a source first created by
/// repair adds it back.
pub fn refresh_markers_after_repair(
    game_dir: &Path,
    exe_file_name: &str,
    state: &PersistentState,
) -> std::result::Result<(Vec<MarkerSet>, bool), RunFailure> {
    let source = read_base_hash_source(game_dir, exe_file_name)?;
    let base_path = asset_rel(
        exe_file_name,
        BaseTree::Persistent,
        Folder::Root,
        "base_res_version_hash",
    )
    .map_err(|e| RunFailure::metadata(anyhow!("persistent plan: {e}")))?;
    let mut markers: Vec<MarkerSet> = state
        .markers
        .iter()
        .filter(|m| m.path != base_path)
        .cloned()
        .collect();
    if let Some(bytes) = source.as_deref() {
        let marker = base_res_version_hash_marker(exe_file_name, bytes)
            .map_err(|e| RunFailure::metadata(anyhow!("persistent plan: {e}")))?;
        // `render_markers` puts this one first; keep the same block order so a
        // refreshed publish writes exactly the shape a clean plan would.
        markers.insert(0, marker);
        Ok((markers, false))
    } else {
        Ok((markers, true))
    }
}

/// C14: the audio list the base tier read (`getGameConfigs.audio_pkg_scan_dir`)
/// must name the same file the persistent tier would read. An empty config
/// value has no path to compare and is not a disagreement.
pub fn audio_scan_is_persistent(exe_file_name: &str, scan_dir: &str) -> bool {
    if scan_dir.is_empty() {
        return true;
    }
    match girpr_persistent_plan::data_dir_name(exe_file_name) {
        Ok(data_dir) => {
            scan_dir.eq_ignore_ascii_case(&format!("{data_dir}/Persistent/audio_lang_14"))
        }
        Err(_) => false,
    }
}

/// Build the plan and apply the two C7 scope guards.
///
/// The byte for `base_res_version_hash` is read once here and reused for the
/// marker render, so the guard sees exactly the marker the post phase would
/// publish. `meta.audio` is reused as-is: this tier never writes a second
/// `audio_lang_14` (C14).
#[allow(clippy::too_many_arguments)]
pub fn build(
    ctx: &RunCtx,
    game_dir: &Path,
    exe_file_name: &str,
    audio_scan_dir: &str,
    dispatch: &DispatchInfo,
    fetched: Vec<ManifestInput>,
    base_md5: &HashMap<String, String>,
    base_hash_source: Option<&[u8]>,
) -> std::result::Result<PersistentState, RunFailure> {
    // C14: verify the two paths name one file before assuming parity. The
    // selection itself is `meta.audio`, read-only; nothing is written here.
    if !audio_scan_is_persistent(exe_file_name, audio_scan_dir) {
        tracing::warn!(
            scan_dir = %audio_scan_dir,
            exe = %exe_file_name,
            "audio scan path differs from the persistent audio_lang_14 path; reusing the base selection"
        );
    }

    let mut plan = build_plan(
        exe_file_name,
        dispatch,
        base_md5,
        &fetched,
        base_hash_source,
    )
    .map_err(|e| RunFailure::metadata(anyhow!("persistent plan: {e}")))?;
    let markers = render_markers(exe_file_name, dispatch, base_hash_source)
        .map_err(|e| RunFailure::metadata(anyhow!("persistent plan: {e}")))?;

    // The C7 guards inspect the **unfiltered** dispatcher set: a hotfix the
    // scope hides still makes a revision claim false even though the work list
    // will not contain it. Only after the verdict do we drop hidden assets
    // from the work list ("filtered-out paths do not exist").
    let scope = &ctx.path_scope;
    let suppression = if scope.is_active() {
        match scope_decision(&plan, &markers, |rel| scope.is_visible(rel)) {
            ScopeDecision::Proceed => Suppression::Proceed,
            ScopeDecision::MarkersOutOfScope(path) => {
                return Err(RunFailure::usage(anyhow!(
                    "--with-persistent writes the marker file at '{path}', which is outside the \
                     active path scope (--include '{}' / --exclude '{}'); include it or drop one \
                     of the two flags",
                    scope.describe_includes(),
                    scope.describe_excludes(),
                )));
            }
            ScopeDecision::MirrorOutOfScope(path) => {
                return Err(RunFailure::usage(anyhow!(
                    "--with-persistent writes the mirror file at '{path}', which is outside the \
                     active path scope (--include '{}' / --exclude '{}'); include it or drop one \
                     of the two flags",
                    scope.describe_includes(),
                    scope.describe_excludes(),
                )));
            }
            ScopeDecision::HotfixOutOfScope(path) => {
                tracing::warn!(
                    "persistent hotfix path '{path}' is outside the active path scope; \
                     marker block and mirrors are suppressed so no revision is claimed for a \
                     partial install"
                );
                Suppression::Suppressed
            }
        }
    } else {
        Suppression::Proceed
    };

    if scope.is_active() {
        plan.assets.retain(|a| scope.is_visible(&a.rel));
        plan.total_bytes = plan
            .assets
            .iter()
            .fold(0u64, |sum, a| sum.saturating_add(a.size));
    }

    // The cache of the pure plan is not mutated by the scope filter above;
    // `game_dir` is passed through so the post phase can resolve rels against
    // the same root it uses today.
    let _ = game_dir;
    let status = if ctx.purge_only {
        "skipped"
    } else if ctx.readonly() || ctx.print_plan {
        "listed"
    } else if suppression == Suppression::Suppressed {
        "suppressed"
    } else {
        "published"
    };
    Ok(PersistentState {
        dispatch: dispatch.clone(),
        plan,
        markers,
        suppression,
        status,
        base_hash_omitted: base_hash_source.is_none(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RunExit;

    #[test]
    fn dispatcher_failures_keep_the_metadata_code_and_the_pinned_prefix() {
        let failure = dispatch_failure("bad seed for 7.2.0");
        assert_eq!(failure.exit_code, RunExit::Metadata);
        assert!(
            format!("{failure:#}").starts_with("dispatcher query failed:"),
            "{failure:#}"
        );
        assert!(format!("{failure:#}").contains("bad seed"), "{failure:#}");
    }

    /// The synchronous decoder drives real GETs on the runtime handle from a
    /// blocking thread, so no nested runtime is built. The seam is exercised
    /// against a loopback origin with no production URL.
    #[tokio::test]
    async fn the_real_transport_drives_a_plain_get_from_a_blocking_thread() {
        use girpr_loopback_mock::{Reply, ScriptedServer};

        let server = ScriptedServer::start(vec![Reply::new(200, "99000004 aabbccdd\n")]);
        let handle = Handle::current();
        let client = Client::new();
        let url = format!("{}/StandaloneWindows64/base_revision", server.base());
        let body = spawn_blocking_traced(move || {
            let transport = HttpTransport { client, handle };
            <HttpTransport as BaseRevisionFetcher>::get(&transport, &url)
        })
        .await
        .expect("the blocking task joins");
        assert_eq!(body.expect("the GET succeeds"), "99000004 aabbccdd\n");
    }

    /// C14: one source of truth for the audio list. The path comes from the
    /// game config and is compared with the `<exe>_Data` name derived from the
    /// same config, never hardcoded.
    #[test]
    fn audio_scan_path_is_compared_with_the_persistent_list() {
        assert!(audio_scan_is_persistent(
            "GenshinImpact.exe",
            "GenshinImpact_Data/Persistent/audio_lang_14"
        ));
        assert!(audio_scan_is_persistent(
            "YuanShen.exe",
            "yuanshen_data/Persistent/audio_lang_14"
        ));
        assert!(audio_scan_is_persistent("GenshinImpact.exe", ""));
        assert!(!audio_scan_is_persistent(
            "GenshinImpact.exe",
            "Somewhere/else.txt"
        ));
        assert!(!audio_scan_is_persistent(
            "",
            "GenshinImpact_Data/Persistent/audio_lang_14"
        ));
    }

    fn test_state(source: Option<&[u8]>) -> PersistentState {
        use crate::test_support::{ManifestKind, persistent_dispatch};

        let dispatch = persistent_dispatch(
            "http://127.0.0.1:1",
            "GenshinImpact",
            [
                (ManifestKind::Res, "res.ndjson", b""),
                (ManifestKind::Data, "data.ndjson", b""),
                (ManifestKind::Silence, "silence.ndjson", b""),
            ],
        );
        let markers = render_markers("GenshinImpact.exe", &dispatch, source).unwrap();
        PersistentState {
            dispatch,
            plan: PersistentPlan {
                assets: vec![],
                mirrors: vec![],
                marker_paths: markers.iter().map(|m| m.path.clone()).collect(),
                expected_rels: vec![],
                total_bytes: 0,
            },
            markers,
            suppression: Suppression::Proceed,
            status: "published",
            base_hash_omitted: source.is_none(),
        }
    }

    fn write_source(dir: &std::path::Path, bytes: &[u8]) {
        std::fs::create_dir_all(dir.join("GenshinImpact_Data/StreamingAssets")).unwrap();
        std::fs::write(
            dir.join("GenshinImpact_Data/StreamingAssets/res_versions_streaming"),
            bytes,
        )
        .unwrap();
    }

    /// The marker post publishes is hashed from the bytes on disk *after* the
    /// repair, not from the ones the plan half read.
    #[test]
    fn a_repaired_source_replaces_the_stale_base_hash_marker() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_persistent", "fresh_hash");
        let stale = b"pre repair bytes";
        let fresh = b"post repair bytes";
        let state = test_state(Some(stale));
        write_source(&dir, fresh);

        let (markers, omitted) =
            refresh_markers_after_repair(&dir, "GenshinImpact.exe", &state).unwrap();

        assert!(!omitted, "a source on disk is never an omission");
        assert_eq!(markers.len(), state.markers.len());
        assert_eq!(markers[0].path, state.markers[0].path);
        assert_eq!(markers[0].value, girpr_digest::md5_hex(fresh));
        assert_ne!(
            markers[0].value,
            girpr_digest::md5_hex(stale),
            "the stale pre-repair hash is never published"
        );
        assert_eq!(
            &markers[1..],
            &state.markers[1..],
            "the rest of the block is unchanged"
        );
    }

    /// C16 on a fresh install: the plan omitted the marker because the source
    /// was absent, but base repair created it, so post adds the marker back.
    #[test]
    fn a_source_created_by_the_repair_adds_the_omitted_marker() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_persistent", "created_hash");
        let fresh = b"created by base repair";
        let state = test_state(None);
        write_source(&dir, fresh);

        let (markers, omitted) =
            refresh_markers_after_repair(&dir, "GenshinImpact.exe", &state).unwrap();

        assert!(!omitted, "the omit status must flip back");
        assert_eq!(markers.len(), state.markers.len() + 1);
        assert_eq!(markers[0].value, girpr_digest::md5_hex(fresh));
        assert_eq!(
            &markers[1..],
            &state.markers[..],
            "the other markers stay put"
        );
    }

    /// A genuinely absent source keeps C16's omission: no marker is published
    /// or checked, even if the plan had rendered one from stale bytes.
    #[test]
    fn a_genuinely_absent_source_keeps_the_omission() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_persistent", "absent_hash");
        let state = test_state(Some(b"stale bytes"));

        let (markers, omitted) =
            refresh_markers_after_repair(&dir, "GenshinImpact.exe", &state).unwrap();

        assert!(omitted);
        assert_eq!(markers.len(), state.markers.len() - 1);
        assert!(
            markers
                .iter()
                .all(|m| !m.path.contains("base_res_version_hash"))
        );
    }
}
