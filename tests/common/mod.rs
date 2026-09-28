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
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

use girpr::sophon::{SophonChunk, SophonChunkFile, SophonChunkManifest};
use prost::Message as _;

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
}

fn md5hex(data: &[u8]) -> String {
    format!("{:x}", md5::compute(data))
}

fn zstd_bytes(data: &[u8]) -> Vec<u8> {
    zstd::stream::encode_all(std::io::Cursor::new(data), 3).expect("zstd encode fixture")
}

pub fn build_fixture() -> GameFixture {
    let latest_tag = "9.9.9-test".to_string();
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
    let mut pb_files: Vec<SophonChunkFile> = Vec::new();
    for f in &files {
        let mut pb_chunks: Vec<SophonChunk> = Vec::new();
        for c in &f.chunks {
            let compressed = zstd_bytes(&c.data);
            chunk_store.insert(c.id.clone(), compressed.clone());
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
    }
}

// ---------------------------------------------------------------------------
// temp dirs (no extra deps)
// ---------------------------------------------------------------------------

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Fresh unique temp dir for one test. Caller removes it at test end.
pub fn temp_dir(name: &str) -> PathBuf {
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "girpr_it_{}_{}_{}",
        std::process::id(),
        n,
        name
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp game dir");
    p
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
}

pub struct MockServer {
    pub hyp_base: String,
    pub sophon_base: String,
    /// chunk ids fetched since start (proves reuse / no-download cases)
    pub chunk_requests: Arc<Mutex<Vec<String>>>,
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
        let sophon_base =
            format!("http://127.0.0.1:{port}/downloader/sophon_chunk/api");
        let manifest_prefix =
            format!("http://127.0.0.1:{port}/manifests/test_build");
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

        let log = chunk_requests.clone();
        let handle = tokio::spawn(async move {
            loop {
                let (socket, _) = match listener.accept().await {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let responses = responses.clone();
                let fixture = fixture.clone();
                let log = log.clone();
                tokio::spawn(async move {
                    serve_one(socket, &responses, &fixture, &log).await;
                });
            }
        });

        // Hold lengths for the unused warning (manifest bytes live in fixture).
        let _ = manifest_len;
        Self {
            hyp_base,
            sophon_base,
            chunk_requests,
            handle,
        }
    }

    pub fn chunk_hits(&self) -> Vec<String> {
        self.chunk_requests.lock().unwrap().clone()
    }
}

struct Responses {
    configs: Vec<u8>,
    branches: Vec<u8>,
    deprecated: Vec<u8>,
    build_latest: Vec<u8>,
    build_not_found: Vec<u8>,
    latest_tag: String,
}

fn build_responses(
    fx: &GameFixture,
    opts: &MockOpts,
    manifest_prefix: &str,
    chunk_prefix: &str,
) -> Responses {
    let raw_manifest_len = zstd::stream::decode_all(std::io::Cursor::new(
        fx.manifest_zstd.clone(),
    ))
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
                "default_download_mode": "DOWNLOAD_MODE_CHUNK",
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
    Responses {
        configs: serde_json::to_vec(&configs).unwrap(),
        branches: serde_json::to_vec(&branches).unwrap(),
        deprecated: serde_json::to_vec(&deprecated).unwrap(),
        build_latest: serde_json::to_vec(&build_latest).unwrap(),
        build_not_found: br#"{"retcode":-202,"message":"not found","data":null}"#.to_vec(),
        latest_tag: fx.latest_tag.clone(),
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

    let (status, ctype, body): (&str, &str, Vec<u8>) =
        if target.contains("getGameConfigs") {
            ("200 OK", "application/json", r.configs.clone())
        } else if target.contains("getGameBranches") {
            ("200 OK", "application/json", r.branches.clone())
        } else if target.contains("getGameDeprecatedFileConfigs") {
            ("200 OK", "application/json", r.deprecated.clone())
        } else if target.contains("getBuild") {
            match query_tag(&target) {
                Some(t) if t != r.latest_tag => (
                    "200 OK",
                    "application/json",
                    r.build_not_found.clone(),
                ),
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
            if id == fx.manifest_id {
                (
                    "200 OK",
                    "application/octet-stream",
                    fx.manifest_zstd.clone(),
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
            match fx.chunk_store.get(id) {
                Some(bytes) => {
                    log.lock().unwrap().push(id.to_string());
                    ("200 OK", "application/octet-stream", bytes.clone())
                }
                None => ("404 Not Found", "text/plain", b"no such chunk".to_vec()),
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
