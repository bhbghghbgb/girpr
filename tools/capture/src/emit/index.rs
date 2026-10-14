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

use serde::Serialize;

use crate::hash;

/// One file under the artifact root, as both index documents record it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

/// A writer that feeds both digests from one pass over the bytes.
///
/// The alternative — hashing the file twice — reads every blob twice, which on a
/// 90 MB artifact is the difference between a capture that feels instant and one
/// that does not. `std::io::copy` does the block loop, so there is no hand-rolled
/// buffer here to get the block boundaries wrong.
struct Tee<'a> {
    md5: &'a mut md5::Context,
    sha: &'a mut sha2::Sha256,
}

impl std::io::Write for Tee<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.md5.consume(buf);
        sha2::Digest::update(self.sha, buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Digest one file, streaming so a 22 MB `.blk` does not have to be resident.
pub fn digest_file(path: &Path, encoding: crate::hash::Encoding) -> std::io::Result<FileEntry> {
    let mut file = std::fs::File::open(path)?;
    let mut md5c = md5::Context::new();
    let mut sha = <sha2::Sha256 as sha2::Digest>::new();
    let bytes = {
        let mut tee = Tee {
            md5: &mut md5c,
            sha: &mut sha,
        };
        std::io::copy(&mut file, &mut tee)?
    };
    let to_hex = |b: &[u8]| -> String { b.iter().map(|x| format!("{x:02x}")).collect() };
    Ok(FileEntry {
        path: String::new(),
        bytes,
        md5: to_hex(&md5c.compute().0),
        sha256: to_hex(&sha2::Digest::finalize(sha)),
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
                out.push(e.strip_prefix(root).expect("under the root").to_path_buf());
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
#[derive(Debug, Clone, PartialEq, Serialize)]
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
        const SELF: &str = "verify/checksums.json";
        let mut entries = Vec::new();
        for p in walk(root)? {
            let r = rel(root, &p);
            if r == SELF {
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

    /// The entry for a path, if the document lists it.
    pub fn get(&self, path: &str) -> Option<&FileEntry> {
        self.entries.iter().find(|e| e.path == path)
    }

    /// A16: every file under the root is listed, and re-hashing matches.
    ///
    /// Both halves, because a list that omits a file is a silent gap while a list
    /// whose digests are stale is a lie, and only checking one leaves the other.
    pub fn a16(&self, root: &Path) -> (bool, String) {
        let on_disk: std::collections::BTreeSet<String> = match walk(root) {
            Ok(v) => v
                .into_iter()
                .map(|p| rel(root, &p))
                .filter(|r| r != "verify/checksums.json")
                .collect(),
            Err(e) => return (false, format!("walk {root:?}: {e}")),
        };
        let listed: std::collections::BTreeSet<String> =
            self.entries.iter().map(|e| e.path.clone()).collect();
        if let Some(missing) = on_disk.difference(&listed).next() {
            return (false, format!("{missing} exists but is not in checksums.json"));
        }
        if let Some(ghost) = listed.difference(&on_disk).next() {
            return (false, format!("{ghost} is listed but does not exist"));
        }
        if self.file_count as usize != self.entries.len() {
            return (false, format!("file_count {} but {} entries", self.file_count, self.entries.len()));
        }
        if self.total_bytes != self.entries.iter().map(|e| e.bytes).sum::<u64>() {
            return (false, "total_bytes disagrees with the entries".into());
        }
        for e in &self.entries {
            match digest_file(&root.join(&e.path), e.encoding) {
                Ok(got) if got.md5 == e.md5 && got.sha256 == e.sha256 && got.bytes == e.bytes => {}
                Ok(got) => {
                    return (
                        false,
                        format!(
                            "{}: listed {} B md5 {}, on disk {} B md5 {}",
                            e.path, e.bytes, e.md5, got.bytes, got.md5
                        ),
                    );
                }
                Err(io) => return (false, format!("{}: {io}", e.path)),
            }
        }
        (
            true,
            format!(
                "{} files, {} B, every digest re-hashed and matched",
                self.file_count, self.total_bytes
            ),
        )
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

/// A convenience for hashing a small in-memory body as if it were a file.
pub fn digest_bytes(rel_path: &str, bytes: &[u8]) -> FileEntry {
    FileEntry {
        path: rel_path.to_string(),
        bytes: bytes.len() as u64,
        md5: hash::md5_hex(bytes),
        sha256: hash::sha256_hex(bytes),
        encoding: encoding_of(rel_path),
    }
}

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
        assert_eq!(c.total_bytes, 3, "1 B of a.bin plus 2 B of the selfcheck stub");
        assert_eq!(c.algorithm, "sha256");
        assert_eq!(c.schema_version, crate::SCHEMA_VERSION);
        assert!(c.get("verify/selfcheck.json").is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

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
        assert!(detail.contains("surprise.bin") && detail.contains("not in"), "{detail}");

        std::fs::remove_file(root.join("surprise.bin")).unwrap();
        std::fs::write(root.join("a.bin"), b"CHANGED").unwrap();
        let (ok, detail) = c.a16(&root);
        assert!(!ok);
        assert!(detail.contains("a.bin") && detail.contains("on disk"), "{detail}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a16_catches_a_ghost_entry_and_wrong_totals() {
        let root = scratch("a16ghost");
        std::fs::write(root.join("a.bin"), b"a").unwrap();
        let mut c = Checksums::scan(&root).unwrap();
        c.entries.push(digest_bytes("never-written.bin", b"x"));
        c.file_count = c.entries.len() as u64;
        let (ok, detail) = c.a16(&root);
        assert!(!ok);
        assert!(detail.contains("never-written.bin") && detail.contains("does not exist"), "{detail}");

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
        let e = digest_bytes("x.json.gz", b"body");
        assert_eq!(e.encoding, crate::hash::Encoding::Gzip);
        assert_eq!(serde_json::to_value(&e).unwrap()["encoding"], "gzip");
    }
}

/// `MANIFEST.json` — the bundle's own index.
///
/// Every field here is something the run already knew. The document exists so a
/// reader can answer "what is this, what does it cover, and is it complete"
/// without parsing `api/` and `diff/`, and so `README.md` has one source to
/// render rather than two that could disagree.
#[derive(Debug, Clone, PartialEq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tool {
    /// Always `gircap`.
    pub name: String,
    /// The crate version.
    pub version: String,
}

/// Which run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Versions {
    /// The build being upgraded *to*.
    pub new: Build,
    /// The build being upgraded *from*.
    pub old: Build,
    /// `main.diff_tags` as served.
    pub diff_tags: Vec<String>,
}

/// One build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Build {
    /// The tag.
    pub tag: String,
    /// The Sophon build id.
    pub build_id: String,
    /// Where the tag came from: `main.tag`, `diff_tags[0]`, `--old-tag`, …
    pub source: String,
}

/// The fields that were scored, and how many entries each carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectionPool {
    /// The `matching_field`s.
    pub fields: Vec<String>,
    /// Decoded entries per field.
    pub manifest_entries: BTreeMap<String, u64>,
    /// Why the list can be longer than one.
    pub note: String,
}

/// The headline numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
            if self.capture.complete { "complete" } else { "INCOMPLETE" },
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
            .filter_map(|f| {
                f.path
                    .strip_prefix("game/")
                    .map(|p| (p, f.bytes))
            })
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

        s.push_str("\n## Re-verifying\n\n```\ngircap verify <this directory>\n```\n\n");
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
                    format!("{} is `{}`, which is not one of captured/covered_by/unavailable", row.class, row.status),
                );
            }
            if seen.contains(&row.class.as_str()) {
                return (false, format!("{} appears twice", row.class));
            }
            seen.push(&row.class);
        }
        (
            true,
            format!("{} classes, each present and in an acceptable state", seen.len()),
        )
    }
}
