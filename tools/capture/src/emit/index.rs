//! The index documents: `MANIFEST.json`, `checksums.json`, and the file digest.
//!
//! **M8.** The chunk index is the join between every `(tag, file, chunk)` triple
//! and the single blob that satisfies all of them, including which of the three
//! retention sets each role belongs to.
//!
//! ## Which list is complete
//!
//! `verify/checksums.json` is the complete list of what the artifact contains:
//! it is written last and covers every regular file under the root except itself.
//! `MANIFEST.json → files[]` cannot be, because `MANIFEST.json` cannot list
//! itself and cannot know the digest of a `README.md` or `checksums.json` that
//! has not been written yet — writing those first would mean they could not
//! render `MANIFEST.json`. So `files[]` covers everything written before it, and
//! a reader who wants "is anything missing?" reads `checksums.json`; a reader who
//! wants "what did this run produce?" reads `files[]`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::selfcheck;

use serde::{Deserialize, Serialize};

use crate::hash;

/// Everything `build_manifest` needs, as one argument.
pub struct ManifestInput<'a> {
    /// When the run started, for `capture.captured_at`.
    pub started: chrono::DateTime<chrono::Utc>,
    /// The region.
    pub biz: &'a crate::biz::Biz,
    /// The bases actually used, which may be `--hyp-base` overrides.
    pub endpoints: &'a crate::biz::Endpoints<'a>,
    /// The resolved pair.
    pub versions: &'a crate::config::VersionPair,
    /// `main.diff_tags` as served.
    pub diff_tags: &'a [String],
    /// Audio codes written into the scan file.
    pub audio: &'a [String],
    /// The fields that were scored.
    pub fields: &'a [String],
    /// The `--gzip-threshold` this run used, recorded in `MANIFEST.json`.
    pub gzip_threshold: u64,
    /// The diffs, in the same order as `fields`.
    pub diffs: &'a [crate::manifest::diff::FieldDiff],
    /// The selection.
    pub selection: &'a crate::select::Selection,
    /// The chunk index.
    pub index: &'a crate::blobs::ChunkIndex,
    /// Bytes the CDN sent.
    pub bytes_downloaded: u64,
    /// The command line.
    pub argv: &'a [String],
    /// Whether the run finished.
    pub complete: bool,
}

/// Build `MANIFEST.json`.
///
/// Every field is something the run already knew; nothing here recomputes a
/// measurement, so the document cannot disagree with the tree it describes.
///
/// `files` is a separate argument rather than a field of [`ManifestInput`]
/// because it is the one input the caller cannot supply: it comes from the
/// close-out's full-tree digest pass, which has to run after every other
/// document exists. As a struct field it invited a caller to pass `&[]` and get
/// a document that lied, which is exactly what happened.
///
/// It used to come back empty, with `totals.bytes_on_disk: 0`, and be overwritten
/// by the caller afterwards. The claim this doc makes was false for precisely
/// those two fields; both are now derived here, so the returned document is
/// whole and there is no second step that can be skipped or reordered.
pub fn build_manifest(i: &ManifestInput<'_>, files: &[FileEntry]) -> Result<Manifest, String> {
    let id = i.started.format("%Y%m%dT%H%M%SZ").to_string();
    let manifest_entries: BTreeMap<String, u64> = i
        .fields
        .iter()
        .zip(i.diffs)
        .map(|(f, d)| {
            // `common + added`: the paths that exist on the new side, which is
            // exactly the candidate set this field contributed. A `removed` path
            // is not a candidate — there is nothing to capture on the new side.
            (f.clone(), d.counts.common + d.counts.added)
        })
        .collect();
    let matrix: Vec<ManifestMatrixRow> = i
        .selection
        .matrix
        .iter()
        .map(|row| {
            let admission = row
                .capture_id
                .as_ref()
                .and_then(|cid| i.selection.admitted.iter().find(|a| &a.capture_id == cid));
            ManifestMatrixRow {
                class: row.class.id().to_string(),
                status: row.status.id().to_string(),
                capture_id: row.capture_id.clone(),
                path: row.path.clone(),
                matching_field: admission.map(|a| a.matching_field.clone()),
                covered_by: row.covered_by.clone(),
                also_satisfies: admission.filter(|a| !a.also_satisfies.is_empty()).map(|a| {
                    a.also_satisfies
                        .iter()
                        .map(|c| c.id().to_string())
                        .collect()
                }),
                reason: row.reason.clone(),
                candidates_considered: Some(row.candidates_considered),
            }
        })
        .collect();
    let totals_by_set: BTreeMap<String, u64> = i
        .index
        .sets
        .iter()
        .map(|(k, v)| (k.clone(), v.len() as u64))
        .collect();
    Ok(Manifest {
        schema_version: crate::SCHEMA_VERSION,
        tool: Tool {
            name: "gircap".to_string(),
            version: crate::VERSION.to_string(),
        },
        capture: Capture {
            id,
            captured_at: i.started.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            host_os: std::env::consts::OS.to_string(),
            tool_argv: i.argv.to_vec(),
            complete: i.complete,
        },
        biz: Biz_ {
            id: i.biz.as_str().to_string(),
            launcher_id: i.biz.launcher_id.to_string(),
            game_id: i.biz.game_id.to_string(),
            channel: i.biz.channel.to_string(),
            sub_channel: i.biz.sub_channel.to_string(),
            cps: i.biz.cps.to_string(),
            hyp_base: i.endpoints.hyp.to_string(),
            sophon_base: i.endpoints.sophon.to_string(),
        },
        versions: Versions {
            new: Build {
                tag: i.versions.new.tag.clone(),
                build_id: i.versions.new.build_id.clone(),
                source: i.versions.new.source.clone(),
            },
            old: Build {
                tag: i.versions.old.tag.clone(),
                build_id: i.versions.old.build_id.clone(),
                source: i.versions.old.source.clone(),
            },
            diff_tags: i.diff_tags.to_vec(),
        },
        field: crate::manifest::MANIFEST_FIELD.to_string(),
        audio: i.audio.to_vec(),
        gzip_threshold: i.gzip_threshold,
        selection_pool: SelectionPool {
            fields: i.fields.to_vec(),
            manifest_entries,
            note: "--audio widens this pool; captures may come from any listed field".to_string(),
        },
        totals: Totals {
            files_captured: i.selection.admitted.len() as u64,
            chunks_blobs: i
                .index
                .blobs
                .iter()
                .filter(|b| b.props.stored_path.is_some())
                .count() as u64,
            bytes_downloaded: i.bytes_downloaded,
            bytes_on_disk: files.iter().map(|f| f.bytes).sum(),
        },
        matrix,
        files: files.to_vec(),
        totals_by_set,
    })
}

/// One file under the artifact root, as both index documents record it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]

pub struct FileEntry {
    /// Artifact-relative, `/`-separated.
    pub path: String,
    /// How many bytes.
    pub bytes: u64,
    /// Lowercase hex MD5, for a comparison with a manifest's whole-file digest.
    pub md5: String,
    /// Lowercase hex SHA-256, for a reader with only a stock tool.
    pub sha256: String,
    /// `identity` or `gzip`. Recorded rather than inferred from the name, so a
    /// reader never has to guess which digest covers which bytes.
    pub encoding: crate::hash::Encoding,
}

/// Digest one file, streaming so a 22 MB `.blk` does not have to be resident.
///
/// **A gzipped document is digested as the document, not as the file that happens
/// to hold it.** `Encoding::Gzip` says so in its own doc comment and
/// `encoding_of` exists precisely to decide this - so ignoring `encoding` here
/// meant `checksums.json` recorded the *compressed* digest while promising the
/// decompressed one, and `MANIFEST.json → files[]` did the same. A reader who
/// gunzipped the file and ran `md5sum` on the result got a different answer than
/// either document gave them.
///
/// Nothing in the suite caught it because A16 re-hashes with the same function
/// that wrote the digests, so both sides agreed on the wrong answer. The test
/// that catches it is in `tests/schema.rs`, and it is the reason that file reads
/// the bytes back off disk rather than trusting the document.
///
/// Both digests come from **one pass** — the alternative, hashing the file
/// twice, reads every blob twice, which on a 90 MB artifact is the difference
/// between a capture that feels instant and one that does not. The writer is
/// [`hash::DualDigest`] rather than a `Tee` defined here: it used to be one, and
/// the blob path re-derived the same thing because it could not reach this one.
pub fn digest_file(path: &Path, encoding: crate::hash::Encoding) -> std::io::Result<FileEntry> {
    let mut file = std::fs::File::open(path)?;
    let mut d = hash::DualDigest::new();
    match encoding {
        crate::hash::Encoding::Identity => std::io::copy(&mut file, &mut d)?,
        // Streamed rather than buffered, for the same reason as above: a
        // full manifest decode is several megabytes.
        crate::hash::Encoding::Gzip => {
            let mut gz = flate2::read::GzDecoder::new(file);
            std::io::copy(&mut gz, &mut d)?
        }
    };
    let d = d.finish();
    Ok(FileEntry {
        path: String::new(),
        bytes: d.bytes,
        md5: d.md5,
        sha256: d.sha256,
        encoding,
    })
}

/// Every regular file under `root`, recursively, as artifact-relative paths.
///
/// Sorted, because a list that depends on the filesystem's enumeration order
/// makes two identical artifacts produce different documents — and this tool's
/// whole claim is that the same world produces the same bundle.
pub fn walk(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    fn go(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
            .map(|e| e.map(|e| e.path()))
            .collect::<Result<_, _>>()?;
        entries.sort();
        for e in entries {
            if e.is_dir() {
                go(&e, root, out)?;
            } else if e.is_file() {
                // Unreachable in practice — `read_dir` builds its entries by
                // joining the rooted path it was given — and an `io::Error`
                // anyway, matching every other function here. A `walk` that
                // panics is a `walk` that cannot report a junction or a
                // dangling symlink it did not expect.
                let rel = e.strip_prefix(root).map_err(|_| {
                    std::io::Error::other(format!(
                        "{} is not under the artifact root {}",
                        e.display(),
                        root.display()
                    ))
                })?;
                out.push(rel.to_path_buf());
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    go(root, root, &mut out)?;
    Ok(out)
}

/// The `/`-separated form of an artifact-relative path.
///
/// Always `/`, even on Windows, because these documents are read on other
/// machines and a reader that has to know the writer's separator cannot be
/// checked by anyone else.
pub fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// `verify/checksums.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checksums {
    /// The artifact schema version.
    pub schema_version: u32,
    /// Which digest `sha256` is, stated so a reader need not assume.
    pub algorithm: String,
    /// Every file, sorted by path, excluding this document itself.
    pub entries: Vec<FileEntry>,
    /// How many.
    pub file_count: u64,
    /// `Σ bytes`.
    pub total_bytes: u64,
}

impl Checksums {
    /// Digest everything under `root`, except `checksums.json`.
    ///
    /// "Except itself" is not a convenience: a document that hashes itself has
    /// no fixed point, so the one file that cannot be in the list is the one that
    /// is the list.
    pub fn scan(root: &Path) -> std::io::Result<Checksums> {
        Checksums::scan_reusing(root, &[], &[])
    }

    /// Digest everything under `root`, except `checksums.json`, **reusing**
    /// `known`'s digests for the paths it already covers.
    ///
    /// This is how a capture close-out goes from four full-tree digest passes to
    /// two. It walks the tree every time — that is a `read_dir`, not a hash —
    /// and only reads and digests a file when it is not in `known`, or when it
    /// is named in `rewritten`.
    ///
    /// ## The contract, and why it is safe
    ///
    /// A digest is reused for a path `p` only when `p ∈ known`, `p ∉ rewritten`,
    /// and no `p` was written since `known` was taken. Every call site below
    /// meets that by construction: the writes between one call and the next are
    /// the ones it passes in `rewritten`.
    ///
    /// Nothing is assumed about files that *appear* — a path on disk that is not
    /// in `known` is simply digested, because the walk is what decides membership,
    /// not the caller's list. What the caller must get right is `rewritten`:
    /// **naming a file that changed but is not listed would produce a document
    /// that claims a digest for bytes that are no longer there.**
    ///
    /// `A16` is unaffected and remains the reason this is safe to do at all:
    /// [`Checksums::a16`] re-reads and re-digests every listed file from disk and
    /// compares, so a wrong reuse here is *caught* rather than published.
    pub fn scan_reusing(
        root: &Path,
        known: &[FileEntry],
        rewritten: &[&str],
    ) -> std::io::Result<Checksums> {
        const SELF: &str = "verify/checksums.json";
        let known: std::collections::BTreeMap<&str, &FileEntry> =
            known.iter().map(|e| (e.path.as_str(), e)).collect();
        let mut entries = Vec::new();
        for p in walk(root)? {
            let r = rel(root, &p);
            if r == SELF {
                continue;
            }
            if !rewritten.contains(&r.as_str())
                && let Some(e) = known.get(r.as_str())
            {
                entries.push((*e).clone());
                continue;
            }
            let mut e = digest_file(&root.join(&p), encoding_of(&r))?;
            e.path = r;
            entries.push(e);
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Checksums {
            schema_version: crate::SCHEMA_VERSION,
            algorithm: "sha256".to_string(),
            file_count: entries.len() as u64,
            total_bytes: entries.iter().map(|e| e.bytes).sum::<u64>(),
            entries,
        })
    }

    /// A16: every file under the root is listed, and re-hashing matches.
    ///
    /// Both halves, because a list that omits a file is a silent gap while a list
    /// whose digests are stale is a lie, and only checking one leaves the other.
    ///
    /// The first finding only. A capture wants one row and stops there;
    /// `verify` wants them all and calls [`Checksums::findings`] instead.
    ///
    /// **The passing detail names the file count and not the byte total**, and
    /// that is forced rather than chosen. This row is recorded *inside*
    /// `selfcheck.json`, which is one of the files `total_bytes` sums — so the
    /// last version of that number to be written is the one in `checksums.json`,
    /// and any total written here is by construction a total from an earlier,
    /// smaller tree. `file_count` is safe because no file appears or disappears
    /// between the two passes; a byte count is not. The total is one field read
    /// away in the document it describes.
    pub fn a16(&self, root: &Path) -> (bool, String) {
        match self.findings(root).into_iter().next() {
            Some(f) => (false, f),
            None => (
                true,
                format!("{} files, every digest re-hashed and matched", self.file_count),
            ),
        }
    }

    /// Every A16 failure, in a stable order.
    ///
    /// Exhaustive on purpose. A capture reports the first because it stops at the
    /// first anyway; a *verifier* that reported only the first would send an
    /// operator round a 95-file bundle once per round trip, and the second
    /// finding is usually in the same directory as the first.
    pub fn findings(&self, root: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let on_disk: std::collections::BTreeSet<String> = match walk(root) {
            Ok(v) => v
                .into_iter()
                .map(|p| rel(root, &p))
                .filter(|r| r != "verify/checksums.json")
                .collect(),
            Err(e) => {
                out.push(format!("walk {}: {e}", root.display()));
                return out;
            }
        };
        let listed: std::collections::BTreeSet<String> =
            self.entries.iter().map(|e| e.path.clone()).collect();
        for extra in on_disk.difference(&listed) {
            out.push(format!("{extra} exists but is not in checksums.json"));
        }
        for ghost in listed.difference(&on_disk) {
            out.push(format!("{ghost} is listed but does not exist"));
        }
        if self.file_count as usize != self.entries.len() {
            out.push(format!(
                "file_count {} but {} entries",
                self.file_count,
                self.entries.len()
            ));
        }
        if self.total_bytes != self.entries.iter().map(|e| e.bytes).sum::<u64>() {
            out.push("total_bytes disagrees with the entries".into());
        }
        // Only re-hash a file that still exists: a ghost above already named it,
        // and digesting it would report the same problem a second way.
        for e in &self.entries {
            if !on_disk.contains(&e.path) {
                continue;
            }
            match digest_file(&root.join(&e.path), e.encoding) {
                Ok(got) if got.md5 == e.md5 && got.sha256 == e.sha256 && got.bytes == e.bytes => {}
                Ok(got) => out.push(format!(
                    "{}: listed {} B md5 {}, on disk {} B md5 {}",
                    e.path, e.bytes, e.md5, got.bytes, got.md5
                )),
                Err(io) => out.push(format!("{}: {io}", e.path)),
            }
        }
        out
    }
}

/// The encoding a stored document is in, decided by its extension.
///
/// `hash::should_gzip` decides what gets gzipped; this decides how to *read*
/// what was written, and the two must agree or a reader would digest the wrong
/// bytes.
fn encoding_of(rel_path: &str) -> crate::hash::Encoding {
    if rel_path.ends_with(".gz") {
        crate::hash::Encoding::Gzip
    } else {
        crate::hash::Encoding::Identity
    }
}

/// The artifact schema's per-file entry, reused for `MANIFEST.json → files[]`.
pub type ManifestFile = FileEntry;

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("gircap-m8-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("scratch");
        p
    }

    #[test]
    fn digesting_a_file_matches_hashing_its_bytes() {
        let root = scratch("digest");
        let body = b"the quick brown fox ".repeat(500);
        std::fs::write(root.join("a.bin"), &body).unwrap();
        let e = digest_file(&root.join("a.bin"), crate::hash::Encoding::Identity).unwrap();
        assert_eq!(e.bytes, body.len() as u64);
        assert_eq!(e.md5, hash::md5_hex(&body));
        assert_eq!(e.sha256, hash::sha256_hex(&body));
        assert_eq!(e.md5.len(), 32);
        assert_eq!(e.sha256.len(), 64);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The digest is streamed in 64 KiB blocks, so a file that is an exact
    /// multiple of the block size has to come out the same as one that is not.
    #[test]
    fn a_file_exactly_one_block_long_digests_correctly() {
        let root = scratch("block");
        let body = vec![7_u8; 64 * 1024];
        std::fs::write(root.join("a.bin"), &body).unwrap();
        let e = digest_file(&root.join("a.bin"), crate::hash::Encoding::Identity).unwrap();
        assert_eq!(e.md5, hash::md5_hex(&body));
        assert_eq!(e.bytes, 65_536);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_walk_is_sorted_and_recursive() {
        let root = scratch("walk");
        std::fs::create_dir_all(root.join("game/sub")).unwrap();
        std::fs::write(root.join("game/b.txt"), b"b").unwrap();
        std::fs::write(root.join("game/sub/a.txt"), b"a").unwrap();
        std::fs::write(root.join("z.txt"), b"z").unwrap();
        let paths: Vec<String> = walk(&root).unwrap().iter().map(|p| rel(&root, p)).collect();
        assert_eq!(
            paths,
            vec!["game/b.txt", "game/sub/a.txt", "z.txt"],
            "sorted, and a directory comes before its contents' siblings deterministically"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn relative_paths_use_slashes_even_on_windows() {
        let root = scratch("slashes");
        let p = root.join("a").join("b").join("c.txt");
        assert_eq!(rel(&root, &p), "a/b/c.txt");
    }

    #[test]
    fn checksums_covers_every_file_except_itself() {
        let root = scratch("sums");
        std::fs::write(root.join("a.bin"), b"a").unwrap();
        std::fs::create_dir_all(root.join("verify")).unwrap();
        std::fs::write(root.join("verify/selfcheck.json"), b"{}").unwrap();
        let c = Checksums::scan(&root).unwrap();
        let paths: Vec<&str> = c.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["a.bin", "verify/selfcheck.json"]);
        assert_eq!(c.file_count, 2);
        assert_eq!(
            c.total_bytes, 3,
            "1 B of a.bin plus 2 B of the selfcheck stub"
        );
        assert_eq!(c.algorithm, "sha256");
        assert_eq!(c.schema_version, crate::SCHEMA_VERSION);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A16 is green on an untouched tree.
    #[test]
    fn a16_is_green_on_an_untouched_tree() {
        let root = scratch("a16ok");
        std::fs::write(root.join("a.bin"), b"a").unwrap();
        std::fs::create_dir_all(root.join("verify")).unwrap();
        let c = Checksums::scan(&root).unwrap();
        std::fs::write(root.join("verify/checksums.json"), b"{}").unwrap();
        let (ok, detail) = c.a16(&root);
        assert!(ok, "{detail}");
        assert!(detail.contains("re-hashed"), "{detail}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Reusing a digest must produce **exactly** what re-hashing would have.
    ///
    /// This is the whole safety argument for `scan_reusing`, and it is only true
    /// if the caller names every file it rewrote. Three cases, because the ways
    /// it can go wrong are different: nothing changed, the named file changed,
    /// and a file appeared that the seed never saw.
    #[test]
    fn reusing_digests_gives_the_same_document_as_hashing_everything() {
        let root = scratch("reuse");
        std::fs::create_dir_all(root.join("verify")).unwrap();
        std::fs::write(root.join("a.bin"), b"aaaa").unwrap();
        std::fs::write(root.join("keep.bin"), b"keep").unwrap();

        // Nothing changed: `rewritten` empty.
        let seeded = Checksums::scan(&root).unwrap();
        let reused = Checksums::scan_reusing(&root, &seeded.entries, &[]).unwrap();
        assert_eq!(reused, Checksums::scan(&root).unwrap());

        // The named file changed. This is the close-out's second
        // `selfcheck.json`, and the only reuse that is ever done.
        std::fs::write(root.join("a.bin"), b"aaaaaa").unwrap();
        let reused = Checksums::scan_reusing(&root, &seeded.entries, &["a.bin"]).unwrap();
        let fresh = Checksums::scan(&root).unwrap();
        assert_eq!(reused, fresh, "the rewritten file's digest must be fresh");
        assert_ne!(
            reused.entries,
            seeded.entries,
            "and it really did change"
        );

        // A file appeared that the seed never saw: the walk decides membership,
        // not the caller's list, so it is digested rather than missed. The seed
        // is taken from the *current* tree — reusing the stale one would be the
        // wrong-`rewritten` mistake, which the next test covers.
        let current = Checksums::scan(&root).unwrap();
        std::fs::write(root.join("surprise.bin"), b"new").unwrap();
        let reused = Checksums::scan_reusing(&root, &current.entries, &[]).unwrap();
        assert_eq!(reused, Checksums::scan(&root).unwrap());
        assert!(
            reused.entries.iter().any(|e| e.path == "surprise.bin"),
            "a new file must be digested, not dropped"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// What a *wrong* `rewritten` list costs, and what catches it.
    ///
    /// This is the failure mode `scan_reusing` makes possible: a file changed
    /// and the caller forgot to name it, so the stale digest is published. It
    /// must still be caught, because A16 re-reads from disk.
    #[test]
    fn a_stale_reuse_is_still_caught_by_a16() {
        let root = scratch("stalereuse");
        std::fs::write(root.join("a.bin"), b"aaaa").unwrap();
        let seeded = Checksums::scan(&root).unwrap();
        std::fs::write(root.join("a.bin"), b"aaaaaa").unwrap();

        // The mistake: reusing `a.bin`'s digest after changing it.
        let wrong = Checksums::scan_reusing(&root, &seeded.entries, &[]).unwrap();
        assert_eq!(wrong, seeded, "nothing was re-digested, so it is stale");
        let (ok, detail) = wrong.a16(&root);
        assert!(!ok, "and A16 must still see it");
        assert!(detail.contains("a.bin"), "{detail}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A16's two halves. A file added after the scan is a gap; a file changed
    /// after the scan is a lie. Both have to be caught, and they are different
    /// failures with different fixes.
    #[test]
    fn a16_catches_a_file_added_after_the_scan_and_a_file_changed_after_it() {
        let root = scratch("a16bad");
        std::fs::write(root.join("a.bin"), b"a").unwrap();
        let c = Checksums::scan(&root).unwrap();

        std::fs::write(root.join("surprise.bin"), b"s").unwrap();
        let (ok, detail) = c.a16(&root);
        assert!(!ok);
        assert!(
            detail.contains("surprise.bin") && detail.contains("not in"),
            "{detail}"
        );

        std::fs::remove_file(root.join("surprise.bin")).unwrap();
        std::fs::write(root.join("a.bin"), b"CHANGED").unwrap();
        let (ok, detail) = c.a16(&root);
        assert!(!ok);
        assert!(
            detail.contains("a.bin") && detail.contains("on disk"),
            "{detail}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a16_catches_a_ghost_entry_and_wrong_totals() {
        let root = scratch("a16ghost");
        std::fs::write(root.join("a.bin"), b"a").unwrap();
        let mut c = Checksums::scan(&root).unwrap();
        c.entries.push(FileEntry {
            path: "never-written.bin".into(),
            bytes: 1,
            md5: hash::md5_hex(b"x"),
            sha256: hash::sha256_hex(b"x"),
            encoding: crate::hash::Encoding::Identity,
        });
        c.file_count = c.entries.len() as u64;
        let (ok, detail) = c.a16(&root);
        assert!(!ok);
        assert!(
            detail.contains("never-written.bin") && detail.contains("does not exist"),
            "{detail}"
        );

        let root2 = scratch("a16total");
        std::fs::write(root2.join("a.bin"), b"a").unwrap();
        let mut c2 = Checksums::scan(&root2).unwrap();
        c2.total_bytes += 1;
        let (ok, detail) = c2.a16(&root2);
        assert!(!ok);
        assert!(detail.contains("total_bytes"), "{detail}");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&root2);
    }

    #[test]
    fn a_gzipped_file_is_recorded_as_gzip_so_a_reader_knows_which_digest() {
        assert_eq!(encoding_of("a.bin"), crate::hash::Encoding::Identity);
        assert_eq!(encoding_of("a.json.gz"), crate::hash::Encoding::Gzip);
        assert_eq!(encoding_of("a.json"), crate::hash::Encoding::Identity);
        let e = &c_of("x.json.gz");
        assert_eq!(e.encoding, crate::hash::Encoding::Gzip);
        assert_eq!(serde_json::to_value(e).unwrap()["encoding"], "gzip");
    }

    /// A synthetic entry, for the assertions that need a file that was never written.
    fn c_of(rel_path: &str) -> FileEntry {
        FileEntry {
            path: rel_path.to_string(),
            bytes: 4,
            md5: hash::md5_hex(b"body"),
            sha256: hash::sha256_hex(b"body"),
            encoding: encoding_of(rel_path),
        }
    }

    /// Everything a `ManifestInput` borrows, owned, so a test can build one
    /// without a lifetime puzzle.
    ///
    /// Spelled out field by field rather than derived, on purpose: a fixture
    /// assembled from `Default` would quietly follow every default a future edit
    /// introduces, and the point of these tests is to pin the document's shape
    /// rather than let it drift.
    struct Fixture {
        versions: crate::config::VersionPair,
        selection: crate::select::Selection,
        index: crate::blobs::ChunkIndex,
        biz: crate::biz::Biz,
        endpoints: crate::biz::Endpoints<'static>,
        diff_tags: Vec<String>,
        audio: Vec<String>,
        fields: Vec<String>,
        argv: Vec<String>,
        diffs: Vec<crate::manifest::diff::FieldDiff>,
    }

    impl Fixture {
        fn new() -> Self {
            Fixture {
                versions: crate::config::VersionPair {
                    old: crate::config::VersionRef {
                        tag: "7.0.0".to_string(),
                        build_id: "oldbuild".to_string(),
                        source: "diff_tags[0]".to_string(),
                    },
                    new: crate::config::VersionRef {
                        tag: "7.1.0".to_string(),
                        build_id: "newbuild".to_string(),
                        source: "main.tag".to_string(),
                    },
                    diff_tags: vec!["7.1.1".to_string(), "7.1.2".to_string()],
                },
                selection: crate::select::Selection {
                    admitted: Vec::new(),
                    rejected: Vec::new(),
                    forced_skipped: Vec::new(),
                    matrix: Vec::new(),
                    bytes_admitted: 0,
                    budget: crate::select::Budget::capped(10_000),
                },
                index: crate::blobs::ChunkIndex {
                    schema_version: crate::SCHEMA_VERSION,
                    blobs: Vec::new(),
                    sets: BTreeMap::new(),
                },
                biz: crate::biz::Biz::resolve(crate::cli::Biz::Hk4eGlobal),
                endpoints: crate::biz::Endpoints::owned(
                    "https://hyp.example".to_string(),
                    "https://sophon.example".to_string(),
                ),
                // Two tags as served, neither of them either half of the pair, so
                // the document cannot source `diff_tags` out of `versions`.
                diff_tags: vec!["7.1.1".to_string(), "7.1.2".to_string()],
                audio: vec!["audio_1".to_string()],
                fields: vec!["game".to_string()],
                argv: vec!["gircap".to_string(), "capture".to_string()],
                // `common + added` is what `selection_pool.manifest_entries`
                // records, so a fixture with real counts is what pins that
                // arithmetic. Zeros would pass either way and so would say
                // nothing about it.
                diffs: vec![crate::manifest::diff::FieldDiff {
                    field: "game".to_string(),
                    counts: crate::manifest::diff::FieldCounts {
                        common: 40,
                        identical: 25,
                        added: 3,
                        removed: 7,
                    },
                    scores: Vec::new(),
                    rows: Vec::new(),
                    pool: crate::manifest::diff::ReusePool::default(),
                }],
            }
        }

        fn input(&self) -> ManifestInput<'_> {
            ManifestInput {
                // A fixed instant, so `capture.id` and `captured_at` are the same
                // on every run and can be asserted against literally.
                started: chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
                    .expect("fixture instant")
                    .with_timezone(&chrono::Utc),
                biz: &self.biz,
                endpoints: &self.endpoints,
                versions: &self.versions,
                diff_tags: &self.diff_tags,
                audio: &self.audio,
                fields: &self.fields,
                gzip_threshold: 1024,
                diffs: &self.diffs,
                selection: &self.selection,
                index: &self.index,
                bytes_downloaded: 4_096,
                argv: &self.argv,
                complete: true,
            }
        }
    }

    fn entry(path: &str, bytes: u64) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            bytes,
            md5: "0".repeat(32),
            sha256: "0".repeat(64),
            encoding: hash::Encoding::Identity,
        }
    }

    #[test]
    fn the_manifest_is_whole_when_build_manifest_returns_it() {
        // **The invariant that `files: Vec::new()` broke.** `build_manifest` used
        // to hand back a document with an empty `files[]` and
        // `bytes_on_disk: 0`, leaving the caller to overwrite both afterwards.
        // For that window the document claimed the bundle held no files at all,
        // and its own doc comment claimed every field was a measurement the run
        // already had - false for precisely the two fields that needed the tree
        // walked.
        //
        // So the assertion is deliberately not "the caller filled it in", which
        // is the only thing the old shape could be tested for at all. It is that
        // the returned document is *already* correct, leaving no second step
        // that could be forgotten, skipped, or reordered past something that
        // reads the document.
        let f = Fixture::new();
        // Uneven, and deliberately not summing to a round number: 1 + 2 048 + 33
        // = 2 082, which a hardcoded total would not match by accident.
        let files = vec![entry("a/one.bin", 1), entry("b/two.bin", 2_048), entry("c", 33)];

        let m = build_manifest(&f.input(), &files).expect("manifest");

        assert_eq!(
            m.files.len(),
            files.len(),
            "files[] came back empty or short: the document is not whole on return"
        );
        for (got, want) in m.files.iter().zip(files.iter()) {
            assert_eq!(got.path, want.path);
            assert_eq!(got.bytes, want.bytes);
        }
        // Summed from the listing rather than carried beside it, so the two
        // cannot disagree about how big the bundle is.
        assert_eq!(m.totals.bytes_on_disk, 2_082);
        assert_eq!(
            m.totals.bytes_on_disk,
            m.files.iter().map(|f| f.bytes).sum::<u64>()
        );
    }

    #[test]
    fn an_empty_listing_yields_a_zero_total_rather_than_an_unset_one() {
        // The other half of the same invariant. A run that admitted nothing still
        // produces a document; the question is whether "no files" reads as "zero
        // bytes" or as "not counted yet". Under the old two-phase shape those two
        // were indistinguishable - both were `0` - which is exactly how the bug
        // hid behind a passing assertion.
        let f = Fixture::new();
        let m = build_manifest(&f.input(), &[]).expect("manifest");
        assert!(
            m.files.is_empty(),
            "an empty listing must stay empty, not pick up rows"
        );
        assert_eq!(m.totals.bytes_on_disk, 0);
        assert_eq!(
            m.totals.bytes_downloaded, 4_096,
            "the download total is a separate measurement and must survive"
        );
    }

    #[test]
    fn one_byte_of_the_listing_moves_the_total_by_exactly_one_byte() {
        // Pins that the total is a *sum* - not a constant, and not a count. If a
        // future edit sourced it from somewhere else, the chunk index or
        // `log.bytes_in()` say, this is the test that catches it: the listing
        // and the download total are deliberately different numbers here.
        let f = Fixture::new();
        let one = build_manifest(&f.input(), &[entry("a", 100)])
            .expect("manifest")
            .totals
            .bytes_on_disk;
        let two = build_manifest(&f.input(), &[entry("a", 101)])
            .expect("manifest")
            .totals
            .bytes_on_disk;
        assert_eq!(one, 100);
        assert_eq!(two, 101);
        assert_ne!(
            one, 4_096,
            "the fixture's byte total must not coincide with bytes_downloaded"
        );
    }
}

/// `MANIFEST.json` — the bundle's own index.
///
/// Every field here is something the run already knew. The document exists so a
/// reader can answer "what is this, what does it cover, and is it complete"
/// without parsing `api/` and `diff/`, and so `README.md` has one source to
/// render rather than two that could disagree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// The artifact schema version.
    pub schema_version: u32,
    /// Which tool wrote it.
    pub tool: Tool,
    /// Which run, and whether it finished.
    pub capture: Capture,
    /// Which region.
    pub biz: Biz_,
    /// Which builds.
    pub versions: Versions,
    /// The `matching_field` the captures were drawn from.
    pub field: String,
    /// Audio codes written into the scan file.
    pub audio: Vec<String>,
    /// The `--gzip-threshold` this run used, in bytes.
    ///
    /// Recorded because the flag's own help promises the decision is, and
    /// because a reader holding a bundle cannot otherwise tell *why* one
    /// document is `.gz` and the next is not. `checksums.json` names the
    /// encoding of each file; this is the number that produced those choices,
    /// and without it a re-run with different flags would produce a different
    /// tree with no recorded reason.
    ///
    /// **Required.** It was added after the first bundles were written and
    /// defaulted on read so that `schema_version` could stay `1` — which meant a
    /// bundle with no `gzip_threshold` read back as having used the *default*,
    /// which is a claim about a run nobody recorded. A bundle that predates the
    /// field is stale by definition; say so by refusing it rather than by
    /// inventing its number.
    pub gzip_threshold: u64,
    /// The fields that were scored and diffed.
    pub selection_pool: SelectionPool,
    /// The headline numbers.
    pub totals: Totals,
    /// One row per required class, in admission order.
    pub matrix: Vec<ManifestMatrixRow>,
    /// Every file written before this document.
    pub files: Vec<FileEntry>,
    /// How many blobs each retention set holds.
    pub totals_by_set: BTreeMap<String, u64>,
}

/// Which tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tool {
    /// Always `gircap`.
    pub name: String,
    /// The crate version.
    pub version: String,
}

/// Which run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capture {
    /// The `<capture_id>` directory component, `20261014T101500Z`.
    pub id: String,
    /// When it started.
    pub captured_at: String,
    /// `windows` | `linux` | `macos`.
    pub host_os: String,
    /// The arguments, so a reader can reproduce the request set.
    pub tool_argv: Vec<String>,
    /// False only if the run was interrupted; the selfcheck then says why.
    pub complete: bool,
}

/// Which region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Biz_ {
    /// The `--biz` value.
    pub id: String,
    /// HoYoPlay launcher id.
    pub launcher_id: String,
    /// HoYoPlay game id.
    pub game_id: String,
    /// Channel.
    pub channel: String,
    /// Sub-channel.
    pub sub_channel: String,
    /// CPS name.
    pub cps: String,
    /// The HoYoPlay base used.
    pub hyp_base: String,
    /// The Sophon base used.
    pub sophon_base: String,
}

/// Which builds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Versions {
    /// The build being upgraded *to*.
    pub new: Build,
    /// The build being upgraded *from*.
    pub old: Build,
    /// `main.diff_tags` as served.
    pub diff_tags: Vec<String>,
}

/// One build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    /// The tag.
    pub tag: String,
    /// The Sophon build id.
    pub build_id: String,
    /// Where the tag came from: `main.tag`, `diff_tags[0]`, `--old-tag`, …
    pub source: String,
}

/// The fields that were scored, and how many entries each carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionPool {
    /// The `matching_field`s.
    pub fields: Vec<String>,
    /// Decoded entries per field.
    pub manifest_entries: BTreeMap<String, u64>,
    /// Why the list can be longer than one.
    pub note: String,
}

/// The headline numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totals {
    /// Admitted captures.
    pub files_captured: u64,
    /// Distinct chunk blobs stored.
    pub chunks_blobs: u64,
    /// Bytes the CDN sent.
    pub bytes_downloaded: u64,
    /// Bytes the bundle occupies.
    pub bytes_on_disk: u64,
}

/// One matrix row as `MANIFEST.json` records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestMatrixRow {
    /// The class.
    pub class: String,
    /// `captured` | `covered_by` | `over_budget` | `unavailable`.
    pub status: String,
    /// The capture that answers it, when one does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_id: Option<String>,
    /// The path it answers it with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Which field it came from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matching_field: Option<String>,
    /// The capture that covered it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub covered_by: Option<String>,
    /// Other classes the same capture answers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub also_satisfies: Option<Vec<String>>,
    /// Why it is not captured, when it is not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// How many candidates were considered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates_considered: Option<u64>,
}

impl Manifest {
    /// The `README.md` body: a rendering of this document and the selfcheck.
    ///
    /// Generated, carrying no fact of its own, and safe to delete — which is
    /// exactly why every number in it is quoted from a document rather than
    /// recomputed. A README that disagreed with the manifest would be the one
    /// file a reader trusted.
    pub fn readme(&self, selfcheck: &selfcheck::Selfcheck) -> String {
        let mut s = String::with_capacity(2048);
        s.push_str("# gircap artifact\n\n");
        s.push_str(&format!(
            "- **tool** {} {}\n- **region** `{}`\n- **captured** {} (`{}`, {})\n",
            self.tool.name,
            self.tool.version,
            self.biz.id,
            self.capture.captured_at,
            self.capture.id,
            self.capture.host_os
        ));
        s.push_str(&format!(
            "\n**{}**: `{}` (`{}`) → `{}` (`{}`)",
            if self.capture.complete {
                "complete"
            } else {
                "INCOMPLETE"
            },
            self.versions.old.tag,
            self.versions.old.build_id,
            self.versions.new.tag,
            self.versions.new.build_id
        ));
        s.push_str(&format!(
            "\n\nselfcheck: **{:?}** — {} of {} assertions passed\n",
            selfcheck.result, selfcheck.counts.passed, selfcheck.counts.total
        ));

        s.push_str("\n## Matrix\n\n");
        s.push_str("| class | status | capture | path | field | bytes |\n");
        s.push_str("|---|---|---|---|---|---|\n");
        let sizes: BTreeMap<&str, u64> = self
            .files
            .iter()
            .filter_map(|f| f.path.strip_prefix("game/").map(|p| (p, f.bytes)))
            .collect();
        for row in &self.matrix {
            let bytes = row
                .path
                .as_deref()
                .and_then(|p| sizes.get(p).copied())
                .map(|b| b.to_string())
                .unwrap_or_else(|| "-".into());
            s.push_str(&format!(
                "| `{}` | {} | {} | `{}` | {} | {} |\n",
                row.class,
                row.status,
                row.capture_id.as_deref().unwrap_or("-"),
                row.path.as_deref().unwrap_or("-"),
                row.matching_field.as_deref().unwrap_or("-"),
                bytes
            ));
        }

        s.push_str("\n## Totals\n\n");
        s.push_str(&format!(
            "- {} files captured, {} chunk blobs stored\n- {} B downloaded, {} B on disk\n",
            self.totals.files_captured,
            self.totals.chunks_blobs,
            self.totals.bytes_downloaded,
            self.totals.bytes_on_disk
        ));
        s.push_str(&format!(
            "- selection pool: {}\n",
            self.selection_pool
                .fields
                .iter()
                .map(|f| format!("`{f}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        for (set, n) in &self.totals_by_set {
            s.push_str(&format!("- `{set}`: {n} chunks\n"));
        }

        if !selfcheck.failures().is_empty() {
            s.push_str("\n## Failed assertions\n\n");
            for a in selfcheck.failures() {
                s.push_str(&format!(
                    "- **{}** {} — `{}`: expected `{}`, got `{}`\n",
                    a.id, a.assertion, a.subject, a.expected, a.actual
                ));
            }
        }

        // The flag is spelled, because a reader who types the sentence above it
        // verbatim gets a usage error: `--artifact` is how `verify` is told
        // which directory it is verifying.
        s.push_str(
            "\n## Re-verifying\n\n```\ngircap verify --artifact <this directory>\n```\n\n",
        );
        s.push_str("Offline: no network, no writes. Exit 0 if every assertion still holds.\n");
        s
    }

    /// A17: every required class is present and in one of the three acceptable
    /// states.
    ///
    /// `over_budget` is deliberately **not** acceptable — it means a candidate
    /// existed and the run chose not to pay for it, which is a finding about the
    /// run rather than about the world, so it is reported but does not make the
    /// matrix complete.
    pub fn a17(&self) -> (bool, String) {
        let mut seen: Vec<&str> = Vec::new();
        for row in &self.matrix {
            if !matches!(
                row.status.as_str(),
                "captured" | "covered_by" | "unavailable"
            ) {
                return (
                    false,
                    format!(
                        "{} is `{}`, which is not one of captured/covered_by/unavailable",
                        row.class, row.status
                    ),
                );
            }
            if seen.contains(&row.class.as_str()) {
                return (false, format!("{} appears twice", row.class));
            }
            seen.push(&row.class);
        }
        (
            true,
            format!(
                "{} classes, each present and in an acceptable state",
                seen.len()
            ),
        )
    }
}
