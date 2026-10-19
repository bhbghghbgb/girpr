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
    }
}

/// The **local** build's manifest, with every file's chunks re-flowed: the
/// same chunks (same ids, same sizes, same MD5s) at *different* offsets.
///
/// This is the shape a version upgrade leaves behind: the file's content
/// moved, so the bytes a chunk wants are somewhere else in the file. The
/// file's own size and MD5 describe the re-flowed content, because that is
/// what is on disk.
pub fn build_reflowed_manifest(fx: &GameFixture) -> LocalManifest {
    let mut pb_files: Vec<SophonChunkFile> = Vec::new();
    for f in &fx.files {
        let mut pb_chunks: Vec<SophonChunk> = Vec::new();
        let mut data: Vec<u8> = Vec::new();
        // The chunks in reverse order: the content moved down by one chunk.
        for c in f.chunks.iter().rev() {
            let offset = data.len() as i64;
            data.extend_from_slice(&c.data);
            pb_chunks.push(SophonChunk {
                id: c.id.clone(),
                uncompressed_md5: md5hex(&c.data),
                offset,
                compressed_size: c.data.len() as i64,
                uncompressed_size: c.data.len() as i64,
                unknown: 0,
                compressed_md5: String::new(),
            });
        }
        pb_files.push(SophonChunkFile {
            file: f.rel.clone(),
            chunks: pb_chunks,
            is_folder: false,
            size: data.len() as i64,
            md5: md5hex(&data),
        });
    }
    let manifest = SophonChunkManifest { chuncks: pb_files };
    let mut raw = Vec::new();
    manifest.encode(&mut raw).expect("prost encode fixture");
    LocalManifest {
        id: "manifest_test_game_local".to_string(),
        bytes: raw,
    }
}

/// The file content a reflowed manifest describes: the chunks in the order
/// the local manifest lists them.
pub fn reflowed_file_data(fx: &GameFixture, index: usize) -> Vec<u8> {
    let f = &fx.files[index];
    let mut data = Vec::new();
    for c in f.chunks.iter().rev() {
        data.extend_from_slice(&c.data);
    }
    data
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
/// A manifest served for the **local** build.
///
/// A version upgrade re-flows a file's content: the chunks keep their ids,
/// sizes and MD5s, but they sit at *different* offsets than the latest
/// manifest's. That is the shape the in-place repair has to cope with, and
/// it cannot be modelled by the latest manifest alone.
#[derive(Clone)]
pub struct LocalManifest {
    pub id: String,
    /// The raw protobuf bytes.
    pub bytes: Vec<u8>,
}

#[derive(Default)]
pub struct MockOpts {
    pub deprecated_files: Vec<String>,
    pub scan_dir: String,
    pub res_dir: String,
    pub cache_dir: String,
    /// The local build's manifest, when it is not the latest one. `None` (the
    /// default) makes every non-latest tag answer `retcode -202`, which is
    /// what models "the server no longer serves that build".
    pub local_manifest: Option<LocalManifest>,
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
    /// The build body served for a non-latest tag, when the fixture brings a
    /// local manifest. `None` → every non-latest tag answers `retcode -202`.
    build_local: Option<Vec<u8>>,
    /// The local manifest's id and bytes, when there is one.
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
    // The local build: the same shape, its own manifest. Only served when the
    // fixture brings one — otherwise a non-latest tag is "not found", which
    // is what models a build the server no longer serves.
    let (build_local, local_manifest) = match opts.local_manifest.as_ref() {
        Some(m) => {
            // The manifest is served zstd-compressed, and its checksum is the
            // MD5 of the *decompressed* bytes — same as the real one.
            let raw = m.bytes.clone();
            let checksum = md5hex(&raw);
            let compressed = zstd_bytes(&raw);
            let body = serde_json::json!({
                "retcode": 0,
                "message": "OK",
                "data": {
                    "build_id": "build_test_local",
                    "tag": "1.0.0-old",
                    "manifests": [{
                        "category_id": "10016",
                        "category_name": "game-test",
                        "matching_field": "game",
                        "manifest": {
                            "id": m.id,
                            "checksum": checksum,
                            "compressed_size": compressed.len() as i64,
                            "uncompressed_size": raw.len() as i64
                        },
                        "chunk_download": {"url_prefix": chunk_prefix, "url_suffix": ""},
                        "manifest_download": {"url_prefix": manifest_prefix, "url_suffix": ""}
                    }]
                }
            });
            (
                Some(serde_json::to_vec(&body).unwrap()),
                Some((m.id.clone(), compressed)),
            )
        }
        None => (None, None),
    };
    Responses {
        configs: serde_json::to_vec(&configs).unwrap(),
        branches: serde_json::to_vec(&branches).unwrap(),
        deprecated: serde_json::to_vec(&deprecated).unwrap(),
        build_latest: serde_json::to_vec(&build_latest).unwrap(),
        build_not_found: br#"{"retcode":-202,"message":"not found","data":null}"#.to_vec(),
        build_local,
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
                // The fixture brings a local build: serve it.
                Some(body) => ("200 OK", "application/json", body.clone()),
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
        } else if let Some((local_id, bytes)) = &r.local_manifest
            && id == local_id
        {
            (
                "200 OK",
                "application/octet-stream",
                bytes.clone(),
            )
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
