//! The persistent producer, end to end against a loopback transport.
//!
//! This drives the real `persistent::capture::capture` with a test-only
//! dispatcher seam: the envelope and key are the synthetic, test-only fixture
//! material from `tests/fixtures/persistent_inputs/`, and every descriptor URL
//! is rewritten to an in-memory origin. The test proves that the producer
//! queries the dispatcher, verifies and parses all three manifests, admits the
//! cheapest rows, fetches them and stages records that the existing emit
//! helpers inventory. No production key or endpoint is touched.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gircap::http::{
    Fetcher, HttpRequest, HttpResponse, HttpTransport, Kind, RequestLog, SendFuture, Transport,
    ZeroSleep,
};
use gircap::persistent::DispatchRecord;
use gircap::persistent::capture::{CaptureInput, DispatchContext, DispatchSource};
use gircap::select::{Budget, MatrixStatus};
use girpr_dispatch::{
    DispatchDecodeParams, ManifestKind, channel_prefix, decode_dispatch_response, dispatch_host,
    dispatch_info_from_decoded, private_key_from_bytes,
};
use girpr_model::Region;

#[derive(Debug)]
struct MapTransport {
    bodies: HashMap<String, Vec<u8>>,
    seen: Arc<Mutex<Vec<(String, Kind)>>>,
}

type Mutex<T> = std::sync::Mutex<T>;

impl HttpTransport for MapTransport {
    fn send(&self, request: HttpRequest) -> SendFuture {
        let body = self.bodies.get(&request.url).cloned();
        let seen = Arc::clone(&self.seen);
        Box::pin(async move {
            let Some(body) = body else {
                return Ok(HttpResponse::in_memory(
                    404,
                    request.url,
                    Some("text/plain"),
                    vec![b"no route".to_vec()],
                ));
            };
            let kind = if request.url.contains("/manifests/") {
                Kind::Manifest
            } else {
                Kind::Api
            };
            seen.lock()
                .expect("the journal is not poisoned")
                .push((request.url.clone(), kind));
            Ok(HttpResponse::in_memory(
                200,
                request.url,
                Some("application/octet-stream"),
                vec![body],
            ))
        })
    }
}

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/persistent_inputs/samples/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn context(base: &str) -> DispatchContext {
    let envelope = fixture("dispatch_response.json");
    let key_material = fixture("test_key.pem");
    let key = private_key_from_bytes(&key_material).expect("the synthetic key parses");
    let biz = Region::Hk4eGlobal;
    let decoded = decode_dispatch_response(
        &envelope,
        DispatchDecodeParams {
            biz,
            latest: "9.9.0",
            host: dispatch_host(biz),
            channel_prefix: channel_prefix(biz),
            exe_stem: "GenshinImpact",
            key: &key,
            block_bytes: 256,
        },
    )
    .expect("the synthetic envelope decodes");
    let descriptor_remote_names = decoded.descriptor_remote_names.clone();
    let repo_res_version_url = decoded.repo_res_version_url.clone();
    let repo_design_data_url = decoded.repo_design_data_url.clone();
    let mut info = dispatch_info_from_decoded(decoded, "99000000 001122334455")
        .expect("the synthetic base_revision parses");
    // Replay the recorded facts against this test's origin only: digests,
    // revisions and CDN paths are untouched.
    for d in info.manifest_digests.iter_mut() {
        d.url = format!("{base}/manifests/{}", d.kind.as_str());
    }
    info.client_game_res_url = format!("{base}/game_res");
    info.client_design_data_url = format!("{base}/design_data");
    info.client_silence_data_url = format!("{base}/silence_data");
    info.client_audio_assets_url = format!("{base}/audio_assets");
    DispatchContext {
        info,
        envelope,
        key_material,
        seed: "0011223344556677".into(),
        dispatch_url: "https://dispatcher.invalid/query_cur_region?version=OSRELWin9.9.0".into(),
        query_version: "OSRELWin9.9.0".into(),
        repo_res_version_url,
        repo_design_data_url,
        descriptor_remote_names,
        base_revision: "99000000 001122334455".into(),
    }
}

fn bodies_for(base: &str, ctx: &DispatchContext) -> HashMap<String, Vec<u8>> {
    let mut bodies = HashMap::new();
    for (kind, file) in [
        (ManifestKind::Res, "res.ndjson"),
        (ManifestKind::Data, "data.ndjson"),
        (ManifestKind::Silence, "silence.ndjson"),
    ] {
        let url = ctx
            .info
            .manifest_digests
            .iter()
            .find(|d| d.kind == kind)
            .map(|d| d.url.clone())
            .expect("a descriptor");
        bodies.insert(url, fixture(file));
    }
    let a = fixture("hotfix_a.blk");
    let b = fixture("hotfix_b.pck");
    let by_md5 = |md5: &str| -> Vec<u8> {
        if gircap::hash::md5_hex(&a) == md5 {
            a.clone()
        } else if gircap::hash::md5_hex(&b) == md5 {
            b.clone()
        } else {
            panic!("no fixture bytes for {md5}")
        }
    };
    // Every row the three tiny manifests publish is fetchable, so the extras
    // phase can be exercised too.
    for (kind, file) in [
        (ManifestKind::Res, "res.ndjson"),
        (ManifestKind::Data, "data.ndjson"),
        (ManifestKind::Silence, "silence.ndjson"),
    ] {
        let rows = girpr_persistent_plan::parse_rows(&fixture(file)).expect("rows");
        for row in rows {
            let (url, _) = girpr_persistent_plan::asset_urls(
                &ctx.info,
                kind,
                girpr_persistent_plan::folder_for(&row.remote_name),
                &row.remote_name,
                false,
            )
            .expect("a blob URL");
            bodies.insert(url, by_md5(&row.md5));
        }
    }
    bodies.insert(format!("{base}/unused"), b"never fetched".to_vec());
    bodies
}

#[tokio::test]
async fn the_producer_queries_the_dispatcher_and_writes_records() {
    let base = "http://127.0.0.1:50001";
    let ctx = context(base);
    let bodies = bodies_for(base, &ctx);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = RequestLog::new();
    let fetcher = Fetcher::with_http(
        Transport::local(),
        log.clone(),
        Arc::new(ZeroSleep),
        Arc::new(MapTransport {
            bodies,
            seen: Arc::clone(&seen),
        }),
    )
    .expect("fetcher");

    let calls = Arc::new(AtomicUsize::new(0));
    let supplied = Arc::new(ctx.clone());
    let query_calls = Arc::clone(&calls);
    let query_ctx = Arc::clone(&supplied);
    let source = DispatchSource::Test(Arc::new(move |_biz, latest| {
        query_calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(latest, "9.9.0");
        Ok((*query_ctx).clone())
    }));

    let outcome = gircap::persistent::capture::capture(CaptureInput {
        fetcher: &fetcher,
        biz: Region::Hk4eGlobal,
        latest: "9.9.0",
        budget: Budget::unlimited(),
        source,
    })
    .await;

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the producer asked the dispatcher"
    );
    assert_eq!(outcome.selection.matrix.len(), 3);
    for row in &outcome.selection.matrix {
        assert_eq!(row.status, MatrixStatus::Captured, "{row:?}");
        assert!(row.capture_id.is_some(), "{row:?}");
        assert!(row.path.is_some(), "{row:?}");
        assert!(row.candidates_considered > 0, "{row:?}");
    }
    assert_eq!(
        outcome.selection.bytes_admitted,
        outcome.selection.budget.spent(),
        "one ledger"
    );
    // res: 48 + 64; data: 48 + 48; silence: 64 — five blobs, all five rows
    // from the tiny fixtures, plus the envelope and three manifests.
    assert_eq!(outcome.selection.admitted.len(), 5);
    assert_eq!(
        outcome.selection.admitted[0].remote_name,
        "English(US)/VO_TEST.pck"
    );
    assert!(outcome.reason.is_none());

    let records = outcome.records.expect("records were staged");
    assert_eq!(records.record.schema_version, 4);
    assert_eq!(
        records.record.ciphertext_bytes,
        records.envelope.len() as u64
    );
    assert_eq!(
        records.record.decoded.descriptors.len(),
        3,
        "one aggregate record, three descriptors"
    );
    let kinds: Vec<&str> = records
        .record
        .decoded
        .descriptors
        .iter()
        .map(|d| d.kind.as_str())
        .collect();
    assert_eq!(kinds, ["res", "data", "silence_data"]);
    assert_eq!(records.record.key_file, "dispatch/key.pem");
    assert!(
        records
            .record
            .decoded
            .repo_res_version_url
            .contains("/client_game_res/"),
        "the CDN base, not a derived output URL"
    );
    assert!(
        records
            .index
            .blobs
            .iter()
            .all(|b| !b.source_kind.is_empty())
    );
    assert!(records.index.blobs.iter().all(|b| !b.capture_id.is_empty()));

    // The existing emit helpers inventory everything written here.
    let dir = girpr_tempdir::TempDir::new("gircap-persistent", "records");
    let mut written = gircap::emit::write_dispatch(&dir, &records.record, &records.envelope)
        .expect("dispatch record");
    written.push(
        gircap::emit::write_rel(&dir, "dispatch/key.pem", &records.key_material)
            .expect("key material"),
    );
    written.extend(
        gircap::emit::write_persistent(&dir, &records.index, &records.bodies)
            .expect("persistent records"),
    );
    let listed: Vec<String> = gircap::emit::write_manifest_files(&dir)
        .expect("scan")
        .into_iter()
        .map(|f| f.path)
        .collect();
    for rel in &written {
        assert!(listed.contains(rel), "{rel} not in MANIFEST.json files[]");
    }
    let (_, checksums) = gircap::emit::write_checksums(&dir).expect("checksums");
    let checked: Vec<&str> = checksums.entries.iter().map(|e| e.path.as_str()).collect();
    for rel in &written {
        assert!(checked.contains(&rel.as_str()), "{rel} not checksummed");
    }

    // The envelope and key bytes survive round-trip through the typed record
    // shape the repair side parses.
    let on_disk: DispatchRecord = serde_json::from_slice(
        &std::fs::read(dir.join("dispatch/record.json")).expect("record on disk"),
    )
    .expect("the record parses");
    assert_eq!(on_disk, records.record);
    assert_eq!(
        std::fs::read(dir.join("dispatch/response.json")).expect("envelope on disk"),
        records.envelope
    );

    let rows = seen.lock().expect("journal");
    assert!(
        rows.iter().any(|(_, kind)| *kind == Kind::Manifest),
        "the three manifests were fetched through the fetcher"
    );
    assert!(
        rows.iter().any(|(_, kind)| *kind == Kind::Api),
        "the full-file blobs were fetched as api-kind GETs"
    );
    assert_eq!(log.len(), rows.len(), "every GET is recorded once");
}
