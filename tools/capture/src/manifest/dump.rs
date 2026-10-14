//! Writing a decoded manifest out: the archive, the full JSON decode, the meta
//! document, and the `paths.tsv` sidecar.
//!
//! **M3.** Four files, two decisions.
//!
//! **The gzip rule.** A document is gzipped only when its *uncompressed* body
//! exceeds `--gzip-threshold`. The decision is returned to the caller and
//! recorded, never inferred from a filename — so a reader never has to guess.
//! Every gzipped document ships a plain-text sidecar, because the two features
//! are not independent: the sidecar exists *because* the dump may be compressed.
//!
//! **The archive is stored verbatim.** `<field>.manifest.zst` is the bytes the
//! CDN served, not a re-compression, so `md5(stored) == content-md5` holds and
//! a reader can compare the bundle against the CDN directly. Compressing it
//! again would be one more lossy step in a chain that has no reason to have one.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::hash::{self, Encoding};
use crate::cli::ManifestDetail;
use crate::manifest::proto::SophonChunkFile;
use crate::manifest::{CapturedManifest, ManifestMeta, Stored};

/// The artifact-relative path of a manifest's files.
pub fn dir(tag: &str) -> String {
    format!("manifests/{tag}")
}

/// The archive, stored exactly as served.
pub fn archive_rel(tag: &str, field: &str) -> String {
    format!("{}/{field}.manifest.zst", dir(tag))
}

/// The `meta.json` document. Always plain: it is small and read constantly.
pub fn meta_rel(tag: &str, field: &str) -> String {
    format!("{}/{field}.manifest.meta.json", dir(tag))
}

/// The full decode, `.json` or `.json.gz` by the threshold.
pub fn full_prefix(tag: &str, field: &str) -> String {
    format!("{}/{field}.manifest", dir(tag))
}

/// The plain-text sidecar, always plain.
pub fn paths_rel(tag: &str, field: &str) -> String {
    format!("{}/{field}.paths.tsv", dir(tag))
}

/// Where a document ended up and how it is encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAs {
    /// Artifact-relative path, including the `.gz` when there is one.
    pub path: String,
    /// How the bytes on disk are encoded.
    pub encoding: Encoding,
    /// Uncompressed body length — what the threshold was compared against.
    pub uncompressed_bytes: u64,
}

/// Write one captured manifest: archive, meta, full decode, sidecar.
///
/// Returns the paths and encodings written, in the order they were, so the
/// caller can put them in `MANIFEST.json → files[]` without keeping a second
/// bookkeeping list that could disagree with the tree.
///
/// `detail` decides whether the full decode is written at all. Under
/// [`crate::cli::ManifestDetail::PathsOnly`] the archive, the meta document and
/// the sidecar all still land — a sidecar with no archive beside it describes
/// nothing a reader can check — and only the multi-megabyte decode is dropped.
pub fn write(
    root: &Path,
    captured: &mut CapturedManifest,
    gzip_threshold: u64,
    detail: crate::cli::ManifestDetail,
) -> std::io::Result<Vec<StoredAs>> {
    
    let tag = captured.tag.clone();
    let field = captured.matching_field.clone();
    let mut out = Vec::with_capacity(4);

    // 1. The archive, byte-exact.
    let archive = write_file(root, &archive_rel(&tag, &field), &captured.raw)?;
    captured.meta.stored.path = archive.clone();
    out.push(StoredAs {
        path: archive,
        encoding: Encoding::Identity,
        uncompressed_bytes: captured.raw.len() as u64,
    });

    // 2. The full decode, gzipped past the threshold. This is the only document
    //    today that trips it: a 2 744-entry decode is tens of megabytes of JSON.
    //
    //    Both forms are `.manifest.json[.gz]` — the `.json` is there even when
    //    the body is compressed, so the pair is one document with two encodings
    //    rather than two documents that differ in name. `MANIFEST.json ->
    //    files[].encoding` says which one is on disk.
    if detail == ManifestDetail::Full {
        let full = FullDecode::from_captured(captured);
        let body = serde_json::to_string_pretty(&full)
            .map_err(|e| std::io::Error::other(format!("serialise the decode: {e}")))?;
        let gzipped = hash::should_gzip(body.len(), gzip_threshold);
        let uncompressed_bytes = body.len() as u64;
        let (rel, encoding) = if gzipped {
            (format!("{}.json.gz", full_prefix(&tag, &field)), Encoding::Gzip)
        } else {
            (
                format!("{}.json", full_prefix(&tag, &field)),
                Encoding::Identity,
            )
        };
        let payload = if gzipped {
            hash::gzip(body.as_bytes())?
        } else {
            body.into_bytes()
        };
        write_file(root, &rel, &payload)?;
        out.push(StoredAs {
            path: rel,
            encoding,
            uncompressed_bytes,
        });
    }

    // 3. The sidecar. Always plain, always tab-separated, always with a header:
    //    it is the thing a reader greps when the decode beside it is `.gz`, and
    //    under `paths-only` it is the *only* per-path document there is.
    let tsv = paths_tsv(&captured.entries);
    write_file(root, &paths_rel(&tag, &field), tsv.as_bytes())?;
    out.push(StoredAs {
        path: paths_rel(&tag, &field),
        encoding: Encoding::Identity,
        uncompressed_bytes: tsv.len() as u64,
    });

    // 4. The meta document, written last so it can carry the archive's path and
    //    record that the decode was not written.
    captured.meta.full_decode_written = detail == ManifestDetail::Full;
    let meta = serde_json::to_string_pretty(&captured.meta)
        .map_err(|e| std::io::Error::other(format!("serialise the manifest meta: {e}")))?;
    write_file(
        root,
        &meta_rel(&tag, &field),
        format!("{meta}\n").as_bytes(),
    )?;
    out.push(StoredAs {
        path: meta_rel(&tag, &field),
        encoding: Encoding::Identity,
        uncompressed_bytes: meta.len() as u64 + 1,
    });

    Ok(out)
}

/// The whole decoded manifest, as the document schema names it.
#[derive(Debug, Serialize)]
pub struct FullDecode {
    /// The bundle schema version.
    pub schema_version: u32,
    /// The tag this decode belongs to.
    pub tag: String,
    /// The build it was published under.
    pub build_id: String,
    /// `game`, or an audio code.
    pub matching_field: String,
    /// Every entry, verbatim from the protobuf.
    pub entries: Vec<SophonChunkFile>,
}

impl FullDecode {
    /// Build it from a captured manifest.
    pub fn from_captured(c: &CapturedManifest) -> Self {
        FullDecode {
            schema_version: crate::SCHEMA_VERSION,
            tag: c.tag.clone(),
            build_id: c.build_id.clone(),
            matching_field: c.matching_field.clone(),
            entries: c.entries.clone(),
        }
    }
}

/// The `paths.tsv` sidecar: `path \t size \t md5 \t nchunks \t is_folder`.
///
/// Header line, `\n` endings, one row per entry in decode order. Sorted by path
/// so `diff` between two tags' sidecars reads as a diff rather than as a
/// reordering — decode order is an implementation detail of the archive, and a
/// reader comparing two versions is comparing *paths*.
pub fn paths_tsv(entries: &[SophonChunkFile]) -> String {
    let mut rows: Vec<(String, i64, String, usize, bool)> = entries
        .iter()
        .map(|e| {
            (
                e.file.clone(),
                e.size,
                e.md5.clone(),
                e.chunks.len(),
                e.is_folder,
            )
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let mut s = String::with_capacity(rows.len() * 96 + 48);
    s.push_str("path\tsize\tmd5\tnchunks\tis_folder\n");
    for (p, size, md5, n, folder) in rows {
        s.push_str(&p);
        s.push('\t');
        s.push_str(&size.to_string());
        s.push('\t');
        s.push_str(&md5);
        s.push('\t');
        s.push_str(&n.to_string());
        s.push('\t');
        s.push_str(if folder { "true" } else { "false" });
        s.push('\n');
    }
    s
}

fn write_file(root: &Path, rel: &str, bytes: &[u8]) -> std::io::Result<String> {
    let path = root.join(rel);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, bytes)?;
    Ok(rel.to_string())
}

/// Reconcile a decode against `getBuild.stats` and log the verdict.
///
/// Called from `capture` rather than here so that a mismatch is visible in the
/// same place the decode happened, whether or not the bundle is written.
pub fn note_reconciliation(m: &CapturedManifest) {
    let stats = m.meta.declared.stats.as_ref();
    let bad = super::reconcile(&m.meta.decoded, stats);
    if bad.is_empty() {
        tracing::debug!(tag = %m.tag, field = %m.matching_field, "decode reconciles with getBuild.stats");
    } else {
        for (name, declared, actual) in bad {
            tracing::warn!(
                tag = %m.tag,
                field = %m.matching_field,
                stat = name,
                declared, actual,
                "decoded manifest does not reconcile with the declared stats"
            );
        }
    }
}

/// The meta document's `stored` block, for a caller building one by hand.
pub fn stored_block(path: &str, bytes: &[u8], decompressed_len: usize) -> Stored {
    Stored {
        path: path.to_string(),
        bytes: bytes.len() as u64,
        container: "zstd",
        md5_compressed: hash::md5_hex(bytes),
        sha256_compressed: hash::sha256_hex(bytes),
        md5_decompressed: String::new(),
        bytes_decompressed: decompressed_len as u64,
        encoding: Encoding::Identity,
    }
}

/// Where the manifests of one tag live.
pub fn tag_dir(root: &Path, tag: &str) -> PathBuf {
    root.join(dir(tag))
}

/// The meta document as a `serde_json::Value`, for a test or a reader.
pub fn meta_json(m: &ManifestMeta) -> serde_json::Value {
    serde_json::to_value(m).expect("ManifestMeta is all owned data")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::proto::SophonChunk;
    use crate::manifest::{Decoded, IdParts, ManifestMeta, Verified};

    fn entry(path: &str, size: i64, n: usize, folder: bool) -> SophonChunkFile {
        SophonChunkFile {
            file: path.into(),
            chunks: (0..n)
                .map(|i| SophonChunk {
                    id: format!("c{i}"),
                    uncompressed_md5: "u".into(),
                    offset: i as i64 * 10,
                    compressed_size: 5,
                    uncompressed_size: 10,
                    unknown_field6: 1,
                    compressed_md5: "k".into(),
                })
                .collect(),
            is_folder: folder,
            size,
            md5: format!("md5-{path}"),
        }
    }

    fn captured() -> CapturedManifest {
        CapturedManifest {
            tag: "7.1.0".into(),
            build_id: "sMXGW2ll3Fuu".into(),
            matching_field: "game".into(),
            seq: 4,
            raw: vec![0x28, 0xB5, 0x2F, 0xFD, 0x01, 0x02],
            entries: vec![
                entry("GenshinImpact_Data/b.txt", 20, 2, false),
                entry("GenshinImpact_Data/a.txt", 10, 1, false),
                entry("GenshinImpact_Data/dir", 0, 0, true),
            ],
            meta: ManifestMeta {
                schema_version: 1,
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                matching_field: "game".into(),
                category_id: "10016".into(),
                category_name: "\u{6e38}\u{620f}\u{672c}\u{4f53}".into(),
                manifest: super::super::DeclaredManifest {
                    id: "manifest_aaaaaaaaaaaaaaaa_deadbeef".into(),
                    checksum: "deadbeef".into(),
                    compressed_size: "9007187".into(),
                    uncompressed_size: "16827424".into(),
                },
                declared: Default::default(),
                id_parts: IdParts {
                    prefix_16hex: "manifest_aaaaaaaaaaaaaaaa".into(),
                    prefix_semantics: "opaque",
                    suffix_32hex: "deadbeef".into(),
                    suffix_equals_checksum: true,
                },
                stored: super::stored_block("", &[], 0),
                verified: Verified {
                    decompressed_md5_matches_checksum: true,
                    checksum_matches_id_suffix: true,
                    id_prefix_verified: false,
                },
                decoded: Decoded::default(),
                full_decode_written: true,
            },
        }
    }

    fn tmp(name: &str) -> PathBuf {
        // Named per test, not per process: these run concurrently in one
        // process, and a shared path means one test's `remove_dir_all` deletes
        // another's tree mid-run — which fails intermittently and looks like a
        // writer bug rather than a test bug.
        let d = std::env::temp_dir().join(format!("gircap-dump-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_four_files_land_where_the_schema_documents_them() {
        let root = tmp("layout");
        let mut c = captured();
        let written = write(&root, &mut c, 1_048_576, ManifestDetail::Full).unwrap();
        let paths: Vec<&str> = written.iter().map(|w| w.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "manifests/7.1.0/game.manifest.zst",
                "manifests/7.1.0/game.manifest.json",
                "manifests/7.1.0/game.paths.tsv",
                "manifests/7.1.0/game.manifest.meta.json",
            ]
        );
        for p in &paths {
            assert!(root.join(p).exists(), "{p}");
        }
        // The meta document learned where its archive went.
        let meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(paths[3])).unwrap()).unwrap();
        assert_eq!(meta["stored"]["path"], "manifests/7.1.0/game.manifest.zst");
        assert_eq!(meta["id_parts"]["prefix_semantics"], "opaque");
        assert_eq!(meta["verified"]["id_prefix_verified"], false);
        assert_eq!(meta["category_name"], "\u{6e38}\u{620f}\u{672c}\u{4f53}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The archive is the bytes the CDN sent, so its MD5 is the CDN's
    /// `content-md5`. Re-compressing it would break exactly that.
    #[test]
    fn the_archive_is_stored_verbatim() {
        let root = tmp("verbatim");
        let mut c = captured();
        write(&root, &mut c, 1_048_576, ManifestDetail::Full).unwrap();
        let on_disk = std::fs::read(root.join(archive_rel("7.1.0", "game"))).unwrap();
        assert_eq!(on_disk, c.raw);
        assert_eq!(hash::md5_hex(&on_disk), hash::md5_hex(&c.raw));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The one decision that shapes the tree, asserted in both directions.
    /// The one decision that shapes the tree, asserted in both directions.
    ///
    /// Each threshold gets its **own** root, because that is what a run does:
    /// one capture writes one tree with one threshold. Writing twice into one
    /// root would leave both forms on disk and make the test assert something
    /// about a scenario that cannot happen — and a writer that deletes a file it
    /// did not create is worse than one that does not.
    #[test]
    fn the_gzip_rule_fires_above_the_threshold_and_not_below() {
        let plain_root = tmp("gzip-plain");
        let mut a = captured();
        let w = write(&plain_root, &mut a, 10_000_000, ManifestDetail::Full).unwrap();
        let full = w
            .iter()
            .find(|w| w.path.contains("game.manifest.json"))
            .expect("the plain dump is reported");
        assert_eq!(full.encoding, Encoding::Identity);
        assert_eq!(full.path, "manifests/7.1.0/game.manifest.json");
        assert!(plain_root.join(&full.path).exists());
        assert_eq!(
            full.uncompressed_bytes,
            std::fs::metadata(plain_root.join(&full.path)).unwrap().len()
        );

        let gz_root = tmp("gzip-gz");
        let mut b = captured();
        let w = write(&gz_root, &mut b, 0, ManifestDetail::Full).unwrap();
        let full = w
            .iter()
            .find(|w| w.path.contains("game.manifest.json.gz"))
            .unwrap_or_else(|| {
                panic!(
                    "no gzipped dump among {:?}",
                    w.iter().map(|w| &w.path).collect::<Vec<_>>()
                )
            });
        assert_eq!(full.encoding, Encoding::Gzip);
        assert_eq!(full.path, "manifests/7.1.0/game.manifest.json.gz");
        // Exactly one form per run: the `.gz` takes the plain name rather than
        // appearing beside it.
        assert!(
            !gz_root.join("manifests/7.1.0/game.manifest.json").exists(),
            "both forms were written"
        );

        // The recorded `uncompressed_bytes` is what the threshold was compared
        // against, not what landed on disk.
        let on_disk = std::fs::metadata(gz_root.join(&full.path)).unwrap().len();
        assert!(
            on_disk < full.uncompressed_bytes,
            "gzip should shrink it: {on_disk} vs {}",
            full.uncompressed_bytes
        );

        let raw = std::fs::read(gz_root.join(&full.path)).unwrap();
        let back = hash::gunzip(&raw).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&back).unwrap();
        assert_eq!(v["entries"].as_array().unwrap().len(), 3);
        assert_eq!(v["schema_version"], 1);
        // The plain and gzipped forms decompress to the same document, which is
        // what makes `checksums.json` able to carry one digest for both.
        let plain =
            std::fs::read_to_string(plain_root.join("manifests/7.1.0/game.manifest.json"))
                .unwrap();
        assert_eq!(hash::md5_hex(plain.as_bytes()), hash::md5_hex(&back));

        let _ = std::fs::remove_dir_all(&plain_root);
        let _ = std::fs::remove_dir_all(&gz_root);
    }

    /// The sidecar is what makes a gzipped dump readable, so it is always plain.
    #[test]
    fn the_sidecar_is_always_plain_even_when_the_dump_is_gzipped() {
        let root = tmp("tsv");
        let mut c = captured();
        let w = write(&root, &mut c, 0, ManifestDetail::Full).unwrap();
        let tsv = w.iter().find(|w| w.path.ends_with(".tsv")).unwrap();
        assert_eq!(tsv.encoding, Encoding::Identity);
        assert!(!tsv.path.ends_with(".gz"));
        let text = std::fs::read_to_string(root.join(&tsv.path)).unwrap();
        assert!(text.starts_with("path\tsize\tmd5\tnchunks\tis_folder\n"), "{text}");
        assert!(!text.contains('\r'), "LF endings only");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_sidecar_is_sorted_by_path_so_two_tags_diff_cleanly() {
        let rows = paths_tsv(&captured().entries);
        let lines: Vec<&str> = rows.lines().skip(1).collect();
        assert_eq!(
            lines,
            vec![
                "GenshinImpact_Data/a.txt\t10\tmd5-GenshinImpact_Data/a.txt\t1\tfalse",
                "GenshinImpact_Data/b.txt\t20\tmd5-GenshinImpact_Data/b.txt\t2\tfalse",
                "GenshinImpact_Data/dir\t0\tmd5-GenshinImpact_Data/dir\t0\ttrue",
            ]
        );
        // The decode order was b, a, dir; the sidecar is not.
        assert_ne!(captured().entries[0].file, "GenshinImpact_Data/a.txt");
    }

    #[test]
    fn a_manifest_with_no_entries_still_produces_a_header() {
        assert_eq!(
            paths_tsv(&[]),
            "path\tsize\tmd5\tnchunks\tis_folder\n"
        );
    }

    /// `--manifest-detail paths-only` was a flag that did nothing until now.
    /// The archive, the sidecar and the meta document all still land — a
    /// sidecar with no archive beside it describes nothing checkable — and only
    /// the multi-megabyte decode is dropped, with the omission recorded.
    #[test]
    fn paths_only_drops_the_decode_and_records_that_it_did() {
        let root = tmp("paths-only");
        let mut c = captured();
        let w = write(&root, &mut c, 1_048_576, ManifestDetail::PathsOnly).unwrap();
        let paths: Vec<&str> = w.iter().map(|w| w.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "manifests/7.1.0/game.manifest.zst",
                "manifests/7.1.0/game.paths.tsv",
                "manifests/7.1.0/game.manifest.meta.json",
            ]
        );
        assert!(!root.join(full_prefix("7.1.0", "game") + ".json").exists());
        assert!(root.join(archive_rel("7.1.0", "game")).exists(), "the archive stays");
        assert!(root.join(paths_rel("7.1.0", "game")).exists(), "the sidecar stays");

        let meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(paths[2])).unwrap()).unwrap();
        assert_eq!(
            meta["full_decode_written"], false,
            "recorded, so it cannot read as damage"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_full_detail_records_that_the_decode_was_written() {
        let root = tmp("detail-full");
        let mut c = captured();
        write(&root, &mut c, 1_048_576, ManifestDetail::Full).unwrap();
        let meta: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(meta_rel("7.1.0", "game"))).unwrap(),
        )
        .unwrap();
        assert_eq!(meta["full_decode_written"], true);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_full_decode_carries_every_entry_verbatim_including_field_six() {
        let root = tmp("full");
        let mut c = captured();
        write(&root, &mut c, 10_000_000, ManifestDetail::Full).unwrap();
        let v: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(format!("{}.json", full_prefix("7.1.0", "game"))))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(v["tag"], "7.1.0");
        assert_eq!(v["build_id"], "sMXGW2ll3Fuu");
        assert_eq!(v["matching_field"], "game");
        let e = &v["entries"][0];
        assert!(e["chunks"][0].get("unknown_field6").is_some(), "field 6 is preserved");
        assert_eq!(v["entries"].as_array().unwrap().len(), 3);
        let _ = std::fs::remove_dir_all(&root);
    }
}
