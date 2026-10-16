//! Writing the bundle.
//!
//! **M1** landed [`write_requests_log`]: the one file whose whole job is
//! provenance. **M2** adds [`write_api`] — the six byte-exact response bodies
//! plus the generated tag probe. **M8** adds the rest of the tree — `game/`,
//! `expected/`, `MANIFEST.json`, `checksums.json`, `README.md` — in the
//! submodules below.
//!
//! Everything temporary goes to `tmp/` and is moved into place, so an interrupted
//! run never leaves a half-written document that reads as a complete one.
//! `verify/checksums.json` is what proves nothing is missing.

pub mod gamedir;
pub mod index;
pub mod selfcheck;

use std::path::{Path, PathBuf};

use crate::config::rel_path_string;
use crate::http::RequestLog;
use crate::http::probe::{ErrorShapes, TagProbe};
use crate::hyp::{self, Captured};
use crate::manifest;

/// Write `api/requests.ndjson` inside the bundle at `root`.
///
/// Returns how many requests were recorded, which is zero for an empty log — a
/// file that exists but is empty is the honest shape for a run that made no
/// requests, and it is distinguishable from a run that never got far enough to
/// write one.
pub fn write_requests_log(root: &Path, log: &RequestLog) -> std::io::Result<usize> {
    log.write_ndjson(&root.join("api").join("requests.ndjson"))
}

/// Write every `api/` document and return the artifact-relative paths written.
///
/// Returns relative, `/`-separated paths in bundle order, which is also what
/// goes into `MANIFEST.json → files[]`. Computing the list here means the index
/// and the tree cannot disagree about what exists — which is the only reason
/// `error_shapes.json` needs to be pushed here at all: returning it is what gets
/// it into `files[]` and therefore into `verify/checksums.json`, with no further
/// wiring.
///
/// The six response bodies are written **exactly as served** — not re-serialized
/// from a typed view — because a re-serialization would silently drop or restyle
/// a field, and the whole point of capturing the body is that a reader can `diff`
/// it against a fresh request. `tag_probe.json` and `error_shapes.json` are the
/// two generated documents: they are this tool's own measurements, not the API's
/// words.
pub fn write_api(
    root: &Path,
    captured: &Captured,
    probe: &TagProbe,
    shapes: &ErrorShapes,
) -> std::io::Result<Vec<String>> {
    let mut written = Vec::with_capacity(8);
    for body in captured.bodies() {
        write_bytes(root, &format!("api/{}", body.file), &body.bytes)?;
        written.push(format!("api/{}", body.file));
    }
    let probe_json = hyp::render_json(probe).map_err(std::io::Error::other)?;
    write_newline_doc(root, "api/tag_probe.json", probe_json.as_bytes())?;
    written.push("api/tag_probe.json".to_string());
    let shapes_json = hyp::render_json(shapes).map_err(std::io::Error::other)?;
    write_newline_doc(root, "api/error_shapes.json", shapes_json.as_bytes())?;
    written.push("api/error_shapes.json".to_string());
    Ok(written)
}

/// Write `diff/paths.diff.json`, `diff/paths.tsv` and `diff/reuse_pool.json`.
///
/// Returns the paths written, in bundle order. `reuse_pool.json` is the first
/// pool when more than one field was scored — it is the *game* pool that
/// measures what same-file-only reuse costs, and an audio pool's numbers would
/// answer a different question.
pub fn write_diff(
    root: &Path,
    diff: &manifest::diff::PathDiff,
    pool: &manifest::diff::ReusePool,
    diffs: &[manifest::diff::FieldDiff],
) -> std::io::Result<Vec<String>> {
    let mut out = Vec::with_capacity(3);
    out.push(write_doc(root, "diff/paths.diff.json", diff)?);
    out.push(write_doc(root, "diff/reuse_pool.json", pool)?);
    let tsv = manifest::diff::paths_tsv(diffs);
    write_bytes(root, "diff/paths.tsv", tsv.as_bytes())?;
    out.push("diff/paths.tsv".to_string());
    Ok(out)
}

/// Write `MANIFEST.json` and return the artifact-relative path written.
pub fn write_manifest(root: &Path, m: &index::Manifest) -> std::io::Result<String> {
    write_doc(root, "MANIFEST.json", m)
}

/// Write `README.md`, rendered from `MANIFEST.json` and the selfcheck.
pub fn write_readme(
    root: &Path,
    m: &index::Manifest,
    s: &selfcheck::Selfcheck,
) -> std::io::Result<String> {
    write_bytes(root, "README.md", m.readme(s).as_bytes())?;
    Ok("README.md".to_string())
}

/// Write `verify/selfcheck.json`.
pub fn write_selfcheck(root: &Path, s: &selfcheck::Selfcheck) -> std::io::Result<String> {
    write_doc(root, "verify/selfcheck.json", s)
}

/// Digest every file under `root`, returning both the list for
/// `MANIFEST.json → files[]` and the same entries as the seed for
/// `verify/checksums.json`.
///
/// Two consumers, one read of each file. The seed exists so the close-out's
/// later `checksums.json` can carry the digests it already holds instead of
/// re-hashing a whole bundle to re-list a tree that has not changed — see
/// [`write_checksums_reusing`].
///
/// This replaces a private `run.rs` helper that walked the tree a second time
/// and inlined `encoding_of`, so a third encoding would have taught
/// `MANIFEST.json` the wrong rule while `checksums.json` learned the right one.
pub fn write_manifest_files(
    root: &Path,
) -> std::io::Result<(Vec<index::FileEntry>, Vec<index::FileEntry>)> {
    let c = index::Checksums::scan(root)?;
    Ok((c.entries.clone(), c.entries))
}

/// Digest everything under `root` and write `verify/checksums.json`.
///
/// Written **last**, so the document covers every other file in the bundle
/// including `MANIFEST.json` and `README.md`. It cannot cover itself — a
/// document that hashes itself has no fixed point — which is the one gap, and
/// `Checksums::scan` excludes exactly that one path.
pub fn write_checksums(root: &Path) -> std::io::Result<(String, index::Checksums)> {
    write_checksums_reusing(root, &[], &[])
}

/// [`write_checksums`], reusing the digests in `known` for everything not named
/// in `rewritten`.
///
/// **This is the second half of A16's convergence, not a shortcut around it.**
/// `run` writes `verify/selfcheck.json` a second time to record A16's own row,
/// which changes that one file; the re-scan therefore re-digests one file rather
/// than the whole bundle, and `Checksums::a16` still re-reads every file from
/// disk to prove the result. `verify` passes nothing and scans everything, which
/// is correct: a bundle may have been tampered with since it was written, and
/// there is no honest way to carry a digest across that.
pub fn write_checksums_reusing(
    root: &Path,
    known: &[index::FileEntry],
    rewritten: &[&str],
) -> std::io::Result<(String, index::Checksums)> {
    let c = index::Checksums::scan_reusing(root, known, rewritten)?;
    write_doc(root, "verify/checksums.json", &c)?;
    Ok(("verify/checksums.json".to_string(), c))
}

/// Write `patchmaps/<capture_id>.json` and return the artifact-relative path.
///
/// Written for every admitted capture, including `removed` (which has a null
/// `plan`) and `added` (which has a null `old`). A consumer that wants to know
/// what a capture covers should be able to read one file and find out, rather
/// than infer it from which files happen to exist.
pub fn write_patchmap(
    root: &Path,
    capture_id: &str,
    map: &crate::assemble::PatchMap,
) -> std::io::Result<String> {
    write_doc(root, &format!("patchmaps/{capture_id}.json"), map)
}

/// Write `chunks/index.json` and return the artifact-relative path written.
///
/// Written even when nothing was stored — `--no-chunk-blobs` still produces the
/// role lists, because knowing which chunks *would* have been needed is the part
/// of the document that survives a metadata-only run.
pub fn write_chunk_index(root: &Path, index: &crate::blobs::ChunkIndex) -> std::io::Result<String> {
    write_doc(root, "chunks/index.json", index)
}

/// Write `diff/candidates.json` and `diff/matrix.json` — the scoring table and
/// the answer M5 produced from it.
///
/// Both are written even when nothing was admitted, because "nothing was
/// admitted, and here is everything that was considered and why each class came
/// out empty" is the more useful artifact of a run that found no candidates.
/// A matrix that only exists on success cannot answer the question that made
/// someone go looking for it.
pub fn write_selection(
    root: &Path,
    candidates: &serde_json::Value,
    selection: &crate::select::Selection,
    per_class: usize,
) -> std::io::Result<Vec<String>> {
    let mut out = Vec::with_capacity(2);
    out.push(write_doc(root, "diff/candidates.json", candidates)?);
    let matrix = serde_json::json!({
        "schema_version": crate::SCHEMA_VERSION,
        "per_class": per_class,
        "budget": {
            "limit": selection.budget.limit(),
            "spent": selection.bytes_admitted,
            "unlimited": selection.budget.limit().is_none(),
        },
        "required_order": crate::select::Class::REQUIRED_ORDER.map(|c| c.id()),
        "matrix": selection.matrix,
        "admitted": selection.admitted,
        "rejected": selection.rejected,
        "unmet": selection.unmet().iter().map(|c| c.id()).collect::<Vec<_>>(),
    });
    out.push(write_doc(root, "diff/matrix.json", &matrix)?);
    Ok(out)
}

/// The tree a capture writes its **old-side** assembled files into.
pub const GAME_TREE: &str = "game";
/// The tree a capture writes its **new-side** assembled files into.
///
/// **`expected/` is named once, here, on purpose.** `assemble` records
/// `Expectations::result_path` as a claim about where the file *will be*, and
/// this module writes it. With two literals, renaming the tree would have made
/// the recorded expectation a lie that no test could see — every assertion
/// would have been checking the new spelling in both places at once.
pub const EXPECTED_TREE: &str = "expected";

/// Write one artifact-relative document, pretty-printed, newline-terminated.
fn write_doc(root: &Path, rel: &str, value: &impl serde::Serialize) -> std::io::Result<String> {
    let body = serde_json::to_string_pretty(value)
        .map_err(|e| std::io::Error::other(format!("serialise {rel}: {e}")))?;
    write_newline_doc(root, rel, body.as_bytes())?;
    Ok(rel.to_string())
}

/// Write one artifact-relative file, creating its parents.
///
/// **The one place a bundle file is written.** It was six, governing 23 call
/// sites across `emit`, `emit::gamedir`, `manifest::dump` and `run` — two of
/// them byte-identical, the rest differing only in the return type (`PathBuf`
/// or `String`) and in whether they asserted the path round-trips. Six copies of
/// "create the parent, write the bytes" is six places for a bundle rule to be
/// stated once and forgotten five times.
///
/// The small metadata documents written here are **not** written atomically: a
/// half-written one is a small, reproducible failure. Chunk blobs are the only
/// thing written through `tmp/`-and-move, and that is
/// [`crate::blobs::Store`]'s own `write`.
pub fn write_bytes(root: &Path, rel: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    write_parts(root, rel, &[bytes])
}

/// [`write_bytes`] for a document that must end with a newline.
///
/// **Written as two `write_all` calls.** Every call site used
/// `format!("{body}\n")`, which allocates a second full copy of the whole
/// document to append one byte — 27 MB for the `game` manifest decode, the
/// largest document this tool produces.
///
/// The newline is not optional and is not the writer's to invent: bundle
/// documents have always ended in one, `checksums.json` records the digest
/// that results, and a change here would be invisible to
/// `tests/bundle_diff.rs`, which compares two *new* captures to each other
/// rather than either of them to a bundle already on disk.
pub fn write_newline_doc(root: &Path, rel: &str, body: &[u8]) -> std::io::Result<PathBuf> {
    write_parts(root, rel, &[body, b"\n"])
}

/// [`write_bytes`], returning the artifact-relative path that was written.
///
/// For the call sites that go straight into `MANIFEST.json → files[]`, where the
/// answer is the name and not the `PathBuf` it was built from.
pub fn write_rel(root: &Path, rel: &str, bytes: &[u8]) -> std::io::Result<String> {
    write_bytes(root, rel, bytes)?;
    Ok(rel.to_string())
}

fn write_parts(root: &Path, rel: &str, parts: &[&[u8]]) -> std::io::Result<PathBuf> {
    let path = root.join(rel);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::File::create(&path)?;
    for p in parts {
        std::io::Write::write_all(&mut f, p)?;
    }
    debug_assert_eq!(
        rel_path_string(root, &path),
        rel,
        "the path must round-trip"
    );
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biz::BranchParams;
    use crate::config::{NewTagSelector, OldTagSelector, VersionPair};
    use crate::http::probe::{TagProbeRow, finish};
    use crate::hyp::CapturedBody;

    fn body(name: &str, endpoint: &'static str) -> CapturedBody {
        CapturedBody {
            endpoint,
            file: name.to_string(),
            seq: 0,
            bytes: format!(r#"{{"retcode":0,"echo":"{name}"}}"#).into_bytes(),
        }
    }

    fn captured() -> Captured {
        let params = BranchParams {
            package_id: "p".into(),
            password: "w".into(),
            branch: "main".into(),
            tag: "7.1.0".into(),
            diff_tags: vec!["7.0.0".into()],
        };
        let mut versions = VersionPair::select(
            &params,
            NewTagSelector::BranchHead,
            OldTagSelector::FirstDiffTag,
        )
        .unwrap();
        versions.attach_build_ids("n", "o");
        Captured {
            configs: body("getGameConfigs.json", "getGameConfigs"),
            branches: body("getGameBranches.json", "getGameBranches"),
            deprecated: body(
                "getGameDeprecatedFileConfigs.json",
                "getGameDeprecatedFileConfigs",
            ),
            packages: body("getGamePackages.json", "getGamePackages"),
            build_new: body("getBuild.7.1.0.json", "getBuild"),
            build_old: body("getBuild.7.0.0.json", "getBuild"),
            versions,
        }
    }

    fn probe() -> TagProbe {
        finish(TagProbe {
            schema_version: 1,
            requested: vec!["7.1.0".into()],
            results: vec![TagProbeRow {
                tag: "7.1.0".into(),
                in_diff_tags: false,
                http: 200,
                retcode: 0,
                build_id: Some("n".into()),
                tag_echoed: Some("7.1.0".into()),
                served: true,
                already_fetched: true,
            }],
            summary: Default::default(),
        })
    }

    fn shapes() -> ErrorShapes {
        ErrorShapes {
            schema_version: 1,
            shapes: vec![crate::http::probe::ErrorShape {
                id: "chunk_missing".into(),
                family: crate::http::probe::Family::Blob,
                request: "a chunk id the build does not have".into(),
                url: "https://cdn/chunks/s/b/c".into(),
                status: 404,
                content_type: Some("application/xml".into()),
                retcode: None,
                error: true,
                body: "<Error><Code>NoSuchKey</Code></Error>".into(),
                volatile_fields: vec!["RequestId".into()],
                retryable: false,
            }],
        }
    }

    #[test]
    fn the_eight_api_documents_are_written_byte_exact() {
        let dir = std::env::temp_dir().join(format!("gircap-api-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let written = write_api(&dir, &captured(), &probe(), &shapes()).unwrap();

        assert_eq!(
            written,
            vec![
                "api/getGameConfigs.json",
                "api/getGameBranches.json",
                "api/getGameDeprecatedFileConfigs.json",
                "api/getGamePackages.json",
                "api/getBuild.7.1.0.json",
                "api/getBuild.7.0.0.json",
                "api/tag_probe.json",
                "api/error_shapes.json",
            ]
        );
        for rel in &written {
            assert!(dir.join(rel).exists(), "{rel} missing");
        }
        // Byte-exact: the file holds the response, whitespace and all.
        assert_eq!(
            std::fs::read_to_string(dir.join("api/getGameConfigs.json")).unwrap(),
            r#"{"retcode":0,"echo":"getGameConfigs.json"}"#
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The negative document has to reach `files[]`, because that list is what
    /// `MANIFEST.json` and `verify/checksums.json` are built from. Returning it
    /// from here is the whole mechanism, so it is pinned rather than assumed.
    #[test]
    fn the_error_shapes_document_is_returned_so_it_reaches_files_and_checksums() {
        let dir = std::env::temp_dir().join(format!("gircap-api-shapes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let written = write_api(&dir, &captured(), &probe(), &shapes()).unwrap();
        assert!(
            written.contains(&"api/error_shapes.json".to_string()),
            "not in the returned list, so it would not be checksummed: {written:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_generated_probe_is_pretty_printed_and_valid_json() {
        let dir = std::env::temp_dir().join(format!("gircap-api-probe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_api(&dir, &captured(), &probe(), &shapes()).unwrap();
        let text = std::fs::read_to_string(dir.join("api/tag_probe.json")).unwrap();
        assert!(text.contains('\n'), "a document is not one long line");
        assert!(text.ends_with('\n'), "a document ends with a newline");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["results"][0]["already_fetched"], true);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The negative document carries a body that is *not* JSON — it is an XML
    /// error envelope — and the whole document still has to be valid JSON, or
    /// every consumer of a bundle breaks on it.
    #[test]
    fn the_error_shapes_document_is_valid_json_despite_its_xml_body() {
        let dir =
            std::env::temp_dir().join(format!("gircap-api-shapes-json-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_api(&dir, &captured(), &probe(), &shapes()).unwrap();
        let text = std::fs::read_to_string(dir.join("api/error_shapes.json")).unwrap();
        assert!(text.ends_with('\n'), "a document ends with a newline");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["shapes"][0]["id"], "chunk_missing");
        assert_eq!(v["shapes"][0]["status"], 404);
        assert_eq!(v["shapes"][0]["family"], "blob");
        // The XML body is carried as a string, escaped, not dropped.
        assert_eq!(
            v["shapes"][0]["body"],
            "<Error><Code>NoSuchKey</Code></Error>"
        );
        assert_eq!(v["shapes"][0]["retryable"], false);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_log_lands_at_api_requests_ndjson_and_creates_its_directory() {
        let dir = std::env::temp_dir().join(format!("gircap-emit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir
            .join("hk4e_global")
            .join("7.0.0__7.1.0")
            .join("20261014T101500Z");

        let log = RequestLog::new();
        assert_eq!(write_requests_log(&root, &log).unwrap(), 0);
        assert!(root.join("api").join("requests.ndjson").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
