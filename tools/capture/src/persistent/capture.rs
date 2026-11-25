//! Query the dispatcher from the capture pipeline and prepare its records.
//!
//! The dispatcher leaf is synchronous by design: it takes two plain GET
//! traits, not an async client. Here those traits are adapted to gircap's
//! retrying [`Fetcher`] on a blocking thread, with the tokio `Handle` captured
//! before the spawn — the same shape girpr's repair binary uses. Every GET
//! the adapter makes is logged by the fetcher as `Kind::Api`, gircap's
//! existing closed vocabulary for JSON requests.
//!
//! The query runs **after** `select::decide` and **before**
//! `select::require::check`. A capture without a dispatcher response stays a
//! normal capture: the three record rows are `unavailable`, and no `dispatch/`
//! or `persistent/` bytes are written. That is the schema-3 degradation path;
//! records are still optional at this version.

use std::sync::Arc;

use girpr_dispatch::{
    BaseRevisionFetcher, BytesFetcher, DispatchDecodeParams, DispatchError, DispatchInfo,
    ManifestKind, PrivateKey, channel_prefix, config_file_from_stamp, decode_dispatch_response,
    dispatch_host, dispatch_info_from_decoded, master_key_from_json, pinned_seed,
    private_key_fingerprint, query_version, rsa_key_from_xml, template_for, unwrap_metadata_field,
};
use girpr_model::Region;
use girpr_persistent_plan::{Folder, asset_urls, folder_for};
use serde_json::Value;
use tokio::runtime::Handle;

use crate::http::{Fetcher, Kind};
use crate::select::{Budget, Class, MatrixRow, MatrixStatus};

use super::{Decoded, Descriptor, DispatchRecord, ManifestFile, PersistentIndex};

/// Everything the dispatcher query returned that a bundle record needs.
///
/// `query_dispatch` publishes only the typed `DispatchInfo`. The producer
/// additionally needs the byte-exact envelope (stored verbatim), the key
/// material (stored beside the record for offline replay) and the seed/URL it
/// queried with, so this value keeps all of them together.
#[derive(Debug, Clone)]
pub struct DispatchContext {
    /// The typed result the manifest fetches and the plan inputs use.
    pub info: DispatchInfo,
    /// The dispatcher response body, byte-exact.
    pub envelope: Vec<u8>,
    /// Private key bytes from the preset. Test fixtures use a synthetic key;
    /// a live key belongs only in a gitignored artifact directory.
    pub key_material: Vec<u8>,
    /// The versioned seed unwrapped from the preset.
    pub seed: String,
    /// The full dispatcher URL that produced `envelope`.
    pub dispatch_url: String,
    /// `channel_prefix + latest`, as sent in the query string.
    pub query_version: String,
    /// The two CDN bases as decoded from the envelope, before the derived
    /// `output_<rev>_<hash>/client` URLs are built.
    pub repo_res_version_url: String,
    pub repo_design_data_url: String,
    /// The CDN `remoteName` of each descriptor, in `manifest_digests` order.
    pub descriptor_remote_names: [String; 3],
    /// The raw `"<revision> <hash>"` body of the second GET.
    pub base_revision: String,
}

/// Where the dispatcher query comes from.
///
/// Production always uses [`DispatchSource::Live`]. [`DispatchSource::Test`]
/// is a test-only injection seam: it lets a loopback test drive the whole
/// producer (query → manifests → cheapest-first blobs → records) without
/// opening a production metadata URL. It is a value on the input, never a CLI
/// flag, so a run's behaviour is decided by its caller alone.
pub enum DispatchSource {
    /// Query the real metadata/dispatcher/base_revision URLs. `pin` is the
    /// test-only master-pin seam: production passes `true`, a mock that
    /// serves a synthetic master key passes `false`.
    Live { pin: bool },
    /// No dispatcher routes were configured for this capture. The three rows
    /// are recorded as unavailable without a single GET: an ordinary mock
    /// capture must not acquire a request-log row or an error body it cannot
    /// explain.
    Skip(&'static str),
    /// A canned dispatcher context, for offline tests.
    Test(Arc<dyn Fn(Region, &str) -> Result<DispatchContext, String> + Send + Sync>),
}

/// Parse the test-only master-pin hook.
///
/// Only the exact strings `"0"` and `"1"` enable the seam; unset or any other
/// value is `None`, which means production's pinned master key. The hook is an
/// environment variable on purpose: a spawned capture cannot be given a Rust
/// value, and a CLI flag would let an operator accidentally disable the pin.
pub fn master_pin_from(value: Option<&str>) -> Option<bool> {
    match value {
        Some("0") => Some(false),
        Some("1") => Some(true),
        _ => None,
    }
}

/// The production pin, unless a test explicitly overrode it.
pub fn master_pin() -> bool {
    master_pin_from(std::env::var("GIRCAP_TEST_DISPATCH_PIN").ok().as_deref()).unwrap_or(true)
}

/// Adapt gircap's fetcher to the dispatcher's two synchronous GET traits.
struct FetcherTransport<'a> {
    fetcher: &'a Fetcher,
    handle: Handle,
}

impl FetcherTransport<'_> {
    fn bytes(&self, url: &str) -> Result<Vec<u8>, DispatchError> {
        let fetcher = self.fetcher.clone();
        let url = url.to_owned();
        self.handle.block_on(async move {
            fetcher
                .get(&url, Kind::Api, None)
                .await
                .map(|f| f.body)
                .map_err(|e| DispatchError::Transport(format!("GET {url}: {e}")))
        })
    }
}

impl BytesFetcher for FetcherTransport<'_> {
    fn get(&self, url: &str) -> Result<Vec<u8>, DispatchError> {
        self.bytes(url)
    }
}

impl BaseRevisionFetcher for FetcherTransport<'_> {
    fn get(&self, url: &str) -> Result<String, DispatchError> {
        let bytes = self.bytes(url)?;
        String::from_utf8(bytes)
            .map_err(|e| DispatchError::Proto(format!("base_revision is not UTF-8: {e}")))
    }
}

/// Mirror names come from the exe, never from an editable record field — the
/// same rule girpr's product path applies.
fn exe_stem_for(biz: Region) -> &'static str {
    match biz {
        Region::Hk4eGlobal => "GenshinImpact",
        Region::Hk4eCn | Region::Hk4eBilibili => "YuanShen",
    }
}

fn as_string(value: &Value, name: &'static str) -> Result<String, DispatchError> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| DispatchError::Metadata(format!("{name} is not a string")))
}

/// Unwrap one preset field, keeping the plain-scalar case for test fixtures.
fn field(
    config: &Value,
    name: &'static str,
    master: &PrivateKey,
    block_bytes: usize,
) -> Result<Value, DispatchError> {
    let raw = config
        .get(name)
        .ok_or_else(|| DispatchError::Metadata(format!("preset has no {name}")))?;
    unwrap_metadata_field(raw, master, block_bytes)
}

/// The dispatcher query, keeping the raw envelope and key material.
///
/// Mirrors `girpr_dispatch::query_dispatch_with_master_pin` step for step —
/// same metadata URLs, seed table, URL validation, decoder and second GET —
/// but retains the bytes a bundle record has to publish. `pin = false` is the
/// test-only seam for a synthetic master key; production always passes `true`.
pub fn query_with_context<B, R>(
    biz: Region,
    latest: &str,
    fetcher: &B,
    base_revision: &R,
    pin: bool,
) -> Result<DispatchContext, DispatchError>
where
    B: BytesFetcher,
    R: BaseRevisionFetcher,
{
    use girpr_dispatch::{METADATA_BASE, build_dispatcher_url, validate_dispatcher_url};

    // The two fixed metadata URLs are spelled from the published base so this
    // leaf cannot silently use a different release repo than girpr does.
    let master_url = format!("{METADATA_BASE}/config_master.json");
    let stamp_url = format!("{METADATA_BASE}/stamp.json");
    let master_json = fetcher
        .get(&master_url)
        .map_err(|e| DispatchError::Transport(format!("config_master.json: {e}")))?;
    let stamp_json = fetcher
        .get(&stamp_url)
        .map_err(|e| DispatchError::Transport(format!("stamp.json: {e}")))?;
    let (master, master_block) = master_key_from_json(&master_json, pin)?;
    let config_file = config_file_from_stamp(&stamp_json, biz)?;
    let config_url = format!("{METADATA_BASE}/{config_file}");
    let config_json = fetcher
        .get(&config_url)
        .map_err(|e| DispatchError::Transport(format!("{config_file}: {e}")))?;
    let config: Value = serde_json::from_slice(&config_json)
        .map_err(|e| DispatchError::Metadata(format!("bad {config_file}: {e}")))?;

    let template = match config.get("GameDispatchURL") {
        Some(raw) => as_string(
            &unwrap_metadata_field(raw, &master, master_block)?,
            "GameDispatchURL",
        )?,
        None => template_for(biz).to_owned(),
    };
    let seed = as_string(
        &field(&config, "ProtoDispatchKey", &master, master_block)?,
        "ProtoDispatchKey",
    )?;
    if pin {
        let expected = pinned_seed(biz, latest)?;
        if seed != expected {
            return Err(DispatchError::Metadata(format!(
                "metadata seed {seed} != pinned {expected} for {biz} {latest}"
            )));
        }
    }
    let block = field(&config, "DispatcherKeyBitLength", &master, master_block)?
        .as_u64()
        .ok_or_else(|| {
            DispatchError::Metadata("DispatcherKeyBitLength is not an integer".into())
        })?;
    let block_bytes = usize::try_from(block)
        .map_err(|_| DispatchError::Metadata("DispatcherKeyBitLength does not fit".into()))?;
    if block_bytes != 256 {
        return Err(DispatchError::Metadata(format!(
            "dispatcher block length {block_bytes} != 256"
        )));
    }
    let key_material = as_string(
        &field(&config, "DispatcherKey", &master, master_block)?,
        "DispatcherKey",
    )?
    .into_bytes();
    let key = rsa_key_from_xml(
        std::str::from_utf8(&key_material)
            .map_err(|e| DispatchError::Metadata(format!("DispatcherKey is not UTF-8: {e}")))?,
    )?;
    let fingerprint = private_key_fingerprint(&key)?;
    tracing::info!(
        biz = %biz,
        version = %latest,
        fingerprint = %fingerprint,
        "persistent dispatcher key unwrapped"
    );

    let query_version = query_version(biz, latest);
    let dispatch_url = build_dispatcher_url(&template, dispatch_host(biz), &query_version, &seed)?;
    validate_dispatcher_url(&dispatch_url, biz)?;
    let envelope = fetcher
        .get(&dispatch_url)
        .map_err(|e| DispatchError::Transport(format!("dispatcher query: {e}")))?;
    let decoded = decode_dispatch_response(
        &envelope,
        DispatchDecodeParams {
            biz,
            latest,
            host: dispatch_host(biz),
            channel_prefix: channel_prefix(biz),
            exe_stem: exe_stem_for(biz),
            key: &key,
            block_bytes,
        },
    )?;
    let descriptor_remote_names = decoded.descriptor_remote_names.clone();
    let repo_res_version_url = decoded.repo_res_version_url.clone();
    let repo_design_data_url = decoded.repo_design_data_url.clone();
    let revision_body = base_revision
        .get(&decoded.base_revision_url)
        .map_err(|e| DispatchError::Transport(format!("base_revision GET: {e}")))?;
    let info = dispatch_info_from_decoded(decoded, &revision_body)?;
    Ok(DispatchContext {
        info,
        envelope,
        key_material,
        seed,
        dispatch_url,
        query_version,
        repo_res_version_url,
        repo_design_data_url,
        descriptor_remote_names,
        base_revision: revision_body,
    })
}

/// The bundle records and raw bodies prepared by a successful capture.
#[derive(Debug, Clone)]
pub struct PreparedRecords {
    /// The single aggregate dispatch record: all three descriptors, one key.
    pub record: DispatchRecord,
    /// Byte-exact dispatcher response body.
    pub envelope: Vec<u8>,
    /// Private key bytes, written only to the bundle's gitignored artifact.
    pub key_material: Vec<u8>,
    /// The three raw manifests and every admitted full-file blob.
    pub index: PersistentIndex,
    /// `(bundle-relative path, raw bytes)` for `emit::write_persistent`.
    pub bodies: Vec<(String, Vec<u8>)>,
}

/// What the persistent producer decided.
pub struct Outcome {
    /// The matrix rows and admissions, merged by the caller into `Selection`.
    pub selection: crate::select::persistent::PersistentSelection,
    /// Only present when the dispatcher answered and every admitted blob
    /// passed its size/MD5 gate.
    pub records: Option<PreparedRecords>,
    /// Why no records were written, when that is the case.
    pub reason: Option<String>,
}

/// What one capture needs from its caller.
pub struct CaptureInput<'a> {
    /// The same fetcher the rest of the capture uses, so dispatcher GETs join
    /// `api/requests.ndjson` as `Kind::Api` rows.
    pub fetcher: &'a Fetcher,
    /// The region whose dispatcher is queried.
    pub biz: Region,
    /// `meta.latest`: the version the query and the record carry.
    pub latest: &'a str,
    /// The budget the content selection already spent.
    pub budget: Budget,
    /// Live or injected dispatcher.
    pub source: DispatchSource,
}

fn unavailable_all(reason: impl Into<String>, considered: usize) -> Vec<MatrixRow> {
    let reason = reason.into();
    Class::PERSISTENT
        .iter()
        .map(|class| {
            MatrixRow::new(*class, MatrixStatus::Unavailable, considered as u64)
                .with_reason(reason.clone())
        })
        .collect()
}

fn manifest_for(
    kind: ManifestKind,
    manifests: &[crate::select::persistent::VerifiedManifest; 3],
) -> Option<&crate::select::persistent::VerifiedManifest> {
    manifests.iter().find(|m| m.descriptor.kind == kind)
}

fn kind_for(class: Class) -> Option<ManifestKind> {
    match class {
        Class::PersistentRes => Some(ManifestKind::Res),
        Class::PersistentData => Some(ManifestKind::Data),
        Class::PersistentSilence => Some(ManifestKind::Silence),
        _ => None,
    }
}

/// Query, verify, select and stage the persistent records.
///
/// A missing dispatcher response is **not** a capture failure: the three rows
/// are unavailable with the reason and the bundle is written without records.
pub async fn capture(input: CaptureInput<'_>) -> Outcome {
    let CaptureInput {
        fetcher,
        biz,
        latest,
        budget,
        source,
    } = input;

    let context: Result<DispatchContext, String> = match source {
        DispatchSource::Skip(reason) => Err(reason.to_string()),
        DispatchSource::Test(query) => {
            let latest = latest.to_owned();
            std::thread::spawn(move || query(biz, &latest))
                .join()
                .map_err(|_| "the test dispatcher query panicked".to_string())
                .and_then(|r| r)
        }
        DispatchSource::Live { pin } => {
            let f = fetcher.clone();
            let latest = latest.to_owned();
            let handle = Handle::current();
            crate::tasks::join_blocking("dispatcher query", move || {
                let transport = FetcherTransport {
                    fetcher: &f,
                    handle,
                };
                query_with_context(biz, &latest, &transport, &transport, pin)
                    .map_err(|e| e.to_string())
            })
            .await
            .and_then(|r| r)
        }
    };
    let context = match context {
        Ok(c) => c,
        Err(reason) => {
            let why = format!("dispatcher response unavailable: {reason}");
            tracing::warn!(reason = %why, "persistent records not captured");
            return Outcome {
                selection: crate::select::persistent::PersistentSelection {
                    admitted: Vec::new(),
                    matrix: unavailable_all(why.clone(), 0),
                    bytes_admitted: budget.spent(),
                    budget,
                },
                records: None,
                reason: Some(why),
            };
        }
    };

    // The three descriptors are fetched and verified BEFORE parsing. girpr
    // always requests all three from the one aggregate record, so a partial
    // set is never a usable persistent bundle.
    let mut verified: Vec<crate::select::persistent::VerifiedManifest> = Vec::new();
    for descriptor in &context.info.manifest_digests {
        let url = descriptor.url.clone();
        let f = fetcher.clone();
        let fetched = match f.get(&url, Kind::Manifest, None).await {
            Ok(f) => f.body,
            Err(e) => {
                let why = format!("{descriptor:?} manifest unavailable: {e}");
                return unavailable_outcome(budget, why);
            }
        };
        match crate::select::persistent::VerifiedManifest::new(descriptor.clone(), fetched) {
            Ok(m) => verified.push(m),
            Err(e) => return unavailable_outcome(budget, e),
        }
    }
    let verified: [crate::select::persistent::VerifiedManifest; 3] = match verified.try_into() {
        Ok(v) => v,
        Err(_) => return unavailable_outcome(budget, "expected three verified manifests"),
    };

    let selection =
        crate::select::persistent::select(&verified, context.envelope.len() as u64, budget);

    // Fetch each admitted full-file blob through the same fetcher, gate it on
    // the row's size and MD5, and pair it with its bundle-relative path.
    let mut bodies: Vec<(String, Vec<u8>)> = Vec::new();
    for manifest in &verified {
        bodies.push((
            format!("persistent/{}.ndjson", manifest.descriptor.kind.as_str()),
            manifest.raw.clone(),
        ));
    }
    for admission in &selection.admitted {
        let Some(kind) = kind_for(admission.class) else {
            continue;
        };
        let Some(_descriptor) = manifest_for(kind, &verified) else {
            continue;
        };
        let folder: Folder = folder_for(&admission.remote_name);
        let (url, _) = match asset_urls(&context.info, kind, folder, &admission.remote_name, false)
        {
            Ok(urls) => urls,
            Err(e) => return unavailable_outcome(selection.budget, format!("blob URL: {e}")),
        };
        let f = fetcher.clone();
        let bytes = match f.get(&url, Kind::Api, None).await {
            Ok(f) => f.body,
            Err(e) => {
                let why = format!("blob {} unavailable: {e}", admission.remote_name);
                return unavailable_outcome(selection.budget, why);
            }
        };
        if bytes.len() as u64 != admission.budget_cost {
            let why = format!(
                "blob {} is {} B, row says {} B",
                admission.remote_name,
                bytes.len(),
                admission.budget_cost
            );
            return unavailable_outcome(selection.budget, why);
        }
        let md5 = crate::hash::md5_hex(&bytes);
        if !md5.eq_ignore_ascii_case(&admission.md5) {
            let why = format!(
                "blob {} md5 {md5} != row {}",
                admission.remote_name, admission.md5
            );
            return unavailable_outcome(selection.budget, why);
        }
        bodies.push((admission.path.clone(), bytes));
    }

    let descriptors: Vec<Descriptor> = context
        .info
        .manifest_digests
        .iter()
        .zip(context.descriptor_remote_names.iter())
        .map(|(d, remote_name)| Descriptor {
            kind: d.kind.as_str().to_string(),
            // The CDN `remoteName` from the descriptor NDJSON, not the
            // `Persistent/` mirror path the plan derives from it.
            remote_name: remote_name.clone(),
            url: d.url.clone(),
            md5: d.expected_md5.clone(),
            file_size: d.expected_size,
        })
        .collect();

    let index = PersistentIndex {
        schema_version: crate::SCHEMA_VERSION,
        manifests: verified
            .iter()
            .map(|m| ManifestFile {
                kind: m.descriptor.kind.as_str().to_string(),
                path: format!("persistent/{}.ndjson", m.descriptor.kind.as_str()),
                bytes: m.raw.len() as u64,
                md5: crate::hash::md5_hex(&m.raw),
            })
            .collect::<Vec<_>>()
            .try_into()
            .expect("three verified manifests"),
        blobs: selection
            .admitted
            .iter()
            .map(|a| crate::persistent::BlobFile {
                remote_name: a.remote_name.clone(),
                source_kind: kind_for(a.class)
                    .map(|k| k.as_str())
                    .unwrap_or_default()
                    .to_string(),
                capture_id: a.capture_id.clone(),
                path: a.path.clone(),
                bytes: a.budget_cost,
                md5: a.md5.clone(),
            })
            .collect(),
    };

    let mut record = DispatchRecord {
        schema_version: crate::SCHEMA_VERSION,
        version: latest.to_string(),
        query_version: context.query_version.clone(),
        dispatch_url: context.dispatch_url.clone(),
        key_fingerprint: String::new(),
        key_file: "dispatch/key.pem".to_string(),
        seed: context.seed.clone(),
        envelope: "dispatch/response.json".to_string(),
        envelope_md5: crate::hash::md5_hex(&context.envelope),
        envelope_sha256: crate::hash::sha256_hex(&context.envelope),
        ciphertext_bytes: context.envelope.len() as u64,
        decoded: Decoded {
            game_version: context.info.game_version.clone(),
            channel_name: context.info.channel_name.clone(),
            codename: context.info.codename.clone(),
            res_revision: context.info.res_revision,
            data_revision: context.info.data_revision,
            silence_revision: context.info.silence_revision,
            audio_revision: context.info.audio_revision,
            res_hash: context.info.res_hash.clone(),
            data_hash: context.info.data_hash.clone(),
            silence_hash: context.info.silence_hash.clone(),
            audio_hash: context.info.audio_hash.clone(),
            // The CDN *bases* as decoded, not the derived `output_<rev>_<hash>`
            // URLs. girpr compares their paths against the decoder's values.
            repo_res_version_url: context.repo_res_version_url.clone(),
            repo_design_data_url: context.repo_design_data_url.clone(),
            base_revision: context.base_revision.clone(),
            descriptors: descriptors.try_into().expect("three descriptors"),
        },
    };
    // The fingerprint is SHA-256 of the public DER, never of the private
    // bytes; the parser checks it against the key it is given out of band.
    record.key_fingerprint = fingerprint_from_context(&context);

    Outcome {
        selection,
        records: Some(PreparedRecords {
            record,
            envelope: context.envelope,
            key_material: context.key_material,
            index,
            bodies,
        }),
        reason: None,
    }
}

fn fingerprint_from_context(context: &DispatchContext) -> String {
    match girpr_dispatch::private_key_from_bytes(&context.key_material) {
        Ok(key) => private_key_fingerprint(&key).unwrap_or_default(),
        Err(_) => String::new(),
    }
}

fn unavailable_outcome(budget: Budget, reason: impl Into<String>) -> Outcome {
    let why = reason.into();
    tracing::warn!(reason = %why, "persistent records not captured");
    Outcome {
        selection: crate::select::persistent::PersistentSelection {
            admitted: Vec::new(),
            matrix: unavailable_all(why.clone(), 0),
            bytes_admitted: budget.spent(),
            budget,
        },
        records: None,
        reason: Some(why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_only_master_pin_hook_is_exact_and_defaults_to_pinned() {
        assert_eq!(master_pin_from(None), None);
        assert_eq!(master_pin_from(Some("0")), Some(false));
        assert_eq!(master_pin_from(Some("1")), Some(true));
        for off in ["", "false", "00", " 0", "no"] {
            assert_eq!(master_pin_from(Some(off)), None, "{off:?}");
        }
    }

    /// The synchronous adapter must drive GETs on the captured handle from a
    /// blocking thread, without building a nested runtime. A loopback server
    /// is the only origin this test touches.
    #[derive(Debug)]
    struct FixedTransport;

    impl crate::http::HttpTransport for FixedTransport {
        fn send(&self, request: crate::http::HttpRequest) -> crate::http::SendFuture {
            Box::pin(async move {
                Ok(crate::http::HttpResponse::in_memory(
                    200,
                    request.url,
                    Some("text/plain"),
                    vec![b"99000004 aabbccdd\n".to_vec()],
                ))
            })
        }
    }

    #[tokio::test]
    async fn the_transport_adapter_drives_a_plain_get_from_a_blocking_thread() {
        let handle = Handle::current();
        let log = crate::http::RequestLog::new();
        let fetcher = Fetcher::with_http(
            crate::http::Transport::local(),
            log.clone(),
            std::sync::Arc::new(crate::http::ZeroSleep),
            std::sync::Arc::new(FixedTransport),
        )
        .expect("fetcher");
        let url = "https://dispatcher.invalid/base_revision";
        let for_the_task = fetcher.clone();
        let body = crate::tasks::join_blocking("dispatcher query", move || {
            let transport = FetcherTransport {
                fetcher: &for_the_task,
                handle,
            };
            <FetcherTransport<'_> as BaseRevisionFetcher>::get(&transport, url)
        })
        .await
        .expect("the blocking task joins")
        .expect("the GET succeeds");
        assert_eq!(body, "99000004 aabbccdd\n");
        let rows = log.records();
        assert_eq!(rows.len(), 1, "one GET, recorded once");
        assert_eq!(rows[0].url, url);
        assert_eq!(
            rows[0].kind,
            Kind::Api,
            "the dispatcher uses the closed api kind"
        );
        assert_eq!(rows[0].status, 200);
    }
}
