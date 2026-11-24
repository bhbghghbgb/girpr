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
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, anyhow};
use girpr_dispatch::{
    BaseRevisionFetcher, BytesFetcher, DispatchError, DispatchInfo, DispatchQuery, query_dispatch,
};
use girpr_fetch::persistent::{fetch_all_manifests, parse_manifest};
use girpr_model::Region;
use girpr_persistent_plan::{
    ManifestInput, MarkerSet, PersistentPlan, ScopeDecision, build_plan, render_markers,
    scope_decision,
};
use reqwest::Client;
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
/// caller's tracing dispatcher installed. The override is checked first and is
/// the only seam a test needs; an unflagged run never reaches this function.
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
}
