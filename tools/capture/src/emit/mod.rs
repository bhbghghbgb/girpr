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

pub mod closeout;
pub mod fixture;
pub mod gamedir;
pub mod index;
pub mod selfcheck;
pub mod tree;

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
/// Write the six `api/` bodies, the tag probe and the error shapes.
///
/// **`log` is what fills `stored_as` on the six bodies' rows.** Each body knows
/// its request's sequence number (filled in by [`hyp::attach_seq`]), and the log
/// resolves that number to the row it wrote. Without it the bundle records a
/// null `stored_as` for six files that are demonstrably on disk — a reader
/// asking "do I have this body" has to go and look.
///
/// The probe and shape documents are written here too, and are deliberately
/// **not** marked. They come from the tag probe and the negative probes, whose
/// rows are a different set of requests, and a null `stored_as` on a probe row
/// is the honest answer: the probe ran, and its answer is a summary rather than
/// a single stored body.
pub fn write_api(
    root: &Path,
    captured: &Captured,
    probe: &TagProbe,
    shapes: &ErrorShapes,
    log: &RequestLog,
) -> std::io::Result<Vec<String>> {
    let mut written = Vec::with_capacity(8);
    for body in captured.bodies() {
        let rel = format!("api/{}", body.file);
        write_bytes(root, &rel, &body.bytes)?;
        log.set_stored_as(body.seq, &rel);
        written.push(rel);
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
///
/// **`patchmaps/` is created first, as part of writing this.** It used to be a
/// `create_dir_all` in the orchestrator, one line above the call, which meant
/// the directory's existence depended on a caller remembering an unrelated step.
/// `--no-chunk-blobs` admits captures and then stores no blobs, so nothing is
/// written there and the directory simply never appeared — which made `verify`
/// refuse the bundle as unreadable (exit 1, "a bundle with no patchmaps has no
/// captures to verify") rather than report the finding a reader actually wants,
/// which is that the bundle cannot be materialized. An empty directory costs
/// nothing and makes the layout uniform.
///
/// So the guarantee is now local: *if `chunks/index.json` exists, `patchmaps/`
/// exists.* A caller cannot get a bundle with one and not the other.
pub fn write_chunk_index(root: &Path, index: &crate::blobs::ChunkIndex) -> std::io::Result<String> {
    std::fs::create_dir_all(root.join("patchmaps"))?;
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

/// A `Write` that counts what passes through it and keeps none of it.
///
/// [`json_len`] needs to know how long a serialised document will be
/// before deciding how to write it, and a streaming writer cannot be asked.
struct Counter(usize);

impl std::io::Write for Counter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// How many bytes `value` occupies serialised pretty-printed, without building it.
///
/// **The same bytes the buffered path produces**, so the length this
/// returns is the length of the document that gets written and nothing else.
/// `what` names the document in the error, because a serialisation failure
/// halfway into a 27 MB decode with no subject is a bad error report.
pub fn json_len(value: &impl serde::Serialize, what: &str) -> std::io::Result<usize> {
    let mut c = Counter(0);
    serde_json::to_writer_pretty(&mut c, value)
        .map_err(|e| std::io::Error::other(format!("serialise {what}: {e}")))?;
    Ok(c.0)
}

/// Write a serialisable document straight into its file, gzipping on the way.
///
/// Building the whole body with `serde_json::to_string_pretty` costs a ~27 MB
/// `String` for a 2 744-entry manifest decode, and the gzip call then builds a
/// second ~7 MB compressed copy while that `String` is still alive.
/// Serialising through the encoder holds neither: peak allocation is the
/// 8 KB `BufWriter` and the encoder's own window.
///
/// **The bytes are the same as a `to_string_pretty`-then-gzip path would produce**,
/// which is what makes streaming safe here rather than a bundle-visible change.
/// Two facts, both tested:
///
///  - `to_writer_pretty` and `to_string_pretty` share one `PrettyFormatter`, so
///    the uncompressed document is identical. Pinned by
///    `streaming_a_plain_document_writes_the_bytes_the_buffered_path_wrote`.
///  - `flate2`'s deflate output does not depend on how the input is chunked, so
///    the `BufWriter` coalescing `to_writer`'s token-at-a-time writes cannot
///    change the compressed bytes. Pinned by
///    `streaming_a_gzipped_document_writes_the_bytes_the_buffered_path_wrote`
///    and, underneath it,
///    `hash::tests::gzip_bytes_do_not_depend_on_how_the_input_was_fed`.
///
/// Not written atomically, like every other bundle document here: see
/// [`write_bytes`]. A partial manifest decode is a small, reproducible failure,
/// and the alternative was keeping 34 MB alive to make it merely unlikely.
///
/// `gzip` is the caller's decision, not this function's — it is taken by
/// comparing the length from [`json_len`] against `--gzip-threshold`, and it
/// determines the file's name, which is why the length is measured before the
/// body is written.
pub fn write_json_gz(
    root: &Path,
    rel: &str,
    value: &impl serde::Serialize,
    gzip: bool,
) -> std::io::Result<PathBuf> {
    let path = root.join(rel);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::fs::File::create(&path)?;
    if gzip {
        // **The buffer goes between serde_json and the encoder**, which is the
        // opposite end from where it looks like it belongs, and the order is the
        // difference between this change costing 12 s and costing 46 s.
        //
        // `to_writer_pretty` emits a token at a time, so a document with 2 744
        // entries is tens of millions of tiny `write` calls. Wrapping the encoder
        // in the buffer lets deflate see 8 KB at a time; buffering the encoder's
        // *output* instead leaves deflate being called once per token, which
        // costs about 4x the wall clock for the same bytes. Measured, not assumed.
        let mut w = std::io::BufWriter::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::default(),
        ));
        serde_json::to_writer_pretty(&mut w, value)
            .map_err(|e| std::io::Error::other(format!("serialise {rel}: {e}")))?;
        // `into_inner` flushes the buffer and hands the encoder back; it is where
        // the last partial buffer reaches the encoder.
        let enc = w
            .into_inner()
            .map_err(|e| std::io::Error::other(format!("flush {rel}: {}", e.into_error())))?;
        // `finish()?` and not a bare drop. **The bytes are the same either way**
        // — `flate2::write::GzEncoder`'s `Drop` attempts the finish, so the
        // trailer is not what this call is for, contrary to what the comment
        // here used to say. `Drop` cannot report an error, though, and a
        // truncated manifest decode must not be written silently. Pinned by
        // `hash::tests::dropping_a_gz_encoder_without_finishing_produces_the_same_bytes`.
        enc.finish()?;
    } else {
        let mut w = std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(&mut w, value)
            .map_err(|e| std::io::Error::other(format!("serialise {rel}: {e}")))?;
        std::io::Write::flush(&mut w)?;
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

    /// A document shaped like a real manifest decode, ~200 entries.
    ///
    /// **A stand-in, and honestly so.** The real `FullDecode` needs a captured
    /// manifest, and a fixture built from it would be a bundle-sized setup for a
    /// claim about serialisation. What matters is that this carries the same JSON
    /// shapes in the same proportions: borrowed strings, a u32, a bool, an i64,
    /// and a nested array of nested structs — so `to_writer_pretty` is walking
    /// the same kinds of tokens as the real decode.
    ///
    /// **Sized past `BufWriter`'s 8 KB**, so the buffer actually fills and the
    /// encoder is handed partial buffers rather than one clean write. A document
    /// under 8 KB never exercises the boundary this change is about.
    #[derive(serde::Serialize)]
    struct Decode {
        schema_version: u32,
        tag: &'static str,
        build_id: &'static str,
        matching_field: &'static str,
        entries: Vec<Entry>,
    }

    #[derive(serde::Serialize)]
    struct Entry {
        file: String,
        chunks: Vec<Chunk>,
        is_folder: bool,
        size: i64,
        md5: String,
    }

    #[derive(serde::Serialize)]
    struct Chunk {
        id: String,
        compressed_size: i64,
        uncompressed_size: i64,
        md5_compressed: String,
        md5_uncompressed: String,
        unknown_field6: i64,
    }

    fn decode_fixture() -> Decode {
        let entries = (0..200)
            .map(|i| Entry {
                file: format!("Audio/Soundbank{i}/Media/stream_{i:04}.ww2"),
                chunks: (0..3u64)
                    .map(|c| Chunk {
                        id: format!("{:016x}_{c:032x}", (i as u64) * 7919 + c),
                        compressed_size: (i as i64) * 1013,
                        uncompressed_size: (i as i64) * 7919 + c as i64,
                        md5_compressed: format!("{:032x}", (i as u64) * 31 + c),
                        md5_uncompressed: format!("{:032x}", (i as u64) * 17 + c),
                        unknown_field6: c as i64,
                    })
                    .collect(),
                is_folder: i % 17 == 0,
                size: (i as i64) * 23_713,
                md5: format!("{:032x}", (i as u64) * 104_729),
            })
            .collect();
        Decode {
            schema_version: 1,
            tag: "7.1.0",
            build_id: "e0ea3f6d",
            matching_field: "game",
            entries,
        }
    }

    fn tmp(name: &str) -> girpr_tempdir::TempDir {
        // Absent, not created: one of these cases is about the writer creating
        // its own parents, so pre-creating the root would assert nothing.
        girpr_tempdir::TempDir::absent("gircap-emit", name)
    }

    #[test]
    fn streaming_a_gzipped_document_writes_the_bytes_the_buffered_path_wrote() {
        // The compressed bytes of `manifests/*.json.gz` are in `checksums.json`,
        // so if streaming through the encoder moved a single byte every reference
        // bundle on disk would go stale and `SCHEMA_VERSION` would have to move
        // with it. This is the test that says it does not.
        //
        // The comparison is against the whole-body alternative — `to_string_pretty`,
        // then `hash::gzip`, then one `write_all` — not against a golden file, so
        // it cannot rot.
        let root = tmp("gz-identical");
        let d = decode_fixture();
        write_json_gz(&root, "manifest.json.gz", &d, true).expect("streamed write");

        let buffered = crate::hash::gzip(
            serde_json::to_string_pretty(&d)
                .expect("to_string_pretty")
                .as_bytes(),
        )
        .expect("buffered gzip");
        let streamed = std::fs::read(root.join("manifest.json.gz")).expect("read back");

        assert_eq!(
            streamed.len(),
            buffered.len(),
            "the compressed body changed size"
        );
        assert!(
            streamed == buffered,
            "the compressed body changed bytes: every manifest digest in every \
             reference bundle would move"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn streaming_a_plain_document_writes_the_bytes_the_buffered_path_wrote() {
        // The identity branch, which a `--gzip-threshold` above the body size
        // takes. Same claim, no encoder involved: `to_writer_pretty` and
        // `to_string_pretty` must agree.
        let root = tmp("plain-identical");
        let d = decode_fixture();
        write_json_gz(&root, "manifest.json", &d, false).expect("streamed write");

        let buffered = serde_json::to_string_pretty(&d).expect("to_string_pretty");
        assert!(
            buffered.len() > 8 * 1024,
            "the fixture must outgrow BufWriter's 8 KB, or the buffer boundary \
             this change is about is never crossed (fixture is {} B)",
            buffered.len(),
        );
        let streamed = std::fs::read(root.join("manifest.json")).expect("read back");

        assert_eq!(streamed, buffered.as_bytes());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn json_len_is_the_length_of_the_bytes_that_get_written() {
        // `json_len` decides the file's *name* and what `files[].uncompressed_bytes`
        // claims, and the stream that follows cannot be consulted about either.
        // So the two must agree, for
        // both branches — otherwise the bundle records a length no reader can
        // reproduce from the file beside it.
        let root = tmp("len-agrees");
        let d = decode_fixture();
        let claimed = json_len(&d, "the fixture").expect("json_len");

        write_json_gz(&root, "plain.json", &d, false).expect("plain");
        assert_eq!(
            claimed,
            std::fs::read(root.join("plain.json")).expect("read").len(),
            "the plain document is not the length that was measured"
        );

        write_json_gz(&root, "gz.json.gz", &d, true).expect("gz");
        let round_tripped =
            crate::hash::gunzip(&std::fs::read(root.join("gz.json.gz")).expect("read"))
                .expect("gunzip");
        assert_eq!(
            claimed,
            round_tripped.len(),
            "the gzipped document does not decompress to the length that was measured"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

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
        let dir = girpr_tempdir::TempDir::new("gircap-api", "byte_exact");
        let written =
            write_api(&dir, &captured(), &probe(), &shapes(), &RequestLog::new()).unwrap();

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
        let dir = girpr_tempdir::TempDir::new("gircap-api-shapes", "reaches_files");
        let written =
            write_api(&dir, &captured(), &probe(), &shapes(), &RequestLog::new()).unwrap();
        assert!(
            written.contains(&"api/error_shapes.json".to_string()),
            "not in the returned list, so it would not be checksummed: {written:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_generated_probe_is_pretty_printed_and_valid_json() {
        let dir = girpr_tempdir::TempDir::new("gircap-api-probe", "pretty_printed");
        write_api(&dir, &captured(), &probe(), &shapes(), &RequestLog::new()).unwrap();
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
        let dir = girpr_tempdir::TempDir::new("gircap-api-shapes-json", "valid_json_despite_xml");
        write_api(&dir, &captured(), &probe(), &shapes(), &RequestLog::new()).unwrap();
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
        let dir = girpr_tempdir::TempDir::absent("gircap-emit-log", "creates_its_directory");
        let root = dir
            .join("hk4e_global")
            .join("7.0.0__7.1.0")
            .join("20261014T101500.000000Z");

        let log = RequestLog::new();
        assert_eq!(write_requests_log(&root, &log).unwrap(), 0);
        assert!(root.join("api").join("requests.ndjson").exists());
    }
}
