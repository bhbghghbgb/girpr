//! The replay server's own tests, against a **synthetic** mini-bundle.
//!
//! Spec: [`docs/06-artifact-bridge-spec.md`](../docs/06-artifact-bridge-spec.md)
//! §4. The server is what makes the real bundle replayable, and it has its own
//! contract: four listeners, the surgical `url_prefix` rewrite, the
//! `build_id` invariant, the request log, and the recorded failure shapes.
//!
//! **Why a synthetic bundle.** The server's behaviour does not depend on the
//! bundle being 696 MB of real game bytes: it depends on the bundle's
//! *shape*. A mini-bundle with two `getBuild` bodies, one carried manifest
//! per build, one chunk blob and a set of recorded failure shapes is a few
//! kilobytes, and it exercises every route the server has. So these cases run
//! in every `cargo nextest run`, and the real bundle is only needed by the
//! suite that compares `girpr`'s output against the patch
//! (`tests/artifact_bridge.rs`).
//!
//! **What is served is never invented.** Every body comes out of a file the
//! bundle carries; the only synthesized content is the failure shape, and only
//! when the bundle carries none — and then the server *says so* rather than
//! letting a not-found assertion pass on an invented status.

#[path = "artifact_bridge/server.rs"]
mod server;

use std::path::{Path, PathBuf};

use girpr_tempdir::TempDir;
use server::{Bridge, Bundle, HitKind};

// ---------------------------------------------------------------------------
// the synthetic mini-bundle
// ---------------------------------------------------------------------------

/// The versions the mini-bundle stands for.
const OLD_TAG: &str = "7.0.0";
const NEW_TAG: &str = "7.1.0";
const OLD_BUILD: &str = "OLDbuild000001";
const NEW_BUILD: &str = "NEWbuild000001";

/// The manifest ids the two builds declare.
///
/// Each build declares **two**: the `game` one, which the bundle carries, and
/// an audio one, which it does not — the pair is what makes "the id resolves
/// to *this* build's field archive" and "the id resolves to nothing" two
/// different answers.
const OLD_GAME_MANIFEST: &str =
    "manifest_0000000000000001_0000000000000000aaaaaaaaaaaaaaaaaaaaaaaa";
const NEW_GAME_MANIFEST: &str =
    "manifest_0000000000000002_0000000000000000bbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const OLD_AUDIO_MANIFEST: &str =
    "manifest_0000000000000003_0000000000000000cccccccccccccccccccccccccccccccc";
const NEW_AUDIO_MANIFEST: &str =
    "manifest_0000000000000004_0000000000000000dddddddddddddddddddddddddddd";

/// The chunk the bundle carries.
const CHUNK_ID: &str = "616e6fbbf0a97dc9_a21a4420161bca81a78f73b27bfa359a";

/// The CDN the recorded bodies point at. The rewrite replaces its scheme and
/// authority and keeps its path, so the path has to look like production's.
const CDN: &str = "https://autopatchhk.example/client_app/sophon";

/// A synthetic bundle, built rather than captured.
struct MiniBundle {
    dir: PathBuf,
    /// Removes the tree on drop.
    _guard: TempDir,
}

impl MiniBundle {
    /// A mini-bundle that carries the **recorded failure shapes**.
    fn with_shapes() -> Self {
        Self::build(Some(all_shapes()))
    }

    /// A mini-bundle that carries **one** recorded failure shape.
    fn with_one_shape() -> Self {
        Self::build(Some(vec![shape(
            "getbuild_unknown_tag",
            "json",
            200,
            "application/json",
            r#"{"data":null,"message":"not found","retcode":-202}"#,
        )]))
    }

    /// A mini-bundle that carries **no** recorded failure shapes.
    fn without_shapes() -> Self {
        Self::build(None)
    }

    fn build(shapes: Option<Vec<Shape>>) -> Self {
        let guard = TempDir::new("girpr_artifact_server", "mini");
        let dir = guard.to_path_buf();
        std::fs::create_dir_all(dir.join("api")).expect("create api/");
        std::fs::create_dir_all(dir.join("manifests").join(OLD_TAG)).expect("create manifests/");
        std::fs::create_dir_all(dir.join("manifests").join(NEW_TAG)).expect("create manifests/");
        std::fs::create_dir_all(dir.join("chunks").join("blobs")).expect("create chunks/");

        write_json(
            &dir.join("MANIFEST.json"),
            &serde_json::json!({
                "schema_version": 2,
                "tool": "gircap",
                "biz": {"id": "hk4e_global"},
                "versions": {
                    "old": {"tag": OLD_TAG, "build_id": OLD_BUILD},
                    "new": {"tag": NEW_TAG, "build_id": NEW_BUILD},
                },
            }),
        );
        write_json(
            &dir.join("api").join("getGameConfigs.json"),
            &serde_json::json!({
                "retcode": 0,
                "message": "OK",
                "data": {"launch_configs": [{
                    "game": {"id": "gopR6Cufr3", "biz": "hk4e_global"},
                    "exe_file_name": "GenshinImpact.exe",
                    "audio_pkg_scan_dir": "GenshinImpact_Data/Persistent/audio_lang_14",
                    "audio_pkg_res_dir": "",
                    "audio_pkg_cache_dir": "",
                    "default_download_mode": "DOWNLOAD_MODE_CHUNK",
                    "res_category_dir": "",
                    "blacklist_dir": "",
                    "enable_resource_blacklist": false
                }]}
            }),
        );
        write_json(
            &dir.join("api").join("getGameBranches.json"),
            &serde_json::json!({
                "retcode": 0,
                "message": "OK",
                "data": {"game_branches": [{
                    "game": {"id": "gopR6Cufr3", "biz": "hk4e_global"},
                    "main": {
                        "package_id": "pkg_test",
                        "branch": "main",
                        "password": "",
                        "tag": NEW_TAG,
                        "diff_tags": []
                    },
                    "pre_download": null
                }]}
            }),
        );
        write_json(
            &dir.join("api").join("getGameDeprecatedFileConfigs.json"),
            &serde_json::json!({
                "retcode": 0,
                "message": "OK",
                "data": {"deprecated_file_configs": [{
                    "game": {"id": "gopR6Cufr3", "biz": "hk4e_global"},
                    "deprecated_files": []
                }]}
            }),
        );
        write_json(
            &dir.join("api").join(format!("getBuild.{OLD_TAG}.json")),
            &build_body(OLD_BUILD, OLD_TAG, OLD_GAME_MANIFEST, OLD_AUDIO_MANIFEST),
        );
        write_json(
            &dir.join("api").join(format!("getBuild.{NEW_TAG}.json")),
            &build_body(NEW_BUILD, NEW_TAG, NEW_GAME_MANIFEST, NEW_AUDIO_MANIFEST),
        );

        // The carried archives: one per build, under the field the body's
        // `matching_field` names.
        std::fs::write(
            dir.join("manifests")
                .join(OLD_TAG)
                .join("game.manifest.zst"),
            zstd_bytes(b"the old build's game manifest"),
        )
        .expect("write the old manifest archive");
        std::fs::write(
            dir.join("manifests")
                .join(NEW_TAG)
                .join("game.manifest.zst"),
            zstd_bytes(b"the new build's game manifest"),
        )
        .expect("write the new manifest archive");
        std::fs::write(
            dir.join("chunks")
                .join("blobs")
                .join(format!("{CHUNK_ID}.zst")),
            zstd_bytes(b"a chunk's bytes"),
        )
        .expect("write the chunk blob");

        if let Some(shapes) = shapes {
            write_json(
                &dir.join("api").join("error_shapes.json"),
                &serde_json::json!({
                    "schema_version": 2,
                    "shapes": shapes
                        .iter()
                        .map(|s| serde_json::json!({
                            "id": s.id,
                            "family": s.family,
                            "status": s.status,
                            "content_type": s.content_type,
                            "body": s.body,
                        }))
                        .collect::<Vec<_>>(),
                }),
            );
        }

        Self { dir, _guard: guard }
    }

    fn dir(&self) -> &Path {
        &self.dir
    }
}

/// One recorded failure shape.
struct Shape {
    id: &'static str,
    family: &'static str,
    status: u16,
    content_type: &'static str,
    body: &'static str,
}

fn shape(
    id: &'static str,
    family: &'static str,
    status: u16,
    content_type: &'static str,
    body: &'static str,
) -> Shape {
    Shape {
        id,
        family,
        status,
        content_type,
        body,
    }
}

/// The seven recorded shapes, in the order the bundle records them.
///
/// The three blob shapes are the AliyunOSS `NoSuchKey` body the live CDN
/// answers with; the four JSON shapes are the API's own `retcode` answers.
fn all_shapes() -> Vec<Shape> {
    vec![
        shape(
            "blob_missing",
            "blob",
            404,
            "application/xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error>\n  <Code>NoSuchKey</Code>\n  \
             <Message>The specified key does not exist.</Message>\n  \
             <RequestId>6AD78D3CADEC853636E20FAB</RequestId>\n  <HostId></HostId>\n  \
             <Key>client_app/sophon/manifests/cxhpq4g4rgg0/NEWbuild000001/\
             0000000000000000_00000000000000000000000000000000</Key>\n  \
             <EC>0026-00000001</EC>\n  <RecommendDoc></RecommendDoc>\n</Error>\n",
        ),
        shape(
            "chunk_missing",
            "blob",
            404,
            "application/xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error>\n  <Code>NoSuchKey</Code>\n  \
             <Message>The specified key does not exist.</Message>\n  \
             <RequestId>6AD78D3CA8C0BA31366CEA07</RequestId>\n  <HostId></HostId>\n  \
             <Key>client_app/sophon/chunks/cxhpq4g4rgg0/NEWbuild000001/\
             0000000000000000_00000000000000000000000000000000</Key>\n  \
             <EC>0026-00000001</EC>\n  <RecommendDoc></RecommendDoc>\n</Error>\n",
        ),
        shape(
            "blob_wrong_build",
            "blob",
            404,
            "application/xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error>\n  <Code>NoSuchKey</Code>\n  \
             <Message>The specified key does not exist.</Message>\n  \
             <RequestId>6AD78D3C7F52813836AD0FF4</RequestId>\n  <HostId></HostId>\n  \
             <Key>client_app/sophon/chunks/cxhpq4g4rgg0/OLDbuild000001/\
             616e6fbbf0a97dc9_a21a4420161bca81a78f73b27bfa359a</Key>\n  \
             <EC>0026-00000001</EC>\n  <RecommendDoc></RecommendDoc>\n</Error>\n",
        ),
        shape(
            "getbuild_unknown_tag",
            "json",
            200,
            "application/json",
            r#"{"data":null,"message":"not found","retcode":-202}"#,
        ),
        shape(
            "getbuild_bad_package",
            "json",
            200,
            "application/json",
            r#"{"data":null,"message":"invalid params","retcode":-201}"#,
        ),
        shape(
            "getgameconfigs_bad_game_id",
            "json",
            200,
            "application/json",
            r#"{"data":null,"message":"invalid params","retcode":-1001}"#,
        ),
        shape(
            "getgamebranches_bad_game_id",
            "json",
            200,
            "application/json",
            r#"{"retcode":0,"message":"OK","data":{"game_branches":[]}}"#,
        ),
    ]
}

/// A `getBuild` body: one build, two manifests.
fn build_body(
    build_id: &str,
    tag: &str,
    game_manifest: &str,
    audio_manifest: &str,
) -> serde_json::Value {
    let manifest = |id: &str, field: &str| {
        serde_json::json!({
            "category_id": "10016",
            "category_name": field,
            "matching_field": field,
            "manifest": {
                "id": id,
                "checksum": "00000000000000000000000000000000",
                "compressed_size": "128",
                "uncompressed_size": "256",
            },
            "chunk_download": {
                "encryption": 0,
                "password": "",
                "compression": 1,
                "url_prefix": format!("{CDN}/chunks/cxhpq4g4rgg0/{build_id}"),
                "url_suffix": ""
            },
            "manifest_download": {
                "encryption": 0,
                "password": "",
                "compression": 1,
                "url_prefix": format!("{CDN}/manifests/cxhpq4g4rgg0/{build_id}"),
                "url_suffix": ""
            },
            "stats": {
                "compressed_size": "1",
                "uncompressed_size": "1",
                "file_count": "1",
                "chunk_count": "1"
            },
            "deduplicated_stats": {
                "compressed_size": "1",
                "uncompressed_size": "1",
                "file_count": "1",
                "chunk_count": "1"
            }
        })
    };
    serde_json::json!({
        "retcode": 0,
        "message": "OK",
        "data": {
            "build_id": build_id,
            "tag": tag,
            "manifests": [manifest(game_manifest, "game"), manifest(audio_manifest, "en-us")],
        }
    })
}

fn write_json(path: &Path, value: &serde_json::Value) {
    std::fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialize the body"),
    )
    .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

fn zstd_bytes(data: &[u8]) -> Vec<u8> {
    zstd::stream::encode_all(std::io::Cursor::new(data), 3).expect("zstd encode fixture")
}

// ---------------------------------------------------------------------------
// the cases
// ---------------------------------------------------------------------------

/// A one-shot HTTP GET against the bridge, returning `(status, content-type,
/// body)`. Status is the number, so a test compares it against a recorded shape
/// rather than against a rendered status line.
async fn get(url: &str) -> (u16, String, Vec<u8>) {
    let resp = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("GET {url}: {e}"));
    let status = resp.status().as_u16();
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let body = resp.bytes().await.expect("body").to_vec();
    (status, ctype, body)
}

/// The rewrite must be surgical. `sophon::fetch_manifest` MD5-verifies the
/// **decompressed** manifest against `manifest.checksum`, so a checksum that
/// moved kills the run at metadata with exit `2` — a failure that looks like a
/// product bug and is not one. `manifest.id` is the URL path segment.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_rewrite_changes_only_the_two_url_prefixes() {
    let mini = MiniBundle::with_shapes();
    let bridge = Bridge::start(Bundle::open(mini.dir())).await;
    let bundle = bridge.bundle();

    for tag in [&bundle.old_tag, &bundle.new_tag] {
        let recorded: serde_json::Value =
            serde_json::from_slice(&bundle.recorded_build_body(tag)).expect("recorded body");
        let served: serde_json::Value =
            serde_json::from_slice(&bridge.served_build(tag).expect("tag is served"))
                .expect("served body");

        assert_eq!(
            recorded["retcode"], served["retcode"],
            "{tag}: the envelope"
        );
        assert_eq!(recorded["message"], served["message"], "{tag}");
        assert_eq!(
            recorded["data"]["build_id"], served["data"]["build_id"],
            "{tag}"
        );
        let rec = recorded["data"]["manifests"]
            .as_array()
            .expect("recorded[]")
            .clone();
        let srv = served["data"]["manifests"]
            .as_array()
            .expect("served[]")
            .clone();
        assert_eq!(rec.len(), srv.len(), "{tag}: manifest count changed");
        for (r, s) in rec.iter().zip(srv.iter()) {
            // The claim, pinned by comparing everything that is not one of the
            // two rewritten fields. `manifest` is compared whole, so checksum
            // and id are covered; they are then pinned by name below because
            // *why* they must not move is not obvious from an equality.
            for key in [
                "category_id",
                "category_name",
                "matching_field",
                "stats",
                "deduplicated_stats",
            ] {
                assert_eq!(r[key], s[key], "{tag}: {key} must be identical");
            }
            assert_eq!(
                r["manifest"], s["manifest"],
                "{tag}: manifest{{}} identical"
            );
            assert_eq!(
                s["manifest"]["checksum"], r["manifest"]["checksum"],
                "manifest.checksum is MD5 of the DECOMPRESSED manifest, verified before the \
                 protobuf is parsed; moving it is a metadata failure that looks like a product bug"
            );
            assert_eq!(
                s["manifest"]["id"], r["manifest"]["id"],
                "manifest.id is the URL path segment"
            );
            // `url_suffix` is irrelevant — `sophon::join_url` builds
            // `{prefix}/{id}` and never reads it — but it is still recorded,
            // so it must still be the recorded one.
            for field in ["manifest_download", "chunk_download"] {
                for key in ["url_suffix", "encryption", "password", "compression"] {
                    assert_eq!(
                        r[field][key], s[field][key],
                        "{tag}: {field}.{key} must be identical"
                    );
                }
            }
        }
    }
}

/// Each namespace gets its **own** origin, and the path is kept byte-for-byte
/// so a blob route can be "take the last two segments and ignore the rest" —
/// which is what makes the route indistinguishable from production's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_namespace_keeps_its_path_and_gets_its_own_origin() {
    let mini = MiniBundle::with_shapes();
    let bridge = Bridge::start(Bundle::open(mini.dir())).await;
    let bundle = bridge.bundle();
    let o = bridge.origins().clone();
    assert_ne!(
        o.manifest_origin, o.chunk_origin,
        "the two ports must differ"
    );

    for tag in [&bundle.old_tag, &bundle.new_tag] {
        let recorded: serde_json::Value =
            serde_json::from_slice(&bundle.recorded_build_body(tag)).expect("recorded");
        let served: serde_json::Value =
            serde_json::from_slice(&bridge.served_build(tag).expect("body")).expect("json");
        let rec = recorded["data"]["manifests"]
            .as_array()
            .expect("[]")
            .clone();
        let srv = served["data"]["manifests"].as_array().expect("[]").clone();
        for (r, s) in rec.iter().zip(srv.iter()) {
            for (field, origin) in [
                ("manifest_download", &o.manifest_origin),
                ("chunk_download", &o.chunk_origin),
            ] {
                let before = r[field]["url_prefix"].as_str().expect("prefix");
                let after = s[field]["url_prefix"].as_str().expect("prefix");
                assert!(
                    after.starts_with(origin.as_str()),
                    "{field} must name the listener that serves it: {after}"
                );
                assert_eq!(
                    &after[origin.len()..],
                    recorded_path(before),
                    "{field}: only the scheme and authority may change, never the path"
                );
                assert!(
                    after.starts_with("http://127.0.0.1:"),
                    "loopback only: {after}"
                );
            }
        }
    }
}

/// The path of a recorded `url_prefix` — everything after `scheme://authority`.
fn recorded_path(prefix: &str) -> &str {
    let rest = prefix.split_once("://").expect("scheme://").1;
    match rest.find('/') {
        Some(i) => &rest[i..],
        None => "",
    }
}

/// The live ids are `manifest_<16hex>_<32hex>`; the bundle stores archives
/// under *field* names. The translation is built while rewriting the body, so
/// an id resolves only if **this** build declared it and the bundle carries
/// that field's archive.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_manifest_id_resolves_to_the_field_archive_of_its_own_build() {
    let mini = MiniBundle::with_shapes();
    let bridge = Bridge::start(Bundle::open(mini.dir())).await;
    let bundle = bridge.bundle().clone();

    let mut resolvable = 0;
    let mut uncarried = 0;
    for tag in [&bundle.old_tag, &bundle.new_tag] {
        let body: serde_json::Value =
            serde_json::from_slice(&bundle.recorded_build_body(tag)).expect("body");
        for m in body["data"]["manifests"].as_array().expect("[]") {
            let id = m["manifest"]["id"].as_str().expect("id");
            let (got_tag, field, build, carried) = bridge
                .manifest_resolution(id)
                .unwrap_or_else(|| panic!("id {id} is not indexed"));
            assert_eq!(got_tag, *tag, "the tag decides which archive");
            assert_eq!(field, m["matching_field"].as_str().expect("field"));
            assert_eq!(
                build,
                body["data"]["build_id"].as_str().expect("build_id"),
                "the declaring build is the one whose body declared it"
            );
            assert_eq!(
                carried,
                bundle.manifest_archive(tag, &field).is_some(),
                "carried == the archive exists"
            );
            if carried {
                resolvable += 1;
                // Serve it over loopback: real bytes, real route.
                let url = format!(
                    "{}/client_app/sophon/manifests/cxhpq4g4rgg0/{build}/{id}",
                    bridge.origins().manifest_origin
                );
                let (status, ctype, body) = get(&url).await;
                assert_eq!(status, 200, "{id}");
                assert_eq!(ctype, "application/octet-stream");
                assert!(!body.is_empty(), "{id} served no bytes");
            } else {
                uncarried += 1;
            }
        }
    }
    assert!(
        resolvable > 0,
        "the bundle must carry at least one manifest"
    );
    assert!(
        uncarried > 0,
        "a getBuild body declares the audio manifest too and the bundle stores only \
         the field it captured; if every one resolved, the --audio warning would have no teeth"
    );

    // An id no build declared is a 404, not a best-effort guess.
    let bogus = "manifest_0000000000000000_00000000000000000000000000000000";
    let url = format!(
        "{}/client_app/sophon/manifests/cxhpq4g4rgg0/{}/{bogus}",
        bridge.origins().manifest_origin,
        bundle.new_build
    );
    let (status, ctype, _) = get(&url).await;
    assert_eq!(status, 404, "an undeclared id must not be served");
    assert_eq!(
        ctype, "application/xml",
        "the measured blob 404 content-type"
    );
}

/// The invariant `docs/06` §4.1 buys: **every blob request's `<build_id>` must
/// equal the build whose manifest declared that id.** The same id under the
/// other build's prefix is a 404 — which is what makes a shim with the two blob
/// prefixes crossed impossible to mistake for a working bridge.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_blob_under_the_other_builds_prefix_is_refused() {
    let mini = MiniBundle::with_shapes();
    let bridge = Bridge::start(Bundle::open(mini.dir())).await;
    let bundle = bridge.bundle().clone();
    let o = bridge.origins().clone();

    // A manifest the new build really declares.
    let body: serde_json::Value =
        serde_json::from_slice(&bundle.recorded_build_body(&bundle.new_tag)).expect("body");
    let m = &body["data"]["manifests"].as_array().expect("[]")[0];
    let id = m["manifest"]["id"].as_str().expect("id");
    let field = m["matching_field"].as_str().expect("field");
    assert!(
        bundle.manifest_archive(&bundle.new_tag, field).is_some(),
        "the mini-bundle carries the new build's {field} manifest"
    );

    // Under its own build: served.
    let right = format!(
        "{}/client_app/sophon/manifests/cxhpq4g4rgg0/{}/{id}",
        o.manifest_origin, bundle.new_build
    );
    assert_eq!(get(&right).await.0, 200);

    // Under the other build's: 404, replayed from `blob_wrong_build`.
    let wrong = format!(
        "{}/client_app/sophon/manifests/cxhpq4g4rgg0/{}/{id}",
        o.manifest_origin, bundle.old_build
    );
    let (status, ctype, body) = get(&wrong).await;
    assert_eq!(
        status, 404,
        "the same id under the other build must be a 404"
    );
    assert_eq!(ctype, "application/xml");
    if let Some((recorded_status, recorded_ctype, _)) = bridge.recorded_shape("blob_wrong_build") {
        assert_eq!(status, recorded_status, "replayed status");
        assert_eq!(ctype, recorded_ctype, "replayed content-type");
    }
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("NoSuchKey"), "{text}");
    assert!(
        text.contains(&bundle.old_build),
        "the replayed <Key> must name the build actually requested: {text}"
    );

    // And the chunk namespace: only the latest build's prefix is ever a
    // download target, so the old build's is refused for any id at all.
    let chunk = format!(
        "{}/client_app/sophon/chunks/cxhpq4g4rgg0/{}/whatever",
        o.chunk_origin, bundle.old_build
    );
    assert_eq!(get(&chunk).await.0, 404, "old-build chunk prefix must 404");
}

/// A tag the bundle does not carry answers with the recorded `retcode -202`
/// body — **not** one the shim made up. This is the call that populates the
/// chunk-reuse map, so getting it wrong degrades to an empty reuse map, every
/// chunk downloading, and a run that still exits `0` (`docs/06` §4.3).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unserved_getbuild_tag_replays_the_recorded_not_found() {
    let mini = MiniBundle::with_one_shape();
    let bridge = Bridge::start(Bundle::open(mini.dir())).await;
    let bundle = bridge.bundle().clone();

    // Both served tags are real bodies, not error shapes.
    for tag in [&bundle.old_tag, &bundle.new_tag] {
        let (status, _, body) = get(&format!(
            "{}/getBuild?branch=main&package_id=x&password=&tag={tag}",
            bridge.origins().sophon_base
        ))
        .await;
        assert_eq!(status, 200, "{tag}");
        let v: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(v["retcode"], 0, "{tag} must be a real build body");
    }

    // A tag nobody serves: 200 + retcode -202, replayed.
    let (status, ctype, body) = get(&format!(
        "{}/getBuild?branch=main&package_id=x&password=&tag=0.0.0-not-a-real-tag",
        bridge.origins().sophon_base
    ))
    .await;
    assert_eq!(
        status, 200,
        "the JSON family answers 200 and puts it in retcode"
    );
    assert_eq!(ctype, "application/json");
    let v: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(v["retcode"], -202, "{}", body.len());
    assert!(v["data"].is_null(), "data is null on a not-found");

    if let Some((recorded_status, recorded_ctype, recorded_body)) =
        bridge.recorded_shape("getbuild_unknown_tag")
    {
        assert_eq!(status, recorded_status);
        assert_eq!(ctype, recorded_ctype);
        assert_eq!(
            String::from_utf8_lossy(&body),
            recorded_body,
            "no volatile fields are marked on this shape, so the body is verbatim"
        );
    } else {
        assert!(
            bridge.shapes_synthesized(),
            "a bundle with no error_shapes.json must say so rather than silently \
             passing the not-found assertions"
        );
    }

    // The log distinguishes the latest tag from an explicit one, because the
    // driver asserts the local tag was the one served.
    let hits = bridge.hits();
    assert!(
        hits.iter()
            .any(|h| h.kind == HitKind::Build && h.id == bundle.new_tag),
        "{hits:?}"
    );
    assert!(
        hits.iter()
            .any(|h| h.kind == HitKind::Build && h.id == "0.0.0-not-a-real-tag"),
        "{hits:?}"
    );
}

/// Blank out the fields `docs/06` §4.4 marks volatile, so two bodies can be
/// compared for everything else.
fn without_volatile(body: &str) -> String {
    let mut out = body.to_string();
    for tag in ["Key", "RequestId"] {
        if let (Some(a), Some(b)) = (
            out.find(&format!("<{tag}>")),
            out.find(&format!("</{tag}>")),
        ) && b > a
        {
            out.replace_range(a..b + tag.len() + 3, &format!("<{tag}/>"));
        }
    }
    out
}

/// The length of `<Key>`'s contents, which is the only part of a recorded
/// shape whose length legitimately changes on replay.
fn key_len(body: &str) -> usize {
    body.split("<Key>")
        .nth(1)
        .and_then(|s| s.split("</Key>").next())
        .map(str::len)
        .unwrap_or(0)
}

/// Assert one 404 against the shape the bundle recorded for it.
async fn assert_replays(bridge: &Bridge, shape_id: &str, url: &str) {
    let (status, ctype, body) = get(url).await;
    let Some((rec_status, rec_ctype, rec_body)) = bridge.recorded_shape(shape_id) else {
        panic!(
            "{shape_id}: this bundle has no api/error_shapes.json, so there is no \
             evidence to replay. That is the `shapes_synthesized` case, not this one."
        )
    };
    // `girpr` calls `.error_for_status()` and never reads the body, so status
    // and content-type are the contract and the body is fidelity.
    assert_eq!(status, rec_status, "{shape_id}: replayed status");
    assert_eq!(ctype, rec_ctype, "{shape_id}: replayed content-type");

    let replayed = String::from_utf8_lossy(&body).into_owned();
    assert_eq!(
        without_volatile(&replayed),
        without_volatile(&rec_body),
        "{shape_id}: only RequestId / Key (and the length that follows from Key) \
         may differ from the recorded body"
    );
    // The body must name the key that was actually requested, and its length
    // must follow from that substitution rather than being copied verbatim.
    let requested = url
        .split_once("://")
        .map(|(_, rest)| rest.split_once('/').map(|(_, p)| p).unwrap_or_default())
        .unwrap_or_default();
    assert!(
        replayed.contains(&format!("<Key>{requested}</Key>")),
        "{shape_id}: the replayed body must name the requested key"
    );
    assert_eq!(
        replayed.len(),
        rec_body.len() - key_len(&rec_body) + requested.len(),
        "{shape_id}: the body length must follow from the substituted Key alone"
    );
    assert!(
        !replayed.contains("6AD16DF4"),
        "{shape_id}: the recorded RequestId is marked volatile and must be synthesized"
    );
}

/// The two route families answer failure **differently**, and both answers are
/// replayed from what the bundle recorded rather than chosen here. Measured on
/// 2026-10-16 and tabulated in `docs/06` §4.4:
///
/// | family | request | answer |
/// |---|---|---|
/// | blob (`AliyunOSS`) | bogus id, unknown build, a valid id under the other build | `404` + `application/xml` `NoSuchKey` |
/// | JSON (`sg-downloader-api`) | `getBuild?tag=<unknown>` | `200` + `retcode -202` |
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn not_found_is_replayed_from_the_recorded_shapes() {
    let mini = MiniBundle::with_shapes();
    let bridge = Bridge::start(Bundle::open(mini.dir())).await;
    let bundle = bridge.bundle().clone();
    let o = bridge.origins().clone();
    assert!(
        !bridge.shapes_synthesized(),
        "this bundle carries api/error_shapes.json, so these are replayed"
    );

    let missing = "0000000000000000_00000000000000000000000000000000";
    let manifest_ns = format!(
        "{}/client_app/sophon/manifests/cxhpq4g4rgg0",
        o.manifest_origin
    );
    let chunk_ns = format!("{}/client_app/sophon/chunks/cxhpq4g4rgg0", o.chunk_origin);

    // The blob family: 404 + application/xml, for all three recorded reasons.
    assert_replays(
        &bridge,
        "blob_missing",
        &format!("{manifest_ns}/{}/{missing}", bundle.new_build),
    )
    .await;
    assert_replays(
        &bridge,
        "chunk_missing",
        &format!("{chunk_ns}/{}/{missing}", bundle.new_build),
    )
    .await;
    assert_replays(
        &bridge,
        "blob_wrong_build",
        &format!("{chunk_ns}/{}/{missing}", bundle.old_build),
    )
    .await;

    // The JSON family: 200 + retcode -202, which is the call that populates the
    // chunk-reuse map.
    let (status, ctype, body) = get(&format!(
        "{}/getBuild?branch=main&package_id=x&password=&tag=0.0.0-nope",
        o.sophon_base
    ))
    .await;
    let (rec_status, rec_ctype, rec_body) = bridge
        .recorded_shape("getbuild_unknown_tag")
        .expect("recorded");
    assert_eq!((status, ctype), (rec_status, rec_ctype));
    assert_eq!(String::from_utf8_lossy(&body), rec_body);

    // The document's other three shapes have no route here, because the bridge
    // never issues such a request: a bad `package_id` or a bad `game_ids[]` is
    // something only `gircap capture` sends. They are still checked for
    // presence and shape, so the bundle's record is read rather than assumed.
    for (id, status, retcode) in [
        ("getbuild_bad_package", 200u16, -201i64),
        ("getgameconfigs_bad_game_id", 200, -1001),
        ("getgamebranches_bad_game_id", 200, 0),
    ] {
        let (s, ct, b) = bridge.recorded_shape(id).unwrap_or_else(|| panic!("{id}"));
        assert_eq!((s, ct.as_str()), (status, "application/json"), "{id}");
        let v: serde_json::Value = serde_json::from_str(&b).expect("json body");
        assert_eq!(v["retcode"].as_i64(), Some(retcode), "{id}");
    }
}

/// A bundle that **predates** `api/error_shapes.json` has no evidence to
/// replay. The shim then answers with the measured live shape — `404` +
/// `application/xml` — and **says so**, rather than letting a not-found
/// assertion pass on an invented status. `docs/06` §4.4 makes this the honest
/// branch: inventing the statuses would make the document's claim about the API
/// unfounded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bundle_without_recorded_shapes_says_so_instead_of_inventing() {
    let mini = MiniBundle::without_shapes();
    assert!(
        !mini.dir().join("api").join("error_shapes.json").exists(),
        "the mini-bundle must not carry the file it is here to test the absence of"
    );
    let bridge = Bridge::start(Bundle::open(mini.dir())).await;
    assert!(
        bridge.shapes_synthesized(),
        "with no api/error_shapes.json the shapes are synthesized, and the bridge \
         must be able to say so"
    );
    assert!(
        bridge.recorded_shape("chunk_missing").is_none(),
        "there is nothing recorded to replay"
    );

    let b = bridge.bundle().clone();
    let o = bridge.origins().clone();
    let missing = "0000000000000000_00000000000000000000000000000000";
    for url in [
        format!(
            "{}/client_app/sophon/chunks/cxhpq4g4rgg0/{}/{missing}",
            o.chunk_origin, b.new_build
        ),
        format!(
            "{}/client_app/sophon/manifests/cxhpq4g4rgg0/{}/{missing}",
            o.manifest_origin, b.new_build
        ),
    ] {
        let (status, ctype, body) = get(&url).await;
        assert_eq!(status, 404);
        assert_eq!(
            ctype, "application/xml",
            "the measured blob 404 content-type"
        );
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("NoSuchKey"), "{text}");
        assert!(text.contains(&format!(
            "<Key>{}</Key>",
            url.split_once("://").unwrap().1.split_once('/').unwrap().1
        )));
    }
}
