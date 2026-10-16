//! The `gircap`→`girpr` end-to-end artifact bridge.
//!
//! Spec: [`docs/06-artifact-bridge-spec.md`](../docs/06-artifact-bridge-spec.md).
//! Plan: [`docs/07-artifact-bridge-implementation-plan.md`](../docs/07-artifact-bridge-implementation-plan.md).
//!
//! `gircap capture` writes a self-contained bundle — a real `game/` fixture,
//! the API bodies and manifests behind it, every chunk blob, `expected/` ground
//! truth and a `patchmaps/<id>.json` per test-matrix variant. Three consumption
//! paths work (inspect the metadata, rebuild old/new offline, prove the
//! bundle's own integrity). **This file is the fourth**: run the real `girpr`
//! binary against the bundle's bytes, over loopback, and check it patches to
//! the new version.
//!
//! **Gate 1 — every test here is `#[ignore]`d.** Never runs under a bare
//! `cargo test`; still *compiles*, which is what keeps it from rotting against
//! `RunCtx`, `PathScope` and the `SUMMARY` contract. Invoke as:
//!
//! ```text
//! cargo test -p girpr --test artifact_bridge -- --ignored --nocapture
//! ```
//!
//! **Gate 2 — no bundle means skip, never fail.** `cargo test -- --ignored` is
//! a thing people type, and it must not turn red because nobody has captured
//! anything. Resolution order is `GIRPR_ARTIFACT_BUNDLE`, then a bundle
//! discovered under `artifacts/`, then a printed reason plus the command that
//! would produce one. The skip message is part of the contract: a bridge that
//! fails on a missing input teaches people to run it with `--ignored` forever
//! and stop reading.
//!
//! **What this file is allowed to touch.** The bundle, read-only. The working
//! tree is a scratch *copy* of the bundle's `game/` — pointing `--game-path`
//! at the bundle in place is what turns a cleanup flag into a way to destroy
//! the evidence, and the bundle has to still pass `gircap verify` afterwards.

// The server's surface is wider than M1's half of it exercises — the request
// log, the per-capture lookups and the `--purge-only` predicate are read by the
// driver milestones (M2/M3), so the allowance is about the milestone split and
// not about code nothing reaches.
#[allow(dead_code)]
#[path = "artifact_bridge/server.rs"]
mod server;

use std::path::{Path, PathBuf};

use server::{Bridge, Bundle};

// ---------------------------------------------------------------------------
// gate 2: locating a bundle
// ---------------------------------------------------------------------------

/// The `artifacts/` root beside the package manifest, or
/// `GIRPR_ARTIFACT_ROOT` when one is set.
fn artifacts_root() -> PathBuf {
    match std::env::var_os("GIRPR_ARTIFACT_ROOT") {
        Some(v) => PathBuf::from(v),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts"),
    }
}

/// Bundles are laid out `<root>/<biz>/<old>__<new>/<capture-id>/`, so a bundle
/// is any directory holding a `MANIFEST.json`.
fn scan_for_bundles(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(biz) = std::fs::read_dir(root) else {
        return out;
    };
    for b in biz.filter_map(Result::ok) {
        let Ok(pair) = std::fs::read_dir(b.path()) else {
            continue;
        };
        for p in pair.filter_map(Result::ok) {
            let Ok(id) = std::fs::read_dir(p.path()) else {
                continue;
            };
            for c in id.filter_map(Result::ok) {
                if c.path().join("MANIFEST.json").is_file() {
                    out.push(c.path());
                }
            }
        }
    }
    out.sort();
    out
}

/// Resolve the bundle under test, or `None` with the reason and the exact
/// command that would produce one.
fn locate_bundle() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("GIRPR_ARTIFACT_BUNDLE") {
        let p = PathBuf::from(explicit);
        assert!(
            p.join("MANIFEST.json").is_file(),
            "GIRPR_ARTIFACT_BUNDLE={} is not a bundle (no MANIFEST.json)",
            p.display()
        );
        return Some(p);
    }
    let root = artifacts_root();
    let found = scan_for_bundles(&root);
    match found.len() {
        0 => {
            println!(
                "SKIP artifact_bridge: no bundle under {}\n\
                 \x20     produce one with:  cargo run -p gircap -- capture --out {}\n\
                 \x20     or point at an existing one:  set GIRPR_ARTIFACT_BUNDLE=<dir>",
                root.display(),
                root.display()
            );
            None
        }
        1 => Some(found.into_iter().next().expect("len 1")),
        n => {
            let list: Vec<String> = found.iter().map(|p| format!("  {}", p.display())).collect();
            println!(
                "SKIP artifact_bridge: {n} bundles under {}; name one with GIRPR_ARTIFACT_BUNDLE\n{}",
                root.display(),
                list.join("\n")
            );
            None
        }
    }
}

/// The bundle under test, or a skip. Gate 2.
fn bundle_or_skip() -> Option<Bundle> {
    locate_bundle().map(Bundle::open)
}

/// Start the four listeners over `bundle`.
async fn bridge_or_skip() -> Option<Bridge> {
    let bundle = bundle_or_skip()?;
    Some(Bridge::start(bundle).await)
}

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

// ---------------------------------------------------------------------------
// M1 — the server
// ---------------------------------------------------------------------------

/// The rewrite must be surgical. `sophon::fetch_manifest` MD5-verifies the
/// **decompressed** manifest against `manifest.checksum`, so a checksum that
/// moved kills the run at metadata with exit `2` — a failure that looks like a
/// product bug and is not one. `manifest.id` is the URL path segment.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a gircap bundle: cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
async fn the_rewrite_changes_only_the_two_url_prefixes() {
    let Some(bridge) = bridge_or_skip().await else {
        return;
    };
    let bundle = bridge.bundle();

    for tag in [&bundle.old_tag, &bundle.new_tag] {
        let recorded: serde_json::Value =
            serde_json::from_slice(&bundle.recorded_build_body(tag)).expect("recorded body");
        let served: serde_json::Value =
            serde_json::from_slice(&bridge.served_build(tag).expect("tag is served"))
                .expect("served body");

        assert_eq!(
            recorded["retcode"], served["retcode"],
            "{tag}: the envelope must be untouched"
        );
        assert_eq!(recorded["message"], served["message"], "{tag}");
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
#[ignore = "needs a gircap bundle: cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
async fn each_namespace_keeps_its_path_and_gets_its_own_origin() {
    let Some(bridge) = bridge_or_skip().await else {
        return;
    };
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
#[ignore = "needs a gircap bundle: cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
async fn a_manifest_id_resolves_to_the_field_archive_of_its_own_build() {
    let Some(bridge) = bridge_or_skip().await else {
        return;
    };
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
        "a getBuild body declares five manifests and the bundle stores the fields it \
         captured; if every one resolved, docs/06 4.5's --audio warning has no teeth"
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
#[ignore = "needs a gircap bundle: cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
async fn a_blob_under_the_other_builds_prefix_is_refused() {
    let Some(bridge) = bridge_or_skip().await else {
        return;
    };
    let bundle = bridge.bundle().clone();
    let o = bridge.origins().clone();

    // A manifest the new build really declares.
    let body: serde_json::Value =
        serde_json::from_slice(&bundle.recorded_build_body(&bundle.new_tag)).expect("body");
    let m = &body["data"]["manifests"].as_array().expect("[]")[0];
    let id = m["manifest"]["id"].as_str().expect("id");
    let field = m["matching_field"].as_str().expect("field");
    if bundle.manifest_archive(&bundle.new_tag, field).is_none() {
        println!("SKIP: the bundle carries no {field} manifest for the new build");
        return;
    }

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
#[ignore = "needs a gircap bundle: cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
async fn an_unserved_getbuild_tag_replays_the_recorded_not_found() {
    let Some(bridge) = bridge_or_skip().await else {
        return;
    };
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
            .any(|h| h.kind == server::HitKind::Build && h.id == bundle.new_tag),
        "{hits:?}"
    );
    assert!(
        hits.iter()
            .any(|h| h.kind == server::HitKind::Build && h.id == "0.0.0-not-a-real-tag"),
        "{hits:?}"
    );
}

/// The capture matrix loaded off disk. `docs/06` assertion 5 lives here too:
/// a repeated path would make the second run silently skip and report
/// `download_bytes=0` against a non-zero expectation, which is a baffling
/// failure to diagnose from the number that is wrong.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a gircap bundle: cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
async fn the_capture_matrix_loads_with_distinct_paths() {
    let Some(bridge) = bridge_or_skip().await else {
        return;
    };
    let b = bridge.bundle();

    assert!(
        !b.biz.is_empty(),
        "--biz comes from MANIFEST.json -> biz.id"
    );
    assert_ne!(b.old_build, b.new_build, "two builds, two namespaces");
    assert!(b.captures.len() >= 2, "the matrix is the point");

    let mut paths: Vec<&str> = b.captures.iter().map(|c| c.path.as_str()).collect();
    let before = paths.len();
    paths.sort_unstable();
    paths.dedup();
    assert_eq!(
        paths.len(),
        before,
        "capture paths must be pairwise distinct"
    );

    for c in &b.captures {
        assert!(!c.classes.is_empty(), "{}: classes[]", c.id);
        // `added` is the one class whose path is *absent* from `game/` by
        // construction — it is the file the new version introduces — so the
        // fixture must not carry it, or the run would find nothing to plan.
        let on_disk = b.root.join("game").join(&c.path).is_file();
        assert_eq!(
            on_disk,
            !c.classes.iter().any(|k| k == "added"),
            "{}: game/ presence must follow the class",
            c.id
        );
        match c.download_bytes {
            // `removed` carries `totals: null` and is driven by the purge
            // counters instead (`docs/06` §5.1).
            None => {
                assert!(c.absent_after, "{}: no byte total implies a deletion", c.id);
                assert!(
                    c.must_not_fetch.is_empty(),
                    "{}: nothing is downloaded",
                    c.id
                );
                assert!(
                    c.old_size > 0,
                    "{}: purge_bytes comes from the old size",
                    c.id
                );
            }
            Some(n) => {
                assert!(!c.absent_after, "{}: repairs to a file, not away", c.id);
                assert!(!c.result_md5.is_empty(), "{}: ground truth", c.id);
                assert!(
                    b.root.join(&c.result_path).is_file(),
                    "{}: expected/ ground truth",
                    c.id
                );
                assert!(c.new_size > 0, "{}: new size", c.id);
                if n == 0 {
                    assert!(
                        !c.must_not_fetch.is_empty(),
                        "{}: a zero-byte run is only provable by what was NOT fetched",
                        c.id
                    );
                }
            }
        }
    }
    let _ = b.capture_id;
}
