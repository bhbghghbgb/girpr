//! Fetch the patch-build delete lists a repair plan can sweep.
//!
//! NOTE(starward-parity): Starward reads `delete_tags` only for the version it
//! is upgrading *from* (`DeleteTags.FirstOrDefault(x => x.Tag == localVersion)`),
//! but girpr's repair path is version-agnostic: it has no reliable local version
//! to key on, and the live patch build carries several tags. The settled policy
//! is therefore to **union all tags across all selected manifests** and let the
//! plan's live-manifest cross-check decide what is safe to delete. No version
//! keying lives here. The settled policy and its user-visible contract are
//! documented in `README.md`, "For developers / AI agents — read this first",
//! under the `SUMMARY` / `deleted_extra_bytes` machine contract.
//!
//! Nothing here deletes anything: this module only turns a live `getPatchBuild`
//! answer into a deduplicated list of clean relative paths. The product session
//! wraps the call as best-effort, so failures are returned as `Err` rather than
//! swallowed or turned into an empty success.
//!
//! See <https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GamePackageService.cs#L322-L341>

use std::collections::HashSet;

use anyhow::{Context, Result};
use girpr_proto::SophonPatchManifest;
use prost::Message;

use crate::hyp::{GameBranchPackage, HypClient};
use crate::retry::BackoffPolicy;
use crate::sophon::filter::select_patch_manifests;
use crate::sophon::stream::fetch_verified_blob;

/// The union of every path under `delete_tags -> delete_collection ->
/// delete_files` in the patch build's selected manifests.
///
/// Selection is the same rule repair uses for chunk manifests: game + selected
/// audio, minus the `res_category_dir` ignores. Selected audio is fetched even
/// though today's live audio manifests carry no delete tags — the union must
/// keep working when they start carrying some.
///
/// Paths are normalised once, here, with [`girpr_util_path::clean_rel`], so the
/// caller can compare and delete the exact spelling it was given. A path that
/// is not a plain relative path is warned about and dropped: this is a delete
/// sink, so no spelling that could escape the install root is ever handed on.
/// Duplicate paths (within a tag, across tags, or across manifests) are dropped
/// after the first sighting, preserving first-seen order.
pub async fn fetch_patch_deletes(
    hyp: &HypClient,
    backoff: &BackoffPolicy,
    pkg: &GameBranchPackage,
    audio_langs: &HashSet<String>,
    ignore: &HashSet<String>,
) -> Result<Vec<String>> {
    let build = hyp.get_patch_build(pkg).await?;
    let client = hyp.http();
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for meta in select_patch_manifests(&build, audio_langs, ignore) {
        // Only `manifest_download` is a source of blobs. `diff_download` names
        // the patch bundle, which is empty of chunk/manifest URLs in live patch
        // builds — joining the id there would 404 every time.
        let parsed = fetch_verified_blob(
            &client,
            backoff,
            meta.manifest_download.url_prefix.as_str(),
            meta.manifest.id.as_str(),
            meta.manifest.checksum.as_str(),
            meta.manifest.uncompressed_size,
            |bytes, _attempt| {
                SophonPatchManifest::decode(bytes).context("protobuf decode patch manifest")
            },
        )
        .await?;
        for tag in &parsed.delete_tags {
            let Some(collection) = &tag.delete_collection else {
                continue;
            };
            for file in &collection.delete_files {
                let path = match girpr_util_path::clean_rel(&file.file) {
                    Ok(path) => path,
                    Err(e) => {
                        // A delete sink must never follow an escape. Keep the
                        // rest of the list rather than failing the whole sweep.
                        tracing::warn!(
                            manifest = %meta.manifest.id,
                            tag = %tag.tag,
                            path = %file.file,
                            error = %e,
                            "skip a patch delete path that is not a plain relative path"
                        );
                        continue;
                    }
                };
                if seen.insert(path.clone()) {
                    out.push(path);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test fixtures panic on broken setup

    use super::*;
    use crate::hyp::GameBranchPackage;
    use crate::sophon::stream::join_url;
    use girpr_loopback_mock::{Reply, ScriptedServer};
    use girpr_model::region::Region as Biz;
    use std::sync::{Arc, Mutex};

    /// A zstd patch-manifest blob, its descriptor checksum (MD5 of the
    /// **decompressed** bytes) and the decompressed length for the size hint.
    fn blob(tags: Vec<girpr_proto::SophonPatchDeleteTag>) -> (Vec<u8>, String, usize) {
        let manifest = SophonPatchManifest {
            patches: Vec::new(),
            delete_tags: tags,
            compress_mode: 0,
        };
        let mut raw = Vec::new();
        manifest.encode(&mut raw).expect("encode patch manifest");
        (
            zstd::encode_all(raw.as_slice(), 1).expect("zstd"),
            girpr_util_hash::md5::md5_digest(&raw).to_hex(),
            raw.len(),
        )
    }

    fn tag(name: &str, files: &[&str]) -> girpr_proto::SophonPatchDeleteTag {
        girpr_proto::SophonPatchDeleteTag {
            tag: name.to_string(),
            delete_collection: Some(girpr_proto::SophonPatchDeleteFileCollection {
                delete_files: files
                    .iter()
                    .map(|file| girpr_proto::SophonPatchDeleteFile {
                        file: (*file).to_string(),
                        size: 0,
                        md5: String::new(),
                    })
                    .collect(),
            }),
        }
    }

    /// A descriptor with string sizes, as the live `getPatchBuild` body has.
    /// `diff_prefix` is a trap: the fetch must use `manifest_download` instead.
    fn descriptor(
        field: &str,
        id: &str,
        checksum: &str,
        raw_len: usize,
        prefix: &str,
        diff_prefix: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "category_id": field,
            "category_name": field,
            "matching_field": field,
            "manifest": {
                "id": id,
                "checksum": checksum,
                "compressed_size": "123",
                "uncompressed_size": raw_len.to_string(),
            },
            "diff_download": { "url_prefix": diff_prefix, "url_suffix": "" },
            "manifest_download": { "url_prefix": prefix, "url_suffix": "" },
        })
    }

    fn json_reply(manifests: serde_json::Value) -> String {
        serde_json::json!({
            "retcode": 0,
            "message": "OK",
            "data": {
                "build_id": "b",
                "patch_id": "p",
                "tag": "7.2.0",
                "manifests": manifests,
            },
        })
        .to_string()
    }

    fn server(replies: Vec<Reply>) -> ScriptedServer {
        ScriptedServer::start(replies)
    }

    fn build_server(manifests: serde_json::Value) -> ScriptedServer {
        server(vec![Reply::new(200, json_reply(manifests))])
    }

    fn client_for(base: &str) -> HypClient {
        HypClient::new_with_bases(
            Biz::Hk4eGlobal,
            base.to_string(),
            base.to_string(),
            BackoffPolicy::none(),
        )
        .expect("loopback client")
    }

    fn pkg() -> GameBranchPackage {
        GameBranchPackage {
            package_id: "pkg".into(),
            branch: "main".into(),
            password: "pw".into(),
            tag: "7.2.0".into(),
            diff_tags: vec!["7.1.0".into(), "7.0.0".into()],
        }
    }

    /// A WARN-only subscriber. Capturing logs needs no new dependency: the
    /// delete-sink warning is the only line this test cares about.
    #[derive(Clone, Default)]
    struct WarnCapture(Arc<Mutex<Vec<String>>>);

    impl tracing::Subscriber for WarnCapture {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            metadata.level() <= &tracing::Level::WARN
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::Id {
            tracing::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::Id, _: &tracing::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut fields = Fields(Vec::new());
            event.record(&mut fields);
            if event.metadata().level() == &tracing::Level::WARN {
                self.0.lock().expect("lock").push(fields.0.join(" "));
            }
        }
        fn enter(&self, _: &tracing::Id) {}
        fn exit(&self, _: &tracing::Id) {}
    }

    struct Fields(Vec<String>);
    impl tracing::field::Visit for Fields {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.push(format!("{}={value:?}", field.name()));
        }
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.push(format!("{}={value}", field.name()));
        }
    }

    /// Two selected manifests, two tags in the first, one in the second:
    /// union-all wins over version keying, duplicates are dropped in first-seen
    /// order, `./a//b` normalises to `a/b`, and an escaping path is warned and
    /// dropped. The journals also pin that only `manifest_download` is used:
    /// the `diff_download` trap server sees no request at all.
    #[tokio::test]
    async fn patch_deletes_union_all_tags_and_manifests_in_first_seen_order() {
        let (game, game_md5, game_len) = blob(vec![
            tag("7.1.0", &["./a//b", "old/a.dat", "shared.dat"]),
            tag("7.0.0", &["shared.dat", "old/b.dat"]),
        ]);
        let (audio, audio_md5, audio_len) = blob(vec![tag(
            "7.1.0",
            &["en-us/old.dat", "../escape.dat", "a//b"],
        )]);
        // The blob server is started first so its URI can go into the build
        // reply. Its script walks the two blobs in selection order.
        let blobs = server(vec![Reply::new(200, game), Reply::new(200, audio)]);
        let diffs = server(vec![Reply::new(500, "diff bundle trap")]);
        let prefix = format!("{}/manifests", blobs.uri());
        let diff_prefix = format!("{}/diffs", diffs.uri());
        // The uppercase checksum pins the case-insensitive MD5 match.
        let build = build_server(serde_json::json!([
            descriptor(
                "game",
                "m_game",
                &game_md5.to_uppercase(),
                game_len,
                &prefix,
                &diff_prefix
            ),
            descriptor(
                "en-us",
                "m_en",
                &audio_md5,
                audio_len,
                &prefix,
                &diff_prefix
            ),
            // Unselected audio, the ignored res field and the audio heuristic
            // field must never be fetched.
            descriptor("ja-jp", "m_ja", "0", 0, &prefix, &diff_prefix),
            descriptor("mini-zh-cn", "m_mini", "0", 0, &prefix, &diff_prefix),
            descriptor("10302", "m_res", "0", 0, &prefix, &diff_prefix),
        ]));

        let warnings = WarnCapture::default();
        let guard = tracing::subscriber::set_default(warnings.clone());
        let got = fetch_patch_deletes(
            &client_for(&build.uri()),
            &BackoffPolicy::none(),
            &pkg(),
            &["en-us".to_string()].into_iter().collect(),
            &["10302".to_string()].into_iter().collect(),
        )
        .await
        .expect("the build and both manifests are served");
        drop(guard);

        assert_eq!(
            got,
            vec![
                "a/b",
                "old/a.dat",
                "shared.dat",
                "old/b.dat",
                "en-us/old.dat"
            ],
            "all tags and selected manifests, deduplicated and cleaned"
        );
        let lines = warnings.0.lock().expect("lock");
        assert!(
            lines
                .iter()
                .any(|line| line.contains("not a plain relative path")
                    && line.contains("../escape.dat")),
            "an escaping delete path must be warned about: {lines:?}"
        );
        drop(lines);

        assert_eq!(build.requests().len(), 1, "one getPatchBuild POST");
        assert_eq!(build.requests()[0].method, "POST");
        assert!(build.requests()[0].target.starts_with("/getPatchBuild?"));
        let requests = blobs.requests();
        assert_eq!(requests.len(), 2, "only the two selected blobs are fetched");
        assert_eq!(requests[0].target, "/manifests/m_game");
        assert_eq!(requests[1].target, "/manifests/m_en");
        assert_eq!(
            diffs.requests().len(),
            0,
            "diff_download is not a blob source"
        );
    }

    /// A wrong descriptor checksum is retried like any other manifest blob and
    /// finally reported, rather than yielding a partial delete list.
    #[tokio::test]
    async fn wrong_patch_checksum_is_retried_and_refused() {
        let (body, _good, len) = blob(vec![tag("7.1.0", &["old.dat"])]);
        let blobs = server(vec![Reply::new(200, body)]);
        let build = build_server(serde_json::json!([descriptor(
            "game",
            "m_game",
            "00000000000000000000000000000000",
            len,
            &format!("{}/m", blobs.uri()),
            ""
        ),]));
        let err = fetch_patch_deletes(
            &client_for(&build.uri()),
            &BackoffPolicy::none(),
            &pkg(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .await
        .expect_err("a wrong checksum must never yield deletes");
        assert_eq!(build.requests().len(), 1, "the build is fetched once");
        assert_eq!(blobs.requests().len(), 5, "the bad blob gets five tries");
        let chain = format!("{err:#}");
        assert!(chain.contains("checksum mismatch"), "{chain}");
        assert!(chain.contains("after 5 attempts"), "{chain}");
    }

    /// Bad zstd and bad protobuf are errors, not panics. Each descriptor's
    /// checksum is of the bytes actually served, so the failure is in the
    /// decode rather than in the digest gate.
    #[tokio::test]
    async fn bad_zstd_and_bad_protobuf_are_errors_not_panics() {
        let bad_frame = b"not a zstd frame".to_vec();
        let bad_message = b"\x0f".to_vec(); // invalid wire type 7
        let framed_message = zstd::encode_all(bad_message.as_slice(), 1).expect("zstd");
        // The served body, the bytes the descriptor checksums, and the cause
        // each case must report.
        for (body, raw, cause) in [
            (bad_frame.clone(), bad_frame, "zstd"),
            (
                framed_message,
                bad_message,
                "protobuf decode patch manifest",
            ),
        ] {
            let checksum = girpr_util_hash::md5::md5_digest(&raw).to_hex();
            let blobs = server(vec![Reply::new(200, body.clone())]);
            let build = build_server(serde_json::json!([descriptor(
                "game",
                "m_game",
                &checksum,
                raw.len(),
                &format!("{}/m", blobs.uri()),
                ""
            ),]));
            let err = fetch_patch_deletes(
                &client_for(&build.uri()),
                &BackoffPolicy::none(),
                &pkg(),
                &HashSet::new(),
                &HashSet::new(),
            )
            .await
            .expect_err("a bad blob cannot be decoded");
            let chain = format!("{err:#}");
            assert!(
                chain.contains(cause),
                "the decode is the reported cause: {chain}"
            );
            assert!(chain.contains("after 5 attempts"), "{chain}");
            assert_eq!(blobs.requests().len(), 5);
        }
    }

    /// No manifests, or no delete tags, is an empty success: the caller need
    /// not special-case a build that has nothing to sweep.
    #[tokio::test]
    async fn empty_build_and_empty_delete_tags_are_empty_successes() {
        let build = build_server(serde_json::json!([]));
        let empty = fetch_patch_deletes(
            &client_for(&build.uri()),
            &BackoffPolicy::none(),
            &pkg(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .await
        .expect("an empty build is a success");
        assert!(empty.is_empty());
        assert_eq!(build.requests().len(), 1, "no blob was requested");

        let (body, checksum, len) = blob(Vec::new());
        let blobs = server(vec![Reply::new(200, body)]);
        let build = build_server(serde_json::json!([descriptor(
            "game",
            "m_game",
            &checksum,
            len,
            &format!("{}/m", blobs.uri()),
            ""
        ),]));
        let empty = fetch_patch_deletes(
            &client_for(&build.uri()),
            &BackoffPolicy::none(),
            &pkg(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .await
        .expect("no delete tags is a success");
        assert!(empty.is_empty());
        assert_eq!(blobs.requests().len(), 1);
    }

    /// The pure URL policy: a production-shaped prefix keeps its build segment
    /// and the patch manifest id is appended below it — verified against the
    /// same join the fetch uses.
    #[test]
    fn patch_manifest_url_uses_prefix_and_id() {
        let prefix =
            "https://autopatchhk.yuanshen.com/client_app/sophon/patch/cxhpq4g4rgg0/sMXGW2ll3Fuu";
        let id = "manifest_671e1a92a6cf53ff_8d4dfb34d2ee2cf64aae45a9b1ecf58d";
        assert_eq!(
            join_url(prefix, id).expect("the prefix resolves").as_str(),
            format!("{prefix}/{id}")
        );
    }
}
