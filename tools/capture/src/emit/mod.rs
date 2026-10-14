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

use std::path::{Path, PathBuf};

use crate::config::rel_path_string;
use crate::http::RequestLog;
use crate::http::probe::TagProbe;
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
/// and the tree cannot disagree about what exists.
///
/// The six response bodies are written **exactly as served** — not re-serialized
/// from a typed view — because a re-serialization would silently drop or restyle
/// a field, and the whole point of capturing the body is that a reader can `diff`
/// it against a fresh request. `tag_probe.json` is the one generated document:
/// it is this tool's own measurement, not the API's words.
pub fn write_api(root: &Path, captured: &Captured, probe: &TagProbe) -> std::io::Result<Vec<String>> {
    let mut written = Vec::with_capacity(7);
    for body in captured.bodies() {
        write_bytes(root, &format!("api/{}", body.file), &body.bytes)?;
        written.push(format!("api/{}", body.file));
    }
    let probe_json = hyp::render_json(probe).map_err(std::io::Error::other)?;
    write_bytes(
        root,
        "api/tag_probe.json",
        format!("{probe_json}\n").as_bytes(),
    )?;
    written.push("api/tag_probe.json".to_string());
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

/// Write one artifact-relative document, pretty-printed, newline-terminated.
fn write_doc(root: &Path, rel: &str, value: &impl serde::Serialize) -> std::io::Result<String> {
    let body = serde_json::to_string_pretty(value)
        .map_err(|e| std::io::Error::other(format!("serialise {rel}: {e}")))?;
    write_bytes(root, rel, format!("{body}\n").as_bytes())?;
    Ok(rel.to_string())
}

/// Write one artifact-relative file, creating its parents.
///
/// Nothing here is atomic: M8 introduces the `tmp/`-and-move rule when the
/// artifact starts holding files worth more than a few kilobytes. A half-written
/// metadata document is a small, reproducible failure; it is not worth the
/// machinery yet.
fn write_bytes(root: &Path, rel: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let path = root.join(rel);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, bytes)?;
    debug_assert_eq!(rel_path_string(root, &path), rel, "the path must round-trip");
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
        let mut versions =
            VersionPair::select(&params, NewTagSelector::BranchHead, OldTagSelector::FirstDiffTag)
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

    #[test]
    fn the_seven_api_documents_are_written_byte_exact() {
        let dir = std::env::temp_dir().join(format!("gircap-api-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let written = write_api(&dir, &captured(), &probe()).unwrap();

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

    #[test]
    fn the_generated_probe_is_pretty_printed_and_valid_json() {
        let dir = std::env::temp_dir().join(format!("gircap-api-probe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_api(&dir, &captured(), &probe()).unwrap();
        let text = std::fs::read_to_string(dir.join("api/tag_probe.json")).unwrap();
        assert!(text.contains('\n'), "a document is not one long line");
        assert!(text.ends_with('\n'), "a document ends with a newline");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["results"][0]["already_fetched"], true);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_log_lands_at_api_requests_ndjson_and_creates_its_directory() {
        let dir = std::env::temp_dir().join(format!("gircap-emit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("hk4e_global").join("7.0.0__7.1.0").join("20261014T101500Z");

        let log = RequestLog::new();
        assert_eq!(write_requests_log(&root, &log).unwrap(), 0);
        assert!(root.join("api").join("requests.ndjson").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}