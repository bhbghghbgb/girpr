//! Offline mock API for integration/e2e tests.
//!
//! Spins a tiny HTTP server on `127.0.0.1:<ephemeral>` (no external network)
//! that mimics the live HoYoPlay + Sophon shapes used by `girpr`:
//!
//! - `GET <hyp>/getGameConfigs` -> `{"launch_configs":[...]}`
//! - `GET <hyp>/getGameBranches` -> `{"game_branches":[...]}`
//! - `GET <sophon>/getBuild[?tag=]` -> chunk build, or `retcode -202` for
//!   unknown tags (exercises the local-build fallback in `repair::run`)
//! - `GET <hyp>/getGameDeprecatedFileConfigs` -> deprecated list
//! - `GET <manifest_prefix>/<manifest_id>` -> zstd(protobuf manifest)
//! - `GET <chunk_prefix>/<chunk_id>` -> zstd(chunk payload)
//!
//! Shapes were captured from the live `hk4e_global` endpoints (Sept 2026):
//! same wrapper (`retcode`/`message`/`data`), same node names, manifest
//! sizes as int-or-string, chunk/manifest URL prefix + id joining.

// The module is shared by several test binaries and each of them uses a
// different part of it, so most of it is dead code in any one of them. The
// same allowance `tempdir.rs` carries, for the same reason.
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use girpr::sophon::{SophonChunk, SophonChunkFile, SophonChunkManifest};
use prost::Message as _;

// The scratch-directory helper lives beside this file rather than in it, so the
// other integration targets can reach it without dragging in the mock server.
#[path = "tempdir.rs"]
pub mod tempdir;

// ---------------------------------------------------------------------------
// tiny deterministic game fixture (2 files, 3 chunks, all in-memory)
// ---------------------------------------------------------------------------

pub struct FixtureChunk {
    pub id: String,
    pub data: Vec<u8>,
    pub offset: i64,
}

pub struct FixtureFile {
    pub rel: String,
    pub data: Vec<u8>,
    pub md5: String,
    pub chunks: Vec<FixtureChunk>,
}

pub struct GameFixture {
    pub latest_tag: String,
    pub manifest_id: String,
    pub manifest_zstd: Vec<u8>,
    pub manifest_checksum: String,
    pub files: Vec<FixtureFile>,
    /// chunk id -> zstd(compressed payload) bytes served by the mock
    pub chunk_store: HashMap<String, Vec<u8>>,
    /// chunk id -> zstd(payload with one byte flipped). Well-formed: same
    /// uncompressed length, different MD5, so the chunk decodes and then fails
    /// verification. That is what makes a file *retry* rather than just fail.
    pub corrupt_chunk_store: HashMap<String, Vec<u8>>,
    /// A second manifest, describing the **old** file's chunk layout, served
    /// for a non-latest tag. `None` = the server does not serve the local tag
    /// at all (`getBuild(local)` answers -202), which is the default.
    pub local_manifest: Option<FixtureManifest>,
    /// The old file's bytes — the thing the repair has to move — for the
    /// fixtures that have one.
    pub old_data: Option<(String, Vec<u8>)>,
    /// Every file that is **on disk**, by path.
    ///
    /// The old side's bytes are here rather than derived from the sides on
    /// purpose: the shapes the repair has to survive include a file that is
    /// longer on disk than its manifest describes (the dead tail the trim
    /// releases) and one that is shorter (the growth the repair writes).
    pub disk: BTreeMap<String, Vec<u8>>,
}

/// A second manifest: the old file's chunk layout.
pub struct FixtureManifest {
    pub id: String,
    pub zstd: Vec<u8>,
    pub checksum: String,
}

/// The fixture's latest tag. The mock serves the chunk manifest for
/// `getBuild?tag=<LATEST_TAG>` and returns `retcode -202` for any other tag, so
/// writing this into `config.ini` is how a test says "the local build's
/// manifests are available" (i.e. chunk reuse is available).
pub const LATEST_TAG: &str = "9.9.9-test";

fn md5hex(data: &[u8]) -> String {
    format!("{:x}", md5::compute(data))
}

fn zstd_bytes(data: &[u8]) -> Vec<u8> {
    zstd::stream::encode_all(std::io::Cursor::new(data), 3).expect("zstd encode fixture")
}

// ---------------------------------------------------------------------------
// the fixture description
// ---------------------------------------------------------------------------
//
// A fixture is **described**, not built: the old side, the new side and the
// bytes on disk are three lists, and a shape is a shape rather than a builder
// of its own. That is what lets one mock serve a uniform shift, a mixed
// shift, a shrinking file and a file with a hole in its coverage — the four
// shapes the repair's own branches turn on — without four builders that
// each know a shape the others do not.

/// One chunk of a file: its bytes, where they land, and the id the mock
/// serves it under.
///
/// The id is part of the description because the mock's chunk store is keyed
/// by it and the repair's request log reports it: a test asserts against the
/// ids it named here.
#[derive(Clone, Debug)]
pub struct ChunkSpec {
    /// The id the store serves this chunk under.
    pub id: String,
    /// The chunk's bytes.
    pub payload: Vec<u8>,
    /// Where the chunk lands in the file.
    pub offset: i64,
}

impl ChunkSpec {
    /// A chunk with the given id, bytes and offset.
    pub fn new(id: &str, payload: Vec<u8>, offset: i64) -> Self {
        Self {
            id: id.to_string(),
            payload,
            offset,
        }
    }
}

/// One file: a path and the chunks it is made of.
#[derive(Clone, Debug)]
pub struct FileSpec {
    /// The path, relative to `--game-path`, `/`-separated.
    pub rel: String,
    /// The chunks, in manifest order.
    pub chunks: Vec<ChunkSpec>,
}

impl FileSpec {
    /// A file with the given path and chunks.
    pub fn new(rel: &str, chunks: Vec<ChunkSpec>) -> Self {
        Self {
            rel: rel.to_string(),
            chunks,
        }
    }
}

/// The ways a fixture's manifest can lie about itself.
///
/// A manifest is a claim about bytes the tool has not seen yet, so every
/// field in it is a hint. These are the ways a hint can be wrong, and each
/// one has a branch in the repair that has to survive it.
#[derive(Clone, Debug, Default)]
pub struct Lies {
    /// Chunk ids whose declared `uncompressed_md5` no payload can match: the
    /// digest gate has to refuse the bytes it is given.
    pub wrong_chunk_md5: Vec<String>,
    /// Chunk ids whose declared `compressed_size` is not the blob's length:
    /// the size is a hint, and the real length is what crosses the wire.
    pub wrong_chunk_size: Vec<String>,
    /// Chunk ids the store serves as a body that is **not a zstd frame**: the
    /// decode has to fail on it rather than land garbage in the install.
    pub not_zstd: Vec<String>,
}

/// A whole fixture: the new side, the old side, and what is on disk.
#[derive(Clone, Debug, Default)]
pub struct FixtureSpec {
    /// The files the **latest** manifest describes — the repair's target.
    pub new: Vec<FileSpec>,
    /// The files the **local** build's manifest describes — the reuse map's
    /// sources. `None` = the server does not serve the local tag at all,
    /// which is what "no chunk reuse" looks like.
    pub old: Option<Vec<FileSpec>>,
    /// The bytes on disk, by path. A path that is not listed does not exist.
    pub disk: BTreeMap<String, Vec<u8>>,
    /// The ways the manifest lies.
    pub lies: Lies,
}

impl FixtureSpec {
    /// A fixture with the given new-side files and nothing else: no local
    /// manifest, and nothing on disk.
    pub fn new(files: Vec<FileSpec>) -> Self {
        Self {
            new: files,
            ..Default::default()
        }
    }

    /// The same fixture, with a local build's manifest — the reuse map's
    /// sources.
    pub fn with_old(mut self, files: Vec<FileSpec>) -> Self {
        self.old = Some(files);
        self
    }

    /// The same fixture, with a file on disk.
    pub fn with_disk(mut self, rel: &str, bytes: Vec<u8>) -> Self {
        self.disk.insert(rel.to_string(), bytes);
        self
    }

    /// The same fixture, with the manifest lying in the given ways.
    pub fn with_lies(mut self, lies: Lies) -> Self {
        self.lies = lies;
        self
    }
}

/// The bytes of a file laid out at its chunks' offsets.
///
/// A gap between two chunks is **zero bytes**, not nothing: the file on disk
/// is a byte range, and the hole is a part of it.
pub fn lay_out(chunks: &[ChunkSpec]) -> Vec<u8> {
    let end = chunks
        .iter()
        .map(|c| c.offset.max(0) as usize + c.payload.len())
        .max()
        .unwrap_or(0);
    let mut out = vec![0u8; end];
    for c in chunks {
        let at = c.offset.max(0) as usize;
        out[at..at + c.payload.len()].copy_from_slice(&c.payload);
    }
    out
}

/// Build the mock's fixture from a description.
///
/// The two sides are two lists of files and the bytes on disk are a third, so
/// every shape the repair has a branch for is a description rather than a
/// builder.
pub fn build_fixture_spec(spec: &FixtureSpec) -> GameFixture {
    let mut chunk_store: HashMap<String, Vec<u8>> = HashMap::new();
    let mut corrupt_chunk_store: HashMap<String, Vec<u8>> = HashMap::new();
    let mut files: Vec<FixtureFile> = Vec::new();

    // The new side, as the mock's own view of it.
    for f in &spec.new {
        let data = lay_out(&f.chunks);
        files.push(FixtureFile {
            rel: f.rel.clone(),
            md5: md5hex(&data),
            data,
            chunks: f
                .chunks
                .iter()
                .map(|c| FixtureChunk {
                    id: c.id.clone(),
                    data: c.payload.clone(),
                    offset: c.offset,
                })
                .collect(),
        });
    }

    // The manifest, and the store it points at.
    let mut pb_files: Vec<SophonChunkFile> = Vec::new();
    for f in &files {
        let mut pb_chunks: Vec<SophonChunk> = Vec::new();
        for c in &f.chunks {
            let compressed = zstd_bytes(&c.data);
            // A chunk the store serves as something that is not a zstd frame
            // is served **raw**: the decode is what has to fail, and a
            // well-formed frame with wrong bytes would fail somewhere else.
            let served = if spec.lies.not_zstd.contains(&c.id) {
                c.data.clone()
            } else {
                compressed.clone()
            };
            chunk_store.insert(c.id.clone(), served);
            // Same length, one byte different: decodes, fails its MD5.
            let mut bad = c.data.clone();
            if let Some(last) = bad.len().checked_sub(1) {
                bad[last] ^= 0xFF;
            }
            corrupt_chunk_store.insert(c.id.clone(), zstd_bytes(&bad));
            pb_chunks.push(SophonChunk {
                id: c.id.clone(),
                uncompressed_md5: if spec.lies.wrong_chunk_md5.contains(&c.id) {
                    // A digest no payload can have: the chunk must be refused.
                    "00000000000000000000000000000000".to_string()
                } else {
                    md5hex(&c.data)
                },
                offset: c.offset,
                compressed_size: if spec.lies.wrong_chunk_size.contains(&c.id) {
                    compressed.len() as i64 + 7
                } else {
                    compressed.len() as i64
                },
                uncompressed_size: c.data.len() as i64,
                unknown: 0,
                compressed_md5: md5hex(&compressed),
            });
        }
        pb_files.push(SophonChunkFile {
            file: f.rel.clone(),
            chunks: pb_chunks,
            is_folder: false,
            size: f.data.len() as i64,
            md5: f.md5.clone(),
        });
    }
    let (manifest_zstd, manifest_checksum) = encode_manifest(&pb_files);

    // The local build's manifest: the old side's chunk layout, and nothing
    // else. Its chunks are never fetched — the local manifest only ever feeds
    // the reuse map.
    let local_manifest = spec.old.as_ref().map(|old| {
        let mut old_pb: Vec<SophonChunkFile> = Vec::new();
        for f in old {
            let data = lay_out(&f.chunks);
            let mut pb_chunks: Vec<SophonChunk> = Vec::new();
            for c in &f.chunks {
                let compressed = zstd_bytes(&c.payload);
                pb_chunks.push(SophonChunk {
                    id: c.id.clone(),
                    uncompressed_md5: md5hex(&c.payload),
                    offset: c.offset,
                    compressed_size: compressed.len() as i64,
                    uncompressed_size: c.payload.len() as i64,
                    unknown: 0,
                    compressed_md5: md5hex(&compressed),
                });
            }
            old_pb.push(SophonChunkFile {
                file: f.rel.clone(),
                chunks: pb_chunks,
                is_folder: false,
                size: data.len() as i64,
                md5: md5hex(&data),
            });
        }
        let (zstd, checksum) = encode_manifest(&old_pb);
        FixtureManifest {
            id: "manifest_test_local_001".to_string(),
            zstd,
            checksum,
        }
    });

    // The old file's bytes: the first file on disk. Kept as its own field
    // because that is the shape the moved fixture has always had.
    let old_data = spec
        .disk
        .iter()
        .next()
        .map(|(rel, bytes)| (rel.clone(), bytes.clone()));

    GameFixture {
        latest_tag: LATEST_TAG.to_string(),
        manifest_id: "manifest_test_game_001".to_string(),
        manifest_zstd,
        manifest_checksum,
        files,
        chunk_store,
        corrupt_chunk_store,
        local_manifest,
        old_data,
        disk: spec.disk.clone(),
    }
}

pub fn build_fixture() -> GameFixture {
    // Fixed payloads (distinct bytes per chunk so corruption is detectable).
    let foo_c0 = vec![0x41u8; 64]; // 'A' x64
    let foo_c1 = vec![0x42u8; 64]; // 'B' x64
    let bar_c0 = vec![0x43u8; 48]; // 'C' x48

    build_fixture_spec(&FixtureSpec::new(vec![
        FileSpec::new(
            "game/foo.dat",
            vec![
                ChunkSpec::new("chunk_foo0", foo_c0, 0),
                ChunkSpec::new("chunk_foo1", foo_c1, 64),
            ],
        ),
        FileSpec::new(
            "game/bar.dat",
            vec![ChunkSpec::new("chunk_bar0", bar_c0, 0)],
        ),
    ]))
}

/// A fixture whose **old** file has a different chunk layout from the new
/// one — the content moved, so the repair has to move bytes *inside* the
/// file.
///
/// ```text
///   old:  [0,64)=P   [64,128)=Q   [128,192)=R
///   new:  [0,64)=R   [64,128)=P   [128,192)=T
/// ```
///
/// The new chunk 0 (`R`) has its source *later* in the file (offset 128) and
/// the new chunk 1 (`P`) has its source *earlier* (offset 0). The sources are
/// therefore in **both** directions, which is the only case where the two
/// write orders differ: the RAM tier keeps every source (it stages the bytes
/// first) and the ordered tier keeps one direction's worth.
///
/// The old file's manifest is served for a non-latest tag, so the run loads
/// it as the local build and the reuse map points at the old offsets — which
/// is what makes the sources "elsewhere in the file" at all.
pub fn build_moved_fixture() -> GameFixture {
    let p = vec![0x50u8; 64]; // 'P'
    let q = vec![0x51u8; 64]; // 'Q'
    let r = vec![0x52u8; 64]; // 'R'
    let t = vec![0x54u8; 64]; // 'T'

    // The old file: P || Q || R, with its own chunk ids — they are never
    // fetched, because the local manifest only ever feeds the reuse map.
    let old_data = lay_out(&[
        ChunkSpec::new("local_p", p.clone(), 0),
        ChunkSpec::new("local_q", q.clone(), 64),
        ChunkSpec::new("local_r", r.clone(), 128),
    ]);

    build_fixture_spec(
        &FixtureSpec::new(vec![FileSpec::new(
            "game/moved.dat",
            vec![
                ChunkSpec::new("chunk_r", r.clone(), 0),
                ChunkSpec::new("chunk_p", p.clone(), 64),
                ChunkSpec::new("chunk_t", t.clone(), 128),
            ],
        )])
        .with_old(vec![FileSpec::new(
            "game/moved.dat",
            vec![
                ChunkSpec::new("local_p", p, 0),
                ChunkSpec::new("local_q", q, 64),
                ChunkSpec::new("local_r", r, 128),
            ],
        )])
        .with_disk("game/moved.dat", old_data),
    )
}

/// Encode a manifest body the way the server sends it: zstd over the
/// protobuf, with the checksum being the MD5 of the *decompressed* bytes.
fn encode_manifest(files: &[SophonChunkFile]) -> (Vec<u8>, String) {
    let manifest = SophonChunkManifest {
        chuncks: files.to_vec(),
    };
    let mut raw = Vec::new();
    manifest.encode(&mut raw).expect("prost encode fixture");
    let checksum = md5hex(&raw);
    (zstd_bytes(&raw), checksum)
}

// ---------------------------------------------------------------------------
// temp dirs (no extra deps)
// ---------------------------------------------------------------------------

/// Fresh unique temp dir for one test, removed when the returned guard drops.
///
/// The name carries the pid, a timestamp and a per-process counter so two runs
/// — or two `cargo test` invocations — never share a tree, and the guard means
/// the tree is removed even when a case fails before its last line. See
/// [`tempdir`] for why each part is there.
pub fn temp_dir(name: &str) -> tempdir::TempDir {
    tempdir::TempDir::new("girpr_it", name)
}

pub fn write_game_file(game_dir: &std::path::Path, rel: &str, bytes: &[u8]) {
    let rel_fs = rel.replace('/', std::path::MAIN_SEPARATOR_STR);
    let p = game_dir.join(rel_fs);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(&p, bytes).expect("write game file");
}

pub fn read_game_file(game_dir: &std::path::Path, rel: &str) -> Option<Vec<u8>> {
    let rel_fs = rel.replace('/', std::path::MAIN_SEPARATOR_STR);
    std::fs::read(game_dir.join(rel_fs)).ok()
}

// ---------------------------------------------------------------------------
// mock server
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct MockOpts {
    pub deprecated_files: Vec<String>,
    pub scan_dir: String,
    pub res_dir: String,
    pub cache_dir: String,
    /// `getGameConfigs.default_download_mode`. `None` serves the live
    /// `DOWNLOAD_MODE_CHUNK`; `Some(s)` serves `s` verbatim, so a test can send
    /// `Some("")` to model the API omitting the field.
    pub download_mode: Option<String>,
    /// Serve every chunk request as `500`. Models a reachable-but-broken origin,
    /// which is what exercises the retry/backoff paths (`--fail-fast`) without
    /// a dead host and its connection timeouts.
    pub fail_chunks: bool,
    /// Serve every chunk request as `404`. Models the blob routes' real answer
    /// to an id they do not have: the live CDN (`autopatchhk.yuanshen.com`,
    /// `Server: AliyunOSS`) answers `404` with an `application/xml` `NoSuchKey`
    /// body and never a `200`-with-a-code, and an id under the wrong build's
    /// prefix is a `404` too. The distinction from `fail_chunks` is the whole
    /// point: a 5xx is worth retrying and a 404 is not, so only one of the two
    /// can be told apart by how many times it was asked for.
    pub not_found_chunks: bool,
    /// Serve every manifest request as `404`, for the same reason. The id is
    /// published atomically with the build that serves it, so this models a
    /// build whose manifests are gone rather than a not-yet-propagated one.
    pub not_found_manifests: bool,
    /// Serve the first N chunk responses with valid-but-**wrong** bytes: the
    /// same uncompressed length and a well-formed body, so the chunk decodes
    /// fine and fails on its MD5. Models a corrupt or half-propagated origin,
    /// which is what makes a file retry — and therefore what distinguishes
    /// "bytes of promoted files" from "bytes that crossed the wire".
    pub corrupt_first_chunks: u32,
    /// Serve these chunk ids corrupt on their **first** request.
    ///
    /// `corrupt_first_chunks` is a counter over the requests, so it can only
    /// ever break the *first* chunk a file asks for. Naming the ids is what
    /// breaks a later one — the case where an attempt has already written a
    /// chunk that passed its gate before it fails, and the next attempt has to
    /// find that chunk still correct.
    pub corrupt_chunks: Vec<String>,
    /// Advertise a manifest checksum the blob does not have. The blob itself
    /// is served unchanged, so the only thing that can catch it is the
    /// checksum gate — the MD5 of the **decompressed** bytes.
    pub wrong_manifest_checksum: bool,
}

pub struct MockServer {
    pub hyp_base: String,
    pub sophon_base: String,
    /// chunk ids fetched since start (proves reuse / no-download cases)
    pub chunk_requests: Arc<Mutex<Vec<String>>>,
    /// every request target seen, in order (proves query params, e.g. the
    /// per-launcher `channel`/`sub_channel` on `getGameDeprecatedFileConfigs`)
    pub targets: Arc<Mutex<Vec<String>>>,
    handle: tokio::task::JoinHandle<()>,
}

impl MockServer {
    pub fn stop(&self) {
        self.handle.abort();
    }

    pub async fn start(fixture: GameFixture, opts: MockOpts) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock server");
        let port = listener.local_addr().expect("local addr").port();
        let hyp_base = format!("http://127.0.0.1:{port}/hyp/hyp-connect/api");
        let sophon_base = format!("http://127.0.0.1:{port}/downloader/sophon_chunk/api");
        let manifest_prefix = format!("http://127.0.0.1:{port}/manifests/test_build");
        let chunk_prefix = format!("http://127.0.0.1:{port}/chunks/test_build");

        let manifest_len = fixture.manifest_zstd.len();
        let responses = Arc::new(build_responses(
            &fixture,
            &opts,
            &manifest_prefix,
            &chunk_prefix,
        ));
        let fixture = Arc::new(fixture);
        let chunk_requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let targets: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let log = chunk_requests.clone();
        let seen = targets.clone();
        let handle = tokio::spawn(async move {
            loop {
                let (socket, _) = match listener.accept().await {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let responses = responses.clone();
                let fixture = fixture.clone();
                let log = log.clone();
                let seen = seen.clone();
                tokio::spawn(async move {
                    serve_one(socket, &responses, &fixture, &log, &seen).await;
                });
            }
        });

        // Hold lengths for the unused warning (manifest bytes live in fixture).
        let _ = manifest_len;
        Self {
            hyp_base,
            sophon_base,
            chunk_requests,
            targets,
            handle,
        }
    }

    pub fn chunk_hits(&self) -> Vec<String> {
        self.chunk_requests.lock().unwrap().clone()
    }

    /// Every request target the server has seen, in order.
    pub fn request_targets(&self) -> Vec<String> {
        self.targets.lock().unwrap().clone()
    }

    /// The query string of the first request whose target contains `needle`.
    pub fn first_target_with(&self, needle: &str) -> Option<String> {
        self.request_targets()
            .into_iter()
            .find(|t| t.contains(needle))
    }
}

struct Responses {
    configs: Vec<u8>,
    branches: Vec<u8>,
    deprecated: Vec<u8>,
    build_latest: Vec<u8>,
    build_not_found: Vec<u8>,
    /// The `getBuild` answer for a **local** tag, when the server serves one.
    /// `None` = the local tag is not served, which is the default.
    build_local: Option<Vec<u8>>,
    /// The local manifest's body, by id.
    local_manifest: Option<(String, Vec<u8>)>,
    latest_tag: String,
    fail_chunks: bool,
    not_found_chunks: bool,
    not_found_manifests: bool,
    corrupt_first_chunks: u32,
    /// Chunk ids to serve corrupt on their **first** request, whatever the
    /// request order.
    ///
    /// `corrupt_first_chunks` is a counter over the requests, so it can only
    /// ever break the *first* chunk a file asks for. This one names the ids,
    /// which is what a test needs to break a *later* chunk — the case where an
    /// attempt has already written a chunk that passed its gate before it
    /// fails.
    corrupt_chunks: Vec<String>,
    /// The ids whose first response was already served corrupt. An id is in
    /// here from its first corrupt response onwards, so the second request for
    /// the same id gets the good bytes: a flaky origin, not a broken one.
    corrupt_served: Mutex<HashSet<String>>,
    /// Chunk responses served so far, so the corrupt-N counter is per-server
    /// and not per-connection.
    served: AtomicU64,
}

fn build_responses(
    fx: &GameFixture,
    opts: &MockOpts,
    manifest_prefix: &str,
    chunk_prefix: &str,
) -> Responses {
    let raw_manifest_len = zstd::stream::decode_all(std::io::Cursor::new(fx.manifest_zstd.clone()))
        .expect("decode fixture manifest")
        .len();
    let configs = serde_json::json!({
        "retcode": 0,
        "message": "OK",
        "data": {
            "launch_configs": [{
                "game": {"id": "gopR6Cufr3", "biz": "hk4e_global"},
                "exe_file_name": "GenshinImpact.exe",
                "audio_pkg_scan_dir": opts.scan_dir,
                "audio_pkg_res_dir": opts.res_dir,
                "audio_pkg_cache_dir": opts.cache_dir,
                "default_download_mode": opts
                    .download_mode
                    .clone()
                    .unwrap_or_else(|| "DOWNLOAD_MODE_CHUNK".to_string()),
                "res_category_dir": "",
                "blacklist_dir": "",
                "enable_resource_blacklist": false
            }]
        }
    });
    let branches = serde_json::json!({
        "retcode": 0,
        "message": "OK",
        "data": {
            "game_branches": [{
                "game": {"id": "gopR6Cufr3", "biz": "hk4e_global"},
                "main": {
                    "package_id": "pkg_test",
                    "branch": "main",
                    "password": "",
                    "tag": fx.latest_tag,
                    "diff_tags": []
                },
                "pre_download": null
            }]
        }
    });
    let deprecated_files: Vec<serde_json::Value> = opts
        .deprecated_files
        .iter()
        .map(|n| serde_json::json!({"name": n}))
        .collect();
    let deprecated = serde_json::json!({
        "retcode": 0,
        "message": "OK",
        "data": {
            "deprecated_file_configs": [{
                "game": {"id": "gopR6Cufr3", "biz": "hk4e_global"},
                "deprecated_files": deprecated_files
            }]
        }
    });
    let build_latest = serde_json::json!({
        "retcode": 0,
        "message": "OK",
        "data": {
            "build_id": "build_test_001",
            "tag": fx.latest_tag,
            "manifests": [{
                "category_id": "10016",
                "category_name": "game-test",
                "matching_field": "game",
                "manifest": {
                    "id": fx.manifest_id,
                    "checksum": if opts.wrong_manifest_checksum {
                        // A checksum no blob can have: the manifest must be
                        // refused, and the run must fail as metadata.
                        "00000000000000000000000000000000".to_string()
                    } else {
                        fx.manifest_checksum.clone()
                    },
                    "compressed_size": fx.manifest_zstd.len() as i64,
                    "uncompressed_size": raw_manifest_len as i64
                },
                "chunk_download": {"url_prefix": chunk_prefix, "url_suffix": ""},
                "manifest_download": {"url_prefix": manifest_prefix, "url_suffix": ""}
            }]
        }
    });
    // The local build, when the server serves one: a different manifest, and
    // so a different chunk layout for the same path. That is what makes the
    // repair move bytes inside the file.
    let build_local = fx.local_manifest.as_ref().map(|m| {
        serde_json::json!({
            "retcode": 0,
            "message": "OK",
            "data": {
                "build_id": "build_test_old",
                "tag": "9.9.8-test",
                "manifests": [{
                    "category_id": "10016",
                    "category_name": "game-test",
                    "matching_field": "game",
                    "manifest": {
                        "id": m.id,
                        "checksum": m.checksum,
                        "compressed_size": m.zstd.len() as i64,
                        "uncompressed_size": 0
                    },
                    "chunk_download": {"url_prefix": chunk_prefix, "url_suffix": ""},
                    "manifest_download": {"url_prefix": manifest_prefix, "url_suffix": ""}
                }]
            }
        })
    });
    let local_manifest = fx
        .local_manifest
        .as_ref()
        .map(|m| (m.id.clone(), m.zstd.clone()));
    Responses {
        configs: serde_json::to_vec(&configs).unwrap(),
        branches: serde_json::to_vec(&branches).unwrap(),
        deprecated: serde_json::to_vec(&deprecated).unwrap(),
        build_latest: serde_json::to_vec(&build_latest).unwrap(),
        build_not_found: br#"{"retcode":-202,"message":"not found","data":null}"#.to_vec(),
        build_local: build_local.map(|b| serde_json::to_vec(&b).unwrap()),
        local_manifest,
        latest_tag: fx.latest_tag.clone(),
        fail_chunks: opts.fail_chunks,
        not_found_chunks: opts.not_found_chunks,
        not_found_manifests: opts.not_found_manifests,
        corrupt_first_chunks: opts.corrupt_first_chunks,
        corrupt_chunks: opts.corrupt_chunks.clone(),
        corrupt_served: Mutex::new(HashSet::new()),
        served: AtomicU64::new(0),
    }
}

fn query_tag(target: &str) -> Option<String> {
    let q = target.split_once('?')?.1;
    for pair in q.split('&') {
        if let Some(v) = pair.strip_prefix("tag=") {
            return Some(v.to_string());
        }
    }
    None
}

async fn serve_one(
    socket: tokio::net::TcpStream,
    r: &Responses,
    fx: &GameFixture,
    log: &Arc<Mutex<Vec<String>>>,
    seen: &Arc<Mutex<Vec<String>>>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut sock = socket;
    let mut buf = vec![0u8; 65536];
    let mut got = 0usize;
    // Read until end of headers (GET has no body).
    loop {
        match sock.read(&mut buf[got..]).await {
            Ok(0) => break,
            Ok(n) => {
                got += n;
                if buf[..got].windows(4).any(|w| w == b"\r\n\r\n") || got >= buf.len() {
                    break;
                }
            }
            Err(_) => return,
        }
    }
    let head = String::from_utf8_lossy(&buf[..got]).to_string();
    let target = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();
    seen.lock().unwrap().push(target.clone());

    let (status, ctype, body): (&str, &str, Vec<u8>) = if target.contains("getGameConfigs") {
        ("200 OK", "application/json", r.configs.clone())
    } else if target.contains("getGameBranches") {
        ("200 OK", "application/json", r.branches.clone())
    } else if target.contains("getGameDeprecatedFileConfigs") {
        ("200 OK", "application/json", r.deprecated.clone())
    } else if target.contains("getBuild") {
        match query_tag(&target) {
            Some(t) if t != r.latest_tag => match &r.build_local {
                // The local build, when the server serves one.
                Some(b) => ("200 OK", "application/json", b.clone()),
                None => ("200 OK", "application/json", r.build_not_found.clone()),
            },
            _ => ("200 OK", "application/json", r.build_latest.clone()),
        }
    } else if target.contains("/manifests/") {
        let id = target
            .split('?')
            .next()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .unwrap_or("");
        if r.not_found_manifests {
            ("404 Not Found", "text/plain", b"no such manifest".to_vec())
        } else if id == fx.manifest_id {
            (
                "200 OK",
                "application/octet-stream",
                fx.manifest_zstd.clone(),
            )
        } else if let Some((local_id, body)) = &r.local_manifest
            && id == local_id
        {
            ("200 OK", "application/octet-stream", body.clone())
        } else {
            ("404 Not Found", "text/plain", b"no such manifest".to_vec())
        }
    } else if target.contains("/chunks/") {
        let id = target
            .split('?')
            .next()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .unwrap_or("");
        if r.not_found_chunks || r.fail_chunks {
            // Still logged, on both branches: "was this chunk even asked for,
            // and how many times" is the assertion that tells a 404 (asked once)
            // from a 5xx (asked five), so a 404 has to reach this log too.
            log.lock().unwrap().push(id.to_string());
            if r.not_found_chunks {
                // The shape the live CDN uses, so a test cannot accidentally
                // depend on a content-type the origin never sends.
                (
                    "404 Not Found",
                    "application/xml",
                    b"<?xml version=\"1.0\"?><Error><Code>NoSuchKey</Code></Error>".to_vec(),
                )
            } else {
                (
                    "500 Internal Server Error",
                    "text/plain",
                    b"chunk store is down".to_vec(),
                )
            }
        } else {
            match fx.chunk_store.get(id) {
                Some(bytes) => {
                    log.lock().unwrap().push(id.to_string());
                    let n = r.served.fetch_add(1, Ordering::Relaxed);
                    // The named ids are corrupt on their **first** request
                    // only: the second one gets the good bytes, which is what
                    // a flaky origin looks like.
                    let first_time = {
                        let mut served = r.corrupt_served.lock().unwrap();
                        r.corrupt_chunks.iter().any(|c| c == id) && served.insert(id.to_string())
                    };
                    if n < r.corrupt_first_chunks as u64 || first_time {
                        // Well-formed, same uncompressed length, wrong MD5.
                        let bad = fx
                            .corrupt_chunk_store
                            .get(id)
                            .cloned()
                            .unwrap_or_else(|| bytes.clone());
                        ("200 OK", "application/octet-stream", bad)
                    } else {
                        ("200 OK", "application/octet-stream", bytes.clone())
                    }
                }
                None => {
                    // An id the store has never heard of is the same event as
                    // `not_found_chunks`, and it is logged for the same reason.
                    log.lock().unwrap().push(id.to_string());
                    (
                        "404 Not Found",
                        "application/xml",
                        b"<?xml version=\"1.0\"?><Error><Code>NoSuchKey</Code></Error>".to_vec(),
                    )
                }
            }
        }
    } else {
        ("404 Not Found", "text/plain", b"unknown route".to_vec())
    };

    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = sock.write_all(header.as_bytes()).await;
    let _ = sock.write_all(&body).await;
    let _ = sock.flush().await;
}
