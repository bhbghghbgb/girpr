//! Persistent manifests and full-file hotfix downloads.
//!
//! Two additions, deliberately without product wiring (docs/03 §7.8, commit 5):
//!
//! - the three descriptors from [`DispatchInfo`] are fetched through the
//!   shared reqwest client and retry policy, verified against their descriptor
//!   (size, then MD5) **before** parsing, and kept as raw bytes in memory only.
//!   The plan half never writes a mirror or a marker;
//! - a full-file hotfix downloader streams a body to a temp sibling, gates it
//!   on size + XXH64/MD5, publishes atomically via `girpr-util-path`, then
//!   re-reads and re-verifies the published file. The alternate URL is tried
//!   on a 404 only — never on a digest failure — and retryable statuses
//!   (408/429/5xx) are retried on the same URL with `girpr-retry`'s backoff.
//!
//! Every boundary an offline test needs is injectable: the reqwest client,
//! the backoff policy, the URLs, the target path and the attempt count. Tests
//! point all of them at a loopback origin and a scratch directory; no test
//! opens a production network path.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::fmt;
use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{Stream, StreamExt};
use girpr_digest::{Algorithms, DigestBytes, Md5, hash_reader};
use girpr_dispatch::ManifestDescriptor;
use girpr_persistent_plan::{ObservedDigest, PersistentRow, parse_rows, verify_observed};
use girpr_util_path::atomic::write_atomic_with;
use tempfile::NamedTempFile;

use crate::retry::{BackoffPolicy, Retry, retry};
use crate::sophon::stream::retryable;

/// Attempts per HTTP request. Kept injectable on the download request for
/// tests that count alternate switches rather than spend backoff.
const ATTEMPTS: u32 = 5;
/// Deadline on a small buffered manifest body read. Large full-file bodies
/// are streamed and deliberately have no total-request timeout.
const MANIFEST_BODY_DEADLINE: Duration = Duration::from_secs(30);
/// Hard ceiling on a raw manifest body. The descriptor's `expected_size` is
/// the gate; this is only the guard against an unbounded hostile response.
const MAX_MANIFEST_BYTES: usize = 16 << 20;
/// Both algorithms in one pass: the row decides which one gates, but the
/// downloader always has the other one available for the re-read report.
const DIGEST_ALGOS: Algorithms = Algorithms::new(true, false, true);
const READ_BUF: usize = 256 << 10;

// ---------------------------------------------------------------------------
// Manifests: fetch -> verify -> parse, bytes never written
// ---------------------------------------------------------------------------

/// A verified raw manifest, still in memory, paired with its descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedManifest {
    pub descriptor: ManifestDescriptor,
    pub raw: Vec<u8>,
}

/// A descriptor gate failure, kept typed so a caller can log which gate
/// failed without parsing error strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestVerifyError {
    Size { expected: u64, actual: u64 },
    Md5 { expected: String, actual: String },
}

impl fmt::Display for ManifestVerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Size { expected, actual } => {
                write!(
                    f,
                    "manifest size mismatch: expected {expected}, got {actual}"
                )
            }
            Self::Md5 { expected, actual } => write!(
                f,
                "manifest MD5 mismatch: expected {expected}, got {actual}"
            ),
        }
    }
}

impl std::error::Error for ManifestVerifyError {}

/// Size first, then MD5, against the values from the descriptor NDJSON —
/// never against a hardcoded digest.
pub fn verify_manifest_bytes(
    descriptor: &ManifestDescriptor,
    raw: &[u8],
) -> std::result::Result<(), ManifestVerifyError> {
    let actual = raw.len() as u64;
    if actual != descriptor.expected_size {
        return Err(ManifestVerifyError::Size {
            expected: descriptor.expected_size,
            actual,
        });
    }
    let md5 = girpr_digest::md5_hex(raw);
    if !md5.eq_ignore_ascii_case(&descriptor.expected_md5) {
        return Err(ManifestVerifyError::Md5 {
            expected: descriptor.expected_md5.clone(),
            actual: md5,
        });
    }
    Ok(())
}

/// Fetch one manifest. The caller owns concurrency: this function uses the
/// supplied shared client, so a later phase can run the three descriptors
/// under the existing `ManifestBudget`; here it is one request at a time.
pub async fn fetch_manifest(
    client: &reqwest::Client,
    backoff: &BackoffPolicy,
    descriptor: &ManifestDescriptor,
) -> Result<FetchedManifest> {
    let url = descriptor.url.as_str();
    retry(ATTEMPTS, backoff, |_attempt| async move {
        let resp = match client.get(url).send().await {
            Ok(r) => r,
            Err(e) => return Retry::Retry(anyhow!("GET manifest {url}: {e}")),
        };
        let status = resp.status();
        if !status.is_success() {
            let e = anyhow!("manifest status {status} url={url}");
            // A required branch: a retryable status is worth another try, a
            // decided status (including 404) is a final manifest failure.
            return if retryable(status.as_u16()) {
                Retry::Retry(e)
            } else {
                Retry::Fatal(e)
            };
        }
        let body = match tokio::time::timeout(MANIFEST_BODY_DEADLINE, resp.bytes()).await {
            Ok(Ok(body)) => body,
            Ok(Err(e)) => return Retry::Retry(anyhow!("read manifest body {url}: {e}")),
            Err(_) => {
                return Retry::Retry(anyhow!(
                    "read manifest body timed out after {MANIFEST_BODY_DEADLINE:?} url={url}"
                ));
            }
        };
        if body.len() > MAX_MANIFEST_BYTES {
            return Retry::Fatal(anyhow!(
                "manifest body {url} exceeds {MAX_MANIFEST_BYTES} bytes"
            ));
        }
        let raw = body.to_vec();
        // Verify BEFORE parsing. Deterministic for these bytes: a digest
        // mismatch is fatal, not five backoffs to learn the same thing.
        if let Err(e) = verify_manifest_bytes(descriptor, &raw) {
            return Retry::Fatal(anyhow::Error::new(e).context(format!("verify manifest {url}")));
        }
        Retry::Done(FetchedManifest {
            descriptor: descriptor.clone(),
            raw,
        })
    })
    .await
}

/// Fetch all three descriptors in order. The caller supplies the three from
/// `DispatchInfo::manifest_digests`; no URL is hardcoded here.
pub async fn fetch_all_manifests(
    client: &reqwest::Client,
    backoff: &BackoffPolicy,
    descriptors: &[ManifestDescriptor; 3],
) -> Result<Vec<FetchedManifest>> {
    let mut out = Vec::with_capacity(descriptors.len());
    for descriptor in descriptors {
        out.push(fetch_manifest(client, backoff, descriptor).await?);
    }
    Ok(out)
}

/// Verify again at the parse boundary and decode the NDJSON into rows.
/// Blank lines are skipped and extra keys are ignored by the pure parser.
pub fn parse_manifest(descriptor: &ManifestDescriptor, raw: &[u8]) -> Result<Vec<PersistentRow>> {
    verify_manifest_bytes(descriptor, raw).context("verify manifest before parse")?;
    parse_rows(raw)
        .context("parse manifest NDJSON")
        .map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Full-file downloader
// ---------------------------------------------------------------------------

/// The one injectable input of a hotfix download.
pub struct DownloadRequest<'a> {
    pub client: &'a reqwest::Client,
    pub backoff: &'a BackoffPolicy,
    pub primary_url: &'a str,
    pub alternate_url: &'a str,
    /// The row's size and preferred digest are the gate.
    pub row: &'a PersistentRow,
    /// Cleaned rel path resolved under the game directory by the caller.
    pub target: &'a Path,
    pub attempts: u32,
    /// Write-charging seam: runs inside `publish_verified` immediately before
    /// any publish (the atomic write, or the temp-sibling `keep()` when the
    /// target cannot be opened). Returning `false` aborts with the typed
    /// [`WriteCeilingStopped`] signal. Production charges its write meters
    /// here so a ceiling cannot be crossed by a file that has already landed.
    pub before_publish: Option<Arc<dyn Fn(u64) -> bool + Send + Sync>>,
    /// Test seam: runs after the atomic publish, before the mandatory re-read.
    /// Production leaves this `None`.
    pub after_publish: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
}

impl<'a> DownloadRequest<'a> {
    pub fn new(
        client: &'a reqwest::Client,
        backoff: &'a BackoffPolicy,
        primary_url: &'a str,
        alternate_url: &'a str,
        row: &'a PersistentRow,
        target: &'a Path,
    ) -> Self {
        Self {
            client,
            backoff,
            primary_url,
            alternate_url,
            row,
            target,
            attempts: ATTEMPTS,
            before_publish: None,
            after_publish: None,
        }
    }
}

/// What a successful download call did. The byte count is the verified
/// on-disk size, which is what the run's meters must charge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadOutcome {
    Written {
        path: PathBuf,
        bytes: u64,
    },
    Skipped {
        path: PathBuf,
        bytes: u64,
    },
    /// The target could not be opened even after clearing read-only, so the
    /// verified bytes stay at their temp sibling instead of failing the run.
    TempFallback {
        path: PathBuf,
        bytes: u64,
    },
}

impl DownloadOutcome {
    pub fn path(&self) -> &Path {
        match self {
            Self::Written { path, .. }
            | Self::Skipped { path, .. }
            | Self::TempFallback { path, .. } => path,
        }
    }

    pub fn bytes(&self) -> u64 {
        match self {
            Self::Written { bytes, .. }
            | Self::Skipped { bytes, .. }
            | Self::TempFallback { bytes, .. } => *bytes,
        }
    }

    /// Whether a fresh copy of the bytes was put down in this call.
    pub fn wrote(&self) -> bool {
        !matches!(self, Self::Skipped { .. })
    }
}

/// Downcast marker for a 404 on one URL: it is the only status that switches
/// to the alternate, and it must survive the retry driver unchanged.
#[derive(Debug)]
struct NotFound(String);

impl fmt::Display for NotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "hotfix not found at {}", self.0)
    }
}

impl std::error::Error for NotFound {}

/// Typed stop signal from a caller's `before_publish` charge.
///
/// A write ceiling refusing the bytes is not a failed download: the caller has
/// already tripped its hard-stop marker, and the execute half downcasts this
/// error so the file is not counted as failed (which would change the exit
/// code) and no bytes are published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteCeilingStopped;

impl fmt::Display for WriteCeilingStopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("write ceiling stopped the hotfix before publish")
    }
}

impl std::error::Error for WriteCeilingStopped {}

/// A verified body in its temp sibling, with the observation from the stream.
struct FetchedTemp {
    file: NamedTempFile,
    digest: ObservedDigest,
}

/// Stream one attempt into a fresh temp sibling and gate it before returning.
async fn fetch_to_temp(
    client: &reqwest::Client,
    backoff: &BackoffPolicy,
    url: &str,
    row: &PersistentRow,
    parent: &Path,
    attempts: u32,
) -> Result<FetchedTemp> {
    retry(attempts, backoff, |_attempt| async move {
        let resp = match client.get(url).send().await {
            Ok(r) => r,
            Err(e) => return Retry::Retry(anyhow!("GET hotfix {url}: {e}")),
        };
        let status = resp.status();
        if status.as_u16() == 404 {
            return Retry::Fatal(NotFound(url.to_owned()).into());
        }
        if !status.is_success() {
            let e = anyhow!("hotfix status {status} url={url}");
            return if retryable(status.as_u16()) {
                Retry::Retry(e)
            } else {
                Retry::Fatal(e)
            };
        }
        let mut temp = match NamedTempFile::new_in(parent) {
            Ok(t) => t,
            Err(e) => {
                return Retry::Fatal(anyhow!("create hotfix temp in {}: {e}", parent.display()));
            }
        };
        let digest =
            match stream_hotfix(resp.bytes_stream(), row.file_size, temp.as_file_mut()).await {
                Ok(d) => d,
                Err(e) => return Retry::Fatal(e),
            };
        let verdict = verify_observed(row, &digest);
        if !verdict.is_ok() {
            // A digest failure never switches to the alternate: the body came
            // from this URL and is definitively wrong for this row.
            return Retry::Fatal(anyhow!("hotfix gate at {url}: {verdict}"));
        }
        Retry::Done(FetchedTemp { file: temp, digest })
    })
    .await
}

/// Feed a body into the temp file, hashing on the way past and refusing a
/// body that crosses the row's declared size. The row's digest decides which
/// of the two algorithms gates, but both are computed in one pass.
async fn stream_hotfix<S, B, E>(
    mut body: S,
    expected: u64,
    file: &mut File,
) -> Result<ObservedDigest>
where
    S: Stream<Item = std::result::Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: fmt::Display,
{
    let mut sink = girpr_digest::Sink::new(DIGEST_ALGOS);
    let mut size = 0u64;
    while let Some(piece) = body.next().await {
        let bytes = piece.map_err(|e| anyhow!("read hotfix body: {e}"))?;
        let chunk = bytes.as_ref();
        let total = size.saturating_add(chunk.len() as u64);
        if total > expected {
            bail!("hotfix body exceeds the {expected} bytes the manifest declares");
        }
        file.write_all(chunk).context("write hotfix temp")?;
        sink.update(chunk);
        size = total;
    }
    Ok(observed_from(sink.finish()))
}

fn observed_from(bytes: DigestBytes) -> ObservedDigest {
    ObservedDigest::of_stream(
        bytes.bytes,
        Md5::from(bytes.md5.unwrap_or([0u8; 16])),
        u64::from_be_bytes(bytes.xxhash64.unwrap_or([0u8; 8])),
    )
}

/// Re-read a file and observe its size and both digests. One read buffer per
/// call; full-file bodies are never resident as a whole.
fn observe_file(path: &Path) -> Result<ObservedDigest> {
    let file = File::open(path).with_context(|| format!("open {} for verify", path.display()))?;
    let mut buf = vec![0u8; READ_BUF];
    let bytes = hash_reader(file, DIGEST_ALGOS, &mut buf)
        .with_context(|| format!("read {} for verify", path.display()))?;
    Ok(observed_from(bytes))
}

/// `Ok(true)` means the file at `path` already satisfies the row.
fn verify_path(row: &PersistentRow, path: &Path) -> Result<bool> {
    let observed = observe_file(path)?;
    Ok(verify_observed(row, &observed).is_ok())
}

/// C17(b): strip the read-only attribute on this one target immediately
/// before overwriting it. No tree-wide walk, no mutation of unrelated files.
fn clear_readonly(path: &Path) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if !meta.permissions().readonly() {
        return;
    }
    let mut permissions = meta.permissions();
    permissions.set_readonly(false);
    if let Err(e) = std::fs::set_permissions(path, permissions) {
        tracing::warn!(
            path = %path.display(),
            error = %e,
            "could not clear read-only before publishing; will try the temp path"
        );
    }
}

/// Could this target be opened for writing? A directory or a locked file
/// answers no; that is when the verified temp sibling becomes the deliverable
/// rather than an error (docs/03 §7.5.2 item 4).
fn can_open_for_write(path: &Path) -> bool {
    std::fs::OpenOptions::new().write(true).open(path).is_ok()
}

/// Publish a verified temp file and re-read the result. The caller's charge,
/// when present, runs before either publish route; its `after_publish` hook
/// runs between publish and re-read so a test can corrupt the published bytes
/// and prove the second gate catches them.
fn publish_verified(
    row: &PersistentRow,
    target: &Path,
    fetched: FetchedTemp,
    before_publish: Option<Arc<dyn Fn(u64) -> bool + Send + Sync>>,
    after_publish: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
) -> Result<DownloadOutcome> {
    let bytes = fetched.digest.size;
    let mut temp = fetched.file;
    // Charge before the bytes land: in the normal path this is immediately
    // before `write_atomic_with`, and in the temp fallback it is before
    // `keep()` publishes at the sibling. A refusal leaves nothing published.
    if let Some(charge) = before_publish {
        if !charge(bytes) {
            return Err(anyhow::Error::new(WriteCeilingStopped));
        }
    }
    if target.exists() {
        clear_readonly(target);
    }
    if target.exists() && !can_open_for_write(target) {
        let (kept, path) = temp
            .keep()
            .context("keep verified hotfix at its temp path")?;
        drop(kept);
        if !verify_path(row, &path)? {
            bail!("verified temp failed its re-read at {}", path.display());
        }
        return Ok(DownloadOutcome::TempFallback { path, bytes });
    }

    write_atomic_with(target, false, |writer| {
        let file = temp.as_file_mut();
        file.seek(SeekFrom::Start(0))?;
        io::copy(file, writer)?;
        Ok(())
    })
    .with_context(|| format!("publish hotfix {}", target.display()))?;
    if let Some(hook) = after_publish {
        hook(target);
    }
    if !verify_path(row, target)? {
        bail!(
            "post-publish re-verify failed for {}; refusing to count the file as done",
            target.display()
        );
    }
    Ok(DownloadOutcome::Written {
        path: target.to_path_buf(),
        bytes,
    })
}

/// Whether `target` already satisfies `row` (size + preferred digest).
///
/// The execute half asks this before reserving any bytes, so an intact target
/// is a pure no-op: no download ceiling charge, no write and no HTTP request.
/// A target that cannot be read is simply "not intact" — the download path
/// deals with it, including the read-only/temp fallback.
pub fn hotfix_is_intact(row: &PersistentRow, target: &Path) -> bool {
    target.is_file() && verify_path(row, target).unwrap_or(false)
}

/// Download one full-file hotfix.
///
/// Order: skip if already intact → primary, then alternate on 404 only →
/// temp sibling → digest gate → clear read-only → atomic publish → re-read.
pub async fn download_hotfix(request: DownloadRequest<'_>) -> Result<DownloadOutcome> {
    let DownloadRequest {
        client,
        backoff,
        primary_url,
        alternate_url,
        row,
        target,
        attempts,
        before_publish,
        after_publish,
    } = request;

    if hotfix_is_intact(row, target) {
        return Ok(DownloadOutcome::Skipped {
            path: target.to_path_buf(),
            bytes: row.file_size,
        });
    }
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    // The temp sibling must live beside the target, so its directory has to
    // exist before the first byte is requested. Idempotent with the execute
    // phase's directory setup.
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create hotfix parent {}", parent.display()))?;

    for url in [primary_url, alternate_url] {
        if url.is_empty() {
            continue;
        }
        match fetch_to_temp(client, backoff, url, row, parent, attempts).await {
            Ok(fetched) => {
                return publish_verified(
                    row,
                    target,
                    fetched,
                    before_publish.clone(),
                    after_publish.clone(),
                );
            }
            Err(e) if e.downcast_ref::<NotFound>().is_some() => {
                tracing::debug!(url, "primary hotfix 404; trying the alternate only");
            }
            Err(e) => return Err(e).with_context(|| format!("hotfix download {url}")),
        }
    }
    bail!(
        "hotfix unavailable: both primary {primary_url} and alternate {alternate_url} returned 404"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use girpr_dispatch::ManifestKind;
    use girpr_loopback_mock::{Reply, ScriptedServer};
    use girpr_tempdir::TempDir;

    const RES: &[u8] =
        include_bytes!("../../../tests/fixtures/persistent_inputs/samples/fixtures/res.ndjson");
    const DATA: &[u8] =
        include_bytes!("../../../tests/fixtures/persistent_inputs/samples/fixtures/data.ndjson");
    const HOTFIX_A: &[u8] =
        include_bytes!("../../../tests/fixtures/persistent_inputs/samples/fixtures/hotfix_a.blk");

    fn client() -> reqwest::Client {
        reqwest::Client::builder().build().unwrap()
    }

    fn descriptor(url: String, raw: &[u8]) -> ManifestDescriptor {
        ManifestDescriptor {
            kind: ManifestKind::Res,
            url,
            expected_md5: girpr_digest::md5_hex(raw),
            expected_size: raw.len() as u64,
            mirror_rel: "GenshinImpact_Data/Persistent/res_versions_persist".into(),
        }
    }

    fn fixture_row() -> PersistentRow {
        parse_rows(RES).unwrap().into_iter().next().unwrap()
    }

    fn target(dir: &TempDir) -> PathBuf {
        dir.join("GenshinImpact_Data/Persistent/blocks/00/11111111.blk")
    }

    async fn download(
        server: &ScriptedServer,
        row: &PersistentRow,
        path: &Path,
        backoff: &BackoffPolicy,
    ) -> Result<DownloadOutcome> {
        let http = client();
        let primary = format!("{}/primary/a.blk", server.base());
        let alternate = format!("{}/alternate/a.blk", server.base());
        download_hotfix(DownloadRequest::new(
            &http, backoff, &primary, &alternate, row, path,
        ))
        .await
    }

    #[tokio::test]
    async fn manifest_fetch_verify_and_parse_the_synthetic_fixtures() {
        let server = ScriptedServer::start(vec![Reply::new(200, RES)]);
        let d = descriptor(format!("{}/res.ndjson", server.base()), RES);
        let fetched = fetch_manifest(&client(), &BackoffPolicy::none(), &d)
            .await
            .unwrap();
        assert_eq!(fetched.raw, RES);
        let rows = parse_manifest(&fetched.descriptor, &fetched.raw).unwrap();
        // `isPatch` and `category` are tolerated; the blank line in data.ndjson
        // is exercised by the second case.
        assert_eq!(rows.len(), 2);
        assert!(rows[0].hash.is_some());

        let server = ScriptedServer::start(vec![Reply::new(200, DATA)]);
        let d = descriptor(format!("{}/data.ndjson", server.base()), DATA);
        let fetched = fetch_manifest(&client(), &BackoffPolicy::none(), &d)
            .await
            .unwrap();
        assert_eq!(parse_manifest(&d, &fetched.raw).unwrap().len(), 2);
    }

    #[tokio::test]
    async fn manifest_gate_runs_before_parsing_and_is_typed() {
        let mut d = descriptor("http://mock/res.ndjson".into(), RES);
        d.expected_md5 = "00000000000000000000000000000000".into();
        let server = ScriptedServer::start(vec![Reply::new(200, RES)]);
        d.url = format!("{}/res.ndjson", server.base());
        let err = fetch_manifest(&client(), &BackoffPolicy::none(), &d)
            .await
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("manifest MD5 mismatch"),
            "{err:#}"
        );
        assert_eq!(server.hits(), 1, "a decided body is not retried");

        let mut d = descriptor("http://mock/res.ndjson".into(), RES);
        d.expected_size = RES.len() as u64 + 1;
        assert_eq!(
            verify_manifest_bytes(&d, RES),
            Err(ManifestVerifyError::Size {
                expected: RES.len() as u64 + 1,
                actual: RES.len() as u64,
            })
        );

        // A malformed NDJSON row with a correct descriptor is a typed parse
        // error, not a panic and not a silent skip.
        let bad = b"{\"remoteName\":\"x\"}\n";
        let d = descriptor("http://mock/bad".into(), bad);
        let err = parse_manifest(&d, bad).unwrap_err();
        assert!(format!("{err:#}").contains("manifest line 1"), "{err:#}");
    }

    #[tokio::test]
    async fn good_body_is_gated_and_published_atomically() {
        let server = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
        let dir = TempDir::new("girpr_test_fetch", "good_body");
        let path = target(&dir);
        let row = fixture_row();
        let outcome = download(&server, &row, &path, &BackoffPolicy::none())
            .await
            .unwrap();
        assert_eq!(
            outcome,
            DownloadOutcome::Written {
                path: path.clone(),
                bytes: 64
            }
        );
        assert_eq!(std::fs::read(&path).unwrap(), HOTFIX_A);
        assert_eq!(server.hits(), 1);
        // A second call sees an intact target and makes no request.
        let outcome = download(&server, &row, &path, &BackoffPolicy::none())
            .await
            .unwrap();
        assert!(matches!(outcome, DownloadOutcome::Skipped { .. }));
        assert_eq!(server.hits(), 1);
    }

    /// The charging seam runs once with the verified size, before publish.
    #[tokio::test]
    async fn an_accepted_pre_publish_charge_runs_once_with_the_verified_size() {
        let server = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
        let dir = TempDir::new("girpr_test_fetch", "charge_once");
        let path = target(&dir);
        let http = client();
        let row = fixture_row();
        let primary = format!("{}/primary/a.blk", server.base());
        let alternate = format!("{}/alternate/a.blk", server.base());
        let backoff = BackoffPolicy::none();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let calls_charge = Arc::clone(&calls);
        let seen_charge = Arc::clone(&seen);
        let mut request = DownloadRequest::new(&http, &backoff, &primary, &alternate, &row, &path);
        request.before_publish = Some(Arc::new(move |bytes: u64| {
            calls_charge.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            seen_charge.store(bytes, std::sync::atomic::Ordering::SeqCst);
            true
        }));
        let outcome = download_hotfix(request).await.unwrap();
        assert_eq!(outcome, DownloadOutcome::Written { path, bytes: 64 });
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 64);
    }

    /// A refusal is the typed stop signal, not a download error: nothing is
    /// published, not even a temp sibling.
    #[tokio::test]
    async fn a_refused_pre_publish_charge_stops_before_the_atomic_write() {
        let server = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
        let dir = TempDir::new("girpr_test_fetch", "write_ceiling");
        let path = target(&dir);
        let http = client();
        let row = fixture_row();
        let primary = format!("{}/primary/a.blk", server.base());
        let alternate = format!("{}/alternate/a.blk", server.base());
        let backoff = BackoffPolicy::none();
        let seen = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let seen_charge = Arc::clone(&seen);
        let mut request = DownloadRequest::new(&http, &backoff, &primary, &alternate, &row, &path);
        request.before_publish = Some(Arc::new(move |bytes: u64| {
            seen_charge.store(bytes, std::sync::atomic::Ordering::SeqCst);
            false
        }));
        let err = download_hotfix(request).await.unwrap_err();
        assert_eq!(
            err.downcast_ref::<WriteCeilingStopped>(),
            Some(&WriteCeilingStopped),
            "a ceiling refusal is typed, not a generic download failure"
        );
        assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 64);
        assert!(!path.exists(), "a refused charge publishes nothing");
        let sibling_dir = dir.join("GenshinImpact_Data/Persistent/blocks/00");
        assert_eq!(
            std::fs::read_dir(&sibling_dir).unwrap().count(),
            0,
            "not even a temp sibling is kept"
        );
    }

    #[tokio::test]
    async fn corrupt_body_fails_the_gate_without_publishing() {
        let corrupt = &HOTFIX_A[..32];
        let server = ScriptedServer::start(vec![Reply::new(200, corrupt)]);
        let dir = TempDir::new("girpr_test_fetch", "corrupt_body");
        let path = target(&dir);
        let row = fixture_row();
        let err = download(&server, &row, &path, &BackoffPolicy::none())
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("size mismatch"), "{err:#}");
        assert!(!path.exists(), "a failed gate never publishes");
        let sibling_dir = dir.join("GenshinImpact_Data/Persistent/blocks/00");
        assert_eq!(
            std::fs::read_dir(&sibling_dir).unwrap().count(),
            0,
            "the temp sibling is removed on failure"
        );
        assert_eq!(
            server.hits(),
            1,
            "a digest failure never tries the alternate"
        );
    }

    #[tokio::test]
    async fn primary_404_switches_to_the_alternate_only() {
        let server =
            ScriptedServer::start(vec![Reply::new(404, "missing"), Reply::new(200, HOTFIX_A)]);
        let dir = TempDir::new("girpr_test_fetch", "primary_404");
        let path = target(&dir);
        let row = fixture_row();
        download(&server, &row, &path, &BackoffPolicy::none())
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), HOTFIX_A);
        let targets: Vec<String> = server.requests().into_iter().map(|r| r.target).collect();
        assert_eq!(targets.len(), 2);
        assert!(targets[0].starts_with("/primary/"), "{targets:?}");
        assert!(targets[1].starts_with("/alternate/"), "{targets:?}");
    }

    #[tokio::test]
    async fn retryable_statuses_retry_on_one_url() {
        let server =
            ScriptedServer::start(vec![Reply::new(503, "busy"), Reply::new(200, HOTFIX_A)]);
        let dir = TempDir::new("girpr_test_fetch", "retryable_status");
        let path = target(&dir);
        let row = fixture_row();
        download(&server, &row, &path, &BackoffPolicy::none())
            .await
            .unwrap();
        assert_eq!(server.hits(), 2);
        assert!(
            server
                .requests()
                .iter()
                .all(|r| r.target.starts_with("/primary/")),
            "a 503 never switches to the alternate"
        );
    }

    #[tokio::test]
    async fn md5_and_xxh64_gates_are_preferred_exactly_once() {
        let row = fixture_row();
        let good_hash = row.hash.unwrap();
        let good_md5 = row.md5.clone();

        // Correct XXH64 with a deliberately wrong MD5 still passes: the hash
        // field wins.
        let server = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
        let dir = TempDir::new("girpr_test_fetch", "hash_wins");
        let hash_row = PersistentRow {
            md5: "00000000000000000000000000000000".into(),
            ..row.clone()
        };
        download(&server, &hash_row, &target(&dir), &BackoffPolicy::none())
            .await
            .unwrap();
        assert_eq!(std::fs::read(&target(&dir)).unwrap(), HOTFIX_A);

        // Correct MD5 with a wrong XXH64 fails; no fallback to MD5.
        let server = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
        let dir = TempDir::new("girpr_test_fetch", "wrong_hash");
        let wrong_hash_row = PersistentRow {
            hash: Some(good_hash ^ 1),
            ..row.clone()
        };
        assert!(
            download(
                &server,
                &wrong_hash_row,
                &target(&dir),
                &BackoffPolicy::none()
            )
            .await
            .is_err()
        );
        assert!(!target(&dir).exists());

        // With no hash, MD5 is the gate.
        let server = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
        let dir = TempDir::new("girpr_test_fetch", "md5_fallback");
        let md5_row = PersistentRow {
            hash: None,
            md5: good_md5,
            ..row
        };
        download(&server, &md5_row, &target(&dir), &BackoffPolicy::none())
            .await
            .unwrap();
        assert_eq!(std::fs::read(&target(&dir)).unwrap(), HOTFIX_A);
    }

    #[tokio::test]
    async fn a_declared_size_that_is_too_small_refuses_the_body() {
        let server = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
        let dir = TempDir::new("girpr_test_fetch", "size_cap");
        let row = PersistentRow {
            file_size: 63,
            ..fixture_row()
        };
        let err = download(&server, &row, &target(&dir), &BackoffPolicy::none())
            .await
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("exceeds the 63 bytes"),
            "{err:#}"
        );
        assert!(!target(&dir).exists());
    }

    #[tokio::test]
    async fn published_bytes_are_re_read_and_re_verified() {
        let server = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
        let dir = TempDir::new("girpr_test_fetch", "reverify");
        let path = target(&dir);
        let http = client();
        let row = fixture_row();
        let primary = format!("{}/primary/a.blk", server.base());
        let alternate = format!("{}/alternate/a.blk", server.base());
        let backoff = BackoffPolicy::none();
        let mut request = DownloadRequest::new(&http, &backoff, &primary, &alternate, &row, &path);
        request.after_publish = Some(Arc::new(|p: &Path| {
            std::fs::write(p, b"tampered after publish").unwrap();
        }));
        let err = download_hotfix(request).await.unwrap_err();
        assert!(
            format!("{err:#}").contains("post-publish re-verify"),
            "{err:#}"
        );
    }

    #[tokio::test]
    async fn read_only_target_is_cleared_and_an_unopenable_one_keeps_the_temp() {
        let row = fixture_row();
        let server = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
        let dir = TempDir::new("girpr_test_fetch", "readonly");
        let path = target(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"old bytes").unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();
        let outcome = download(&server, &row, &path, &BackoffPolicy::none())
            .await
            .unwrap();
        assert!(matches!(outcome, DownloadOutcome::Written { .. }));
        assert_eq!(std::fs::read(&path).unwrap(), HOTFIX_A);

        // A directory at the target cannot be opened for writing: the verified
        // temp sibling stays put instead of failing the run.
        let server = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
        let dir = TempDir::new("girpr_test_fetch", "temp_fallback");
        let path = target(&dir);
        std::fs::create_dir_all(&path).unwrap();
        // The parent of the directory-target already exists by construction.
        let outcome = download(&server, &row, &path, &BackoffPolicy::none())
            .await
            .unwrap();
        match outcome {
            DownloadOutcome::TempFallback { path: temp, bytes } => {
                assert_eq!(bytes, 64);
                assert_eq!(std::fs::read(&temp).unwrap(), HOTFIX_A);
            }
            other => panic!("expected a temp fallback, got {other:?}"),
        }
        assert!(path.is_dir(), "the unopenable target is left untouched");
    }
}
