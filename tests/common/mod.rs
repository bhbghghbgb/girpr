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

use std::collections::HashMap;
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

pub fn build_fixture() -> GameFixture {
    let latest_tag = LATEST_TAG.to_string();
    let manifest_id = "manifest_test_game_001".to_string();

    // Fixed payloads (distinct bytes per chunk so corruption is detectable).
    let foo_c0 = vec![0x41u8; 64]; // 'A' x64
    let foo_c1 = vec![0x42u8; 64]; // 'B' x64
    let bar_c0 = vec![0x43u8; 48]; // 'C' x48

    let mut foo_data = Vec::new();
    foo_data.extend_from_slice(&foo_c0);
    foo_data.extend_from_slice(&foo_c1);

    let files = vec![
        FixtureFile {
            rel: "game/foo.dat".to_string(),
            data: foo_data,
            md5: String::new(),
            chunks: vec![
                FixtureChunk {
                    id: "chunk_foo0".to_string(),
                    data: foo_c0,
                    offset: 0,
                },
                FixtureChunk {
                    id: "chunk_foo1".to_string(),
                    data: foo_c1,
                    offset: 64,
                },
            ],
        },
        FixtureFile {
            rel: "game/bar.dat".to_string(),
            data: bar_c0.clone(),
            md5: String::new(),
            chunks: vec![FixtureChunk {
                id: "chunk_bar0".to_string(),
                data: bar_c0,
                offset: 0,
            }],
        },
    ];

    // Fill file md5s (can't do inline above because data moves).
    let mut files = files;
    for f in &mut files {
        let mut cat = Vec::new();
        for c in &f.chunks {
            cat.extend_from_slice(&c.data);
        }
        f.data = cat;
        f.md5 = md5hex(&f.data);
    }

    // Protobuf manifest + zstd + checksum (decompressed-bytes MD5, like prod).
    let mut chunk_store: HashMap<String, Vec<u8>> = HashMap::new();
    let mut corrupt_chunk_store: HashMap<String, Vec<u8>> = HashMap::new();
    let mut pb_files: Vec<SophonChunkFile> = Vec::new();
    for f in &files {
        let mut pb_chunks: Vec<SophonChunk> = Vec::new();
        for c in &f.chunks {
            let compressed = zstd_bytes(&c.data);
            chunk_store.insert(c.id.clone(), compressed.clone());
            // Same length, one byte different: decodes, fails its MD5.
            let mut bad = c.data.clone();
            let last = bad.len() - 1;
            bad[last] ^= 0xFF;
            corrupt_chunk_store.insert(c.id.clone(), zstd_bytes(&bad));
            pb_chunks.push(SophonChunk {
                id: c.id.clone(),
                uncompressed_md5: md5hex(&c.data),
                offset: c.offset,
                compressed_size: compressed.len() as i64,
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
    let manifest = SophonChunkManifest { chuncks: pb_files };
    let mut raw = Vec::new();
    manifest.encode(&mut raw).expect("prost encode fixture");
    let manifest_checksum = md5hex(&raw);
    let manifest_zstd = zstd_bytes(&raw);

    GameFixture {
        latest_tag,
        manifest_id,
        manifest_zstd,
        manifest_checksum,
        files,
        chunk_store,
        corrupt_chunk_store,
        local_manifest: None,
        old_data: None,
    }
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

    // The new file: R || P || T.
    let mut new_data = Vec::new();
    new_data.extend_from_slice(&r);
    new_data.extend_from_slice(&p);
    new_data.extend_from_slice(&t);
    let files = vec![FixtureFile {
        rel: "game/moved.dat".to_string(),
        data: new_data.clone(),
        md5: md5hex(&new_data),
        chunks: vec![
            FixtureChunk {
                id: "chunk_r".to_string(),
                data: r.clone(),
                offset: 0,
            },
            FixtureChunk {
                id: "chunk_p".to_string(),
                data: p.clone(),
                offset: 64,
            },
            FixtureChunk {
                id: "chunk_t".to_string(),
                data: t.clone(),
                offset: 128,
            },
        ],
    }];

    let mut chunk_store: HashMap<String, Vec<u8>> = HashMap::new();
    let mut corrupt_chunk_store: HashMap<String, Vec<u8>> = HashMap::new();
    let mut pb_files: Vec<SophonChunkFile> = Vec::new();
    for f in &files {
        let mut pb_chunks: Vec<SophonChunk> = Vec::new();
        for c in &f.chunks {
            let compressed = zstd_bytes(&c.data);
            chunk_store.insert(c.id.clone(), compressed.clone());
            let mut bad = c.data.clone();
            let last = bad.len() - 1;
            bad[last] ^= 0xFF;
            corrupt_chunk_store.insert(c.id.clone(), zstd_bytes(&bad));
            pb_chunks.push(SophonChunk {
                id: c.id.clone(),
                uncompressed_md5: md5hex(&c.data),
                offset: c.offset,
                compressed_size: compressed.len() as i64,
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

    // The old file: P || Q || R, with its own chunk ids — they are never
    // fetched, because the local manifest only ever feeds the reuse map.
    let mut old_data = Vec::new();
    old_data.extend_from_slice(&p);
    old_data.extend_from_slice(&q);
    old_data.extend_from_slice(&r);
    let mut old_chunks: Vec<SophonChunk> = Vec::new();
    for (id, data, offset) in [
        ("local_p", &p, 0i64),
        ("local_q", &q, 64),
        ("local_r", &r, 128),
    ] {
        let compressed = zstd_bytes(data);
        old_chunks.push(SophonChunk {
            id: id.to_string(),
            uncompressed_md5: md5hex(data),
            offset,
            compressed_size: compressed.len() as i64,
            uncompressed_size: data.len() as i64,
            unknown: 0,
            compressed_md5: md5hex(&compressed),
        });
    }
    let old_files = vec![SophonChunkFile {
        file: "game/moved.dat".to_string(),
        chunks: old_chunks,
        is_folder: false,
        size: old_data.len() as i64,
        md5: md5hex(&old_data),
    }];
    let (local_zstd, local_checksum) = encode_manifest(&old_files);

    GameFixture {
        latest_tag: LATEST_TAG.to_string(),
        manifest_id: "manifest_test_moved_001".to_string(),
        manifest_zstd,
        manifest_checksum,
        files,
        chunk_store,
        corrupt_chunk_store,
        local_manifest: Some(FixtureManifest {
            id: "manifest_test_moved_old_001".to_string(),
            zstd: local_zstd,
            checksum: local_checksum,
        }),
        old_data: Some(("game/moved.dat".to_string(), old_data)),
    }
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
                    "checksum": fx.manifest_checksum,
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
                    if n < r.corrupt_first_chunks as u64 {
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
