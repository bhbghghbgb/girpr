//! The offline commands: `verify`, `materialize`, `list`.
//!
//! **M9.** No network, ever - these run against a bundle's stored bytes, and a
//! verifier that phoned home to check a bundle would be testing the network
//! rather than the bundle. This is the milestone that makes the artifact
//! trustworthy rather than merely present.
//!
//! ## What `verify` trusts, and in what order
//!
//! The blobs and the layout `chunks/index.json` describes. It does **not** trust
//! `selfcheck.json`, and the order it works in is the argument for that:
//!
//! 1. **A16** - re-hash every file, compare with `verify/checksums.json`. First
//!    because everything after this point assumes it is reading the bytes the
//!    capture wrote; a stale digest list would make every later verdict a
//!    verdict about a different tree.
//! 2. **A6** - re-measure every stored blob, compare with `chunks/index.json`.
//! 3. Rebuild both sides of every capture from the blobs alone (A11), check each
//!    rebuilt digest against the one the patchmap declared (A7/A8), and compare
//!    the rebuilt bytes with `game/` and `expected/` on disk (A18).
//! 4. Re-parse `game/config.ini` against the old tag (A13); re-count the matrix
//!    (A17).
//! 5. Only then: does the recomputation agree with what the capture recorded?
//!
//! Step 5 is last on purpose. A selfcheck that disagrees with a recomputation is
//! a finding *about the recording*, and a reader who met it before step 3 would
//! reasonably go looking for the wrong thing.
//!
//! ## A18, which is not in the spec
//!
//! The spec's seventeen assertions are all about a capture doing its job. None of
//! them asks whether `game/` is what the retained blobs say it is - which is the
//! bundle's central claim, and the one `materialize` exists to test. It gets an
//! id of its own so it cannot be mistaken for one of the seventeen, and so a
//! reader diffing two `selfcheck.json` files sees a row that only a verifier
//! could have written.
//!
//! ## What `verify` cannot re-derive
//!
//! A1-A5, A9, A10, A12 and A14-A15 are statements about the API bodies, the
//! manifests and the diff, and re-running them here would be re-running the
//! capture's own algorithm against the capture's own output. They are counted
//! and named in `covered_by_recording`, never silently dropped.

pub mod materialize;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::assemble::PatchMap;
use crate::blobs::ChunkIndex;
use crate::emit::index::{Checksums, Manifest};
use crate::emit::selfcheck::{Assertion, Selfcheck};
use crate::hash;

/// A18's wording, kept in one place because `verify` and the docs both quote it.
pub const A18: &str =
    "the bytes rebuilt from chunks/ are byte-for-byte the bytes game/ and expected/ hold";

/// The ids [`reverify`] re-derives **and** the capture also recorded.
///
/// The overlap, and the reason it is a `&'static` list rather than a computed
/// one: these are the rows [`check_agreement`] is entitled to compare, so putting
/// an id here silently promotes it from "re-derived" to "re-derived *and* checked
/// against the recording". A18 is re-derived but never recorded - the capture
/// cannot write it, because it is a statement about the bundle rather than about
/// the run - so it is deliberately absent.
pub const REDERIVED: [&str; 7] = ["A16", "A6", "A11", "A7", "A8", "A13", "A17"];

/// A bundle loaded from disk.
///
/// Every document is read back through the same type it was written with, so a
/// schema drift surfaces as a parse error naming the file rather than as a
/// silently missing field.
#[derive(Debug, Clone)]
pub struct Bundle {
    /// The bundle root, as given.
    pub root: PathBuf,
    /// `MANIFEST.json`.
    pub manifest: Manifest,
    /// `chunks/index.json`.
    pub index: ChunkIndex,
    /// `patchmaps/*.json`, keyed by the `capture_id` each document declares.
    pub patchmaps: BTreeMap<String, PatchMap>,
    /// `verify/checksums.json`.
    pub checksums: Checksums,
    /// `verify/selfcheck.json` - the capture's own account of itself.
    pub recorded: Selfcheck,
}

impl Bundle {
    /// Read every document a bundle has to have, naming the first that is absent.
    ///
    /// `MANIFEST.json` first, because the rest are located relative to the schema
    /// version it declares and a directory without one is not a bundle. Saying
    /// "no MANIFEST.json here" beats five separate "no such file" errors.
    pub fn open(root: &Path) -> Result<Bundle, String> {
        if !root.is_dir() {
            return Err(format!(
                "{} is not a directory; --artifact takes the bundle root, the directory \
                 holding MANIFEST.json",
                root.display()
            ));
        }
        let manifest: Manifest = read_json(&root.join("MANIFEST.json"))?;
        let index: ChunkIndex = read_json(&root.join("chunks/index.json"))?;
        let checksums: Checksums = read_json(&root.join("verify/checksums.json"))?;
        let recorded: Selfcheck = read_json(&root.join("verify/selfcheck.json"))?;
        let patchmaps = read_patchmaps(root)?;
        Ok(Bundle { root: root.to_path_buf(), manifest, index, patchmaps, checksums, recorded })
    }

    /// One line naming the bundle, for the stderr narration.
    pub fn describe(&self) -> String {
        let v = &self.manifest.versions;
        format!(
            "{} {} {} ({}) -> {} ({}) - {} blobs, {} patchmaps, recorded {} rows {} failed",
            self.manifest.biz.id,
            v.old.tag,
            v.old.build_id,
            self.manifest.capture.id,
            v.new.tag,
            v.new.build_id,
            self.index.blobs.len(),
            self.patchmaps.len(),
            self.recorded.counts.total,
            self.recorded.counts.failed,
        )
    }
}

/// Read every `patchmaps/*.json`, keyed by the id the document declares.
///
/// Keyed by `capture_id` rather than by filename: the filename is a convenience
/// and the field is the identity, so trusting it would let a renamed file
/// masquerade as a second capture.
fn read_patchmaps(root: &Path) -> Result<BTreeMap<String, PatchMap>, String> {
    let dir = root.join("patchmaps");
    let rd = std::fs::read_dir(&dir).map_err(|e| {
        format!(
            "no patchmaps/ under {}: {e}; a bundle with no patchmaps has no captures to \
             verify",
            root.display()
        )
    })?;
    let mut paths: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    let mut out = BTreeMap::new();
    for p in paths {
        let map: PatchMap = read_json(&p)?;
        out.insert(map.capture_id.clone(), map);
    }
    Ok(out)
}

/// Read one JSON document, naming the file and the reason on failure.
///
/// serde's error already carries the line and column; prefixing it with the path
/// makes a failure inside a 95-file bundle findable without a second pass to
/// work out which file the tool was reading.
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&raw).map_err(|e| format!("{}: {e}", path.display()))
}

/// What re-verification found.
///
/// The rebuilt bytes are kept rather than dropped, because [`materialize`] is the
/// same rebuild with somewhere to put them, and rebuilding twice on a 135 MB
/// bundle costs a minute of the operator's time for nothing.
#[derive(Debug, Clone)]
pub struct Report {
    /// The bundle it came from.
    pub bundle: Bundle,
    /// One row per re-derived check.
    pub rows: Vec<Assertion>,
    /// Every side rebuilt from the blobs alone.
    pub rebuilt: Vec<materialize::Pair>,
    /// Findings that are not one row: the A16 list and any closure error.
    pub findings: Vec<String>,
    /// Why the closure failed, when it did.
    ///
    /// Separate from `findings` because it is the *cause* and the rest are
    /// symptoms, and [`Report::headline`] orders them differently for that
    /// reason.
    pub closure: Option<String>,
    /// Whether the recomputation agreed with what the capture recorded.
    pub agrees: bool,
}

impl Report {
    /// The one line that says what is actually wrong.
    ///
    /// Root cause first. When a blob cannot be decoded, A16 also fails - the
    /// file's digest no longer matches - and a reader who is handed the A16 row
    /// goes looking for a stale checksum list rather than a damaged blob. So:
    ///
    /// 1. the closure error, which names the chunk and why it would not decode;
    /// 2. otherwise the first red A6 row, which names the blob in words a reader
    ///    can act on;
    /// 3. otherwise the first red row of any id.
    pub fn headline(&self) -> Option<String> {
        if let Some(c) = &self.closure {
            return Some(c.clone());
        }
        let pick = |id: &str| self.rows.iter().find(|r| r.id == id && !r.ok);
        pick("A6")
            .or_else(|| self.rows.iter().find(|r| !r.ok))
            .map(|r| format!("{} {}: {}", r.id, r.subject, r.actual))
    }

    /// `pass` when every re-derived row held.
    pub fn result(&self) -> &'static str {
        if self.rows.iter().any(|r| !r.ok) {
            "fail"
        } else {
            "pass"
        }
    }

    /// The rows that did not hold.
    pub fn failures(&self) -> Vec<&Assertion> {
        self.rows.iter().filter(|r| !r.ok).collect()
    }

    /// `(total, passed, failed)`.
    pub fn counts(&self) -> (u64, u64, u64) {
        let failed = self.rows.iter().filter(|r| !r.ok).count() as u64;
        (self.rows.len() as u64, self.rows.len() as u64 - failed, failed)
    }

    /// How many rows each id produced.
    pub fn by_id(&self) -> BTreeMap<String, u64> {
        let mut m: BTreeMap<String, u64> = BTreeMap::new();
        for r in &self.rows {
            *m.entry(r.id.clone()).or_default() += 1;
        }
        m
    }

    /// Bytes rebuilt from the blobs.
    pub fn rebuilt_bytes(&self) -> u64 {
        self.rebuilt
            .iter()
            .flat_map(|p| [p.old.as_ref(), p.new.as_ref()])
            .flatten()
            .map(|s| s.bytes.len() as u64)
            .sum()
    }

    /// The re-derived rows as a `Selfcheck`, for `--output json`.
    ///
    /// The matrix is carried through from the manifest rather than recomputed:
    /// A17 re-derives *that* the matrix is complete, and a document that answered
    /// that by rebuilding the matrix from scratch would be a second, worse copy of
    /// the capture's own selection logic.
    pub fn to_selfcheck(&self) -> Selfcheck {
        let matrix = self
            .bundle
            .manifest
            .matrix
            .iter()
            .map(|m| crate::emit::selfcheck::MatrixRow {
                class: m.class.clone(),
                status: m.status.clone(),
                ok: crate::emit::selfcheck::MatrixRow::ok_status(&m.status),
            })
            .collect();
        Selfcheck::build(self.rows.clone(), matrix, &[])
    }
}

/// Re-derive every claim the bundle's own bytes can support.
///
/// Never fails. A bundle that cannot even be opened fails in [`Bundle::open`];
/// a bundle that opens and then does not hold is the *finding*, and returning it
/// as a report rather than an error is what lets the summary name every broken
/// row instead of only the first.
pub fn reverify(bundle: Bundle) -> Report {
    let root = bundle.root.clone();
    let mut rows: Vec<Assertion> = Vec::new();
    let mut findings: Vec<String> = Vec::new();

    // 1. A16.
    let found = bundle.checksums.findings(&root);
    let listed = bundle.checksums.entries.len();
    rows.push(if found.is_empty() {
        Assertion::pass(
            "A16",
            crate::emit::selfcheck::wording("A16"),
            "verify/checksums.json",
            &format!("{listed} files re-hashed, every digest matched"),
        )
    } else {
        findings.clone_from(&found);
        Assertion::fail(
            "A16",
            crate::emit::selfcheck::wording("A16"),
            &format!("{} of {listed} file(s)", found.len()),
            "every listed file re-hashes to its recorded digest",
            &found[0],
        )
    });

    // 2. A6, one row per stored blob.
    rows.extend(check_blobs(&root, &bundle.index));

    // 3. The closure check. A bundle whose blobs cannot rebuild both files is not
    // closed, and that is the single most important thing this command reports -
    // so it short-circuits: every row after this point would be about bytes that
    // do not exist.
    let rebuilt = match materialize::rebuild_all(&root, &bundle.index, &bundle.patchmaps) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(subject = "closure", error = %e, "the retained blobs cannot rebuild the bundle");
            findings.push(e.clone());
            rows.push(Assertion::fail(
                "A11",
                crate::emit::selfcheck::wording("A11"),
                "chunks/index.json",
                "every capture's chunks tile its file",
                &e,
            ));
            rows.push(Assertion::fail(
                A18_ID,
                A18,
                "chunks/",
                "both files rebuild from the retained blobs",
                &e,
            ));
            return Report {
                bundle,
                rows,
                rebuilt: Vec::new(),
                findings,
                closure: Some(e),
                agrees: false,
            };
        }
    };
    for p in &rebuilt {
        let Some(map) = bundle.patchmaps.get(&p.capture_id) else { continue };
        rows.extend(check_pair(&root, p, map));
    }

    // 4. The two claims that are about documents rather than blobs.
    rows.extend(check_config_ini(&root, &bundle.manifest));
    let (a17_ok, a17_detail) = bundle.manifest.a17();
    rows.push(if a17_ok {
        Assertion::pass(
            "A17",
            crate::emit::selfcheck::wording("A17"),
            "MANIFEST.json matrix",
            &a17_detail,
        )
    } else {
        Assertion::fail(
            "A17",
            crate::emit::selfcheck::wording("A17"),
            "MANIFEST.json matrix",
            "every class captured | covered_by | unavailable",
            &a17_detail,
        )
    });

    // 5. The recording, last.
    let agrees = check_agreement(&bundle, &rows);
    Report { bundle, rows, rebuilt, findings, closure: None, agrees }
}

/// A18's id, as the spec spells ids.
const A18_ID: &str = "A18";

/// A6, one row per stored blob.
///
/// The four manifest-declared checks are recorded in `index.json` and are *not*
/// re-run here: re-deriving them needs the manifest, and a verifier that reaches
/// for the manifest to check the manifest's own blob is checking the wrong thing.
/// What is re-derived is the part the index is responsible for - that the file at
/// `stored_path` is the bytes the index describes.
///
/// A blob the index lists with no path is red, not skipped. `--no-chunk-blobs`
/// produces exactly those, and `verify` reporting them as failures is what tells
/// an operator that the bundle they are holding cannot be materialized.
fn check_blobs(root: &Path, index: &ChunkIndex) -> Vec<Assertion> {
    let mut rows = Vec::new();
    for b in &index.blobs {
        let id = b.props.chunk_id.as_str();
        let Some(rel) = b.props.stored_path.as_deref() else {
            rows.push(Assertion::fail(
                "A6",
                crate::emit::selfcheck::wording("A6"),
                id,
                "a stored blob at a path",
                "no path: the index lists it but nothing was stored",
            ));
            continue;
        };
        let path = root.join(rel);
        match std::fs::read(&path) {
            Err(e) => rows.push(Assertion::fail(
                "A6",
                crate::emit::selfcheck::wording("A6"),
                rel,
                &describe_indexed(&b.props),
                &format!("unreadable: {e}"),
            )),
            Ok(raw) => rows.push(Assertion::check(
                "A6",
                crate::emit::selfcheck::wording("A6"),
                rel,
                &describe_indexed(&b.props),
                &describe_bytes(&raw),
            )),
        }
    }
    rows
}

/// How `chunks/index.json` describes a stored blob, in one comparable string.
fn describe_indexed(p: &crate::blobs::props::BlobProps) -> String {
    format!(
        "{} B md5 {} sha256 {} -> {} B md5 {}",
        p.bytes, p.md5_compressed, p.sha256_compressed, p.uncompressed_size, p.md5_decompressed
    )
}

/// How a blob on disk actually is, in the same shape.
///
/// Exactly the fields [`describe_indexed`] carries, in the same order: a
/// comparison between two differently-shaped descriptions would fail for a reason
/// that has nothing to do with the blob. The sha-256 is here for the reason the
/// index records it - a reader who distrusts this tool's md5 can check with a
/// stock tool, and `verify` should be held to the same standard it asks of
/// others.
///
/// Not-a-frame is its own sentence rather than a digest mismatch: a truncated
/// transfer and wrong content are different faults, and a reader sent to the
/// wrong one loses an afternoon.
fn describe_bytes(raw: &[u8]) -> String {
    if !hash::is_zstd(raw) {
        return format!("{} B, not a zstd frame", raw.len());
    }
    match hash::zstd_decompress(raw, "stored blob") {
        Ok(d) => format!(
            "{} B md5 {} sha256 {} -> {} B md5 {}",
            raw.len(),
            hash::md5_hex(raw),
            hash::sha256_hex(raw),
            d.len(),
            hash::md5_hex(&d)
        ),
        Err(e) => format!(
            "{} B md5 {} sha256 {}, {e}",
            raw.len(),
            hash::md5_hex(raw),
            hash::sha256_hex(raw)
        ),
    }
}

/// A7, A8, A11 and A18, for one capture's two sides.
///
/// One row per side per check, subject named `tree/path` - which is what an
/// operator will go and look at, rather than a capture id they then have to
/// resolve.
fn check_pair(root: &Path, pair: &materialize::Pair, map: &PatchMap) -> Vec<Assertion> {
    let mut rows = Vec::new();
    for (side, tree, declared) in [
        (&pair.old, "game", map.old.as_ref()),
        (&pair.new, "expected", map.new.as_ref()),
    ] {
        let (Some(r), Some(d)) = (side, declared) else { continue };
        let rel = format!("{tree}/{}", r.path);

        // A11: `rebuild` refuses to place a byte into a layout that violates it,
        // so a rebuilt side reaching here has already passed. The row says so
        // rather than being absent, because "checked and held" and "not checked"
        // must not look alike in a document a reader is scanning for gaps.
        rows.push(Assertion::pass(
            "A11",
            crate::emit::selfcheck::wording("A11"),
            &rel,
            &format!("{} chunks, gapless from 0, {} B", r.nchunks, r.bytes.len()),
        ));

        // A7 / A8: the rebuilt digest against the one the patchmap declared.
        let id = if tree == "game" { "A7" } else { "A8" };
        rows.push(Assertion::check(
            id,
            crate::emit::selfcheck::wording(id),
            &rel,
            &d.md5,
            &r.md5,
        ));
        if r.bytes.len() as i64 != d.size {
            rows.push(Assertion::fail(
                id,
                crate::emit::selfcheck::wording(id),
                &format!("{rel} (size)"),
                &format!("{} B", d.size),
                &format!("{} B", r.bytes.len()),
            ));
        }

        // A18: the rebuilt bytes against what the bundle shipped.
        match std::fs::read(root.join(&rel)) {
            Ok(shipped) => match materialize::compare(&rel, &r.bytes, &shipped) {
                Ok(()) => rows.push(Assertion::pass(
                    A18_ID,
                    A18,
                    &rel,
                    &format!("{} B rebuilt, {} B shipped, byte-for-byte", r.bytes.len(), shipped.len()),
                )),
                Err(e) => rows.push(Assertion::fail(
                    A18_ID,
                    A18,
                    &rel,
                    &format!("{} B md5 {}", r.bytes.len(), r.md5),
                    &e,
                )),
            },
            Err(e) => rows.push(Assertion::fail(
                A18_ID,
                A18,
                &rel,
                "present on disk",
                &format!("unreadable: {e}"),
            )),
        }
    }
    rows
}

/// A13, re-read from disk.
///
/// The same check the capture made, for the same reason: a parsed version
/// compared against a configured tag only means something if the parse went
/// through the file a reader would read.
fn check_config_ini(root: &Path, manifest: &Manifest) -> Vec<Assertion> {
    use crate::emit::selfcheck::wording;
    let old_tag = manifest.versions.old.tag.as_str();
    let cfg = root.join("game").join("config.ini");
    let subject = "game/config.ini";
    let text = match std::fs::read_to_string(&cfg) {
        Ok(t) => t,
        Err(e) => {
            return vec![Assertion::fail(
                "A13",
                wording("A13"),
                subject,
                "readable",
                &format!("{e}"),
            )];
        }
    };
    vec![match crate::emit::gamedir::parse_game_version(&text) {
        Ok(v) if v == old_tag => {
            Assertion::pass("A13", wording("A13"), subject, &format!("game_version == {v}"))
        }
        Ok(v) => Assertion::fail(
            "A13",
            wording("A13"),
            subject,
            &format!("game_version == {old_tag}"),
            &format!("game_version == {v}"),
        ),
        Err(e) => Assertion::fail(
            "A13",
            wording("A13"),
            subject,
            &format!("game_version == {old_tag}"),
            &e,
        ),
    }]
}

/// Does the recomputation agree with what the capture recorded?
///
/// One direction only, and the direction that matters: a row the capture
/// recorded as *holding* that no longer holds is a bundle that has changed since
/// it was written. The other direction - a row that used to fail and now holds -
/// is an improvement, and is reported through the row counts rather than as a
/// disagreement.
fn check_agreement(b: &Bundle, rows: &[Assertion]) -> bool {
    let recomputed = |id: &str| rows.iter().filter(|r| r.id == id).all(|r| r.ok);
    let mut ok = true;
    for a in &b.recorded.assertions {
        if !REDERIVED.contains(&a.id.as_str()) || !a.ok {
            continue;
        }
        if recomputed(&a.id) {
            continue;
        }
        tracing::error!(
            id = %a.id,
            subject = %a.subject,
            "the capture recorded this as holding; re-derivation from the bundle's own \
             bytes does not"
        );
        ok = false;
    }
    ok
}

/// One row of `list`: a bundle found under a root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// The bundle root, relative to the `--root` that was walked.
    pub path: String,
    /// `biz.old_tag__new_tag`, the directory the pair lives in.
    pub pair: String,
    /// The capture id.
    pub capture_id: String,
    /// When the capture ran, as the manifest recorded it.
    pub captured_at: String,
    /// `old_tag -> new_tag`.
    pub tags: String,
    /// How many captures the bundle holds.
    pub patchmaps: usize,
    /// Blobs stored on disk.
    pub blobs: usize,
    /// `pass` | `fail` | `incomplete`, as recorded.
    pub selfcheck: String,
    /// Whether the capture said it finished.
    pub complete: bool,
}

/// Every bundle under `root`, deepest first.
///
/// Recursive rather than a fixed depth, because the layout is
/// `<biz>/<old>__<new>/<capture_id>/` today and a root that points at
/// `artifacts/hk4e_global` and one that points at `artifacts/` should both work
/// without the caller knowing which.
///
/// A bundle that fails to open is *not* an error: `list` is an index, and a
/// half-written capture next to a good one is exactly what an operator wants to
/// see listed. It is skipped, and the skip is logged.
pub fn list(root: &Path) -> Result<Vec<Listed>, String> {
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()));
    }
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if dir.join("MANIFEST.json").is_file() {
            match Bundle::open(&dir) {
                Ok(b) => found.push(listed_of(root, &b)),
                Err(e) => tracing::warn!(
                    dir = %dir.display(),
                    error = %e,
                    "a directory under --root looks like a bundle but does not open; skipped"
                ),
            }
            // A bundle does not contain another bundle.
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut subs: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        // Sorted so the walk is the same on every filesystem.
        subs.sort();
        // Reversed, because the stack pops from the back.
        stack.extend(subs.into_iter().rev());
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

/// One `Listed` row from an opened bundle.
fn listed_of(root: &Path, b: &Bundle) -> Listed {
    let rel = crate::emit::index::rel(root, &b.root);
    let v = &b.manifest.versions;
    Listed {
        // The three levels below `--root` are the pair and the capture, and the
        // pair is one directory's name only - so it is read from the bundle's own
        // tags rather than assumed to be the parent directory.
        pair: format!("{}__{}", v.old.tag, v.new.tag),
        path: if rel.is_empty() { ".".to_string() } else { rel },
        capture_id: b.manifest.capture.id.clone(),
        captured_at: b.manifest.capture.captured_at.clone(),
        tags: format!("{} -> {}", v.old.tag, v.new.tag),
        patchmaps: b.patchmaps.len(),
        blobs: b.index.blobs.iter().filter(|x| x.props.stored_path.is_some()).count(),
        selfcheck: serde_json::to_string(&b.recorded.result)
            .unwrap_or_else(|_| "?".into())
            .trim_matches('"')
            .to_string(),
        complete: b.manifest.capture.complete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_a_path_that_is_not_a_directory_says_what_the_flag_takes() {
        let e = Bundle::open(Path::new("E:/definitely/not/here")).unwrap_err();
        assert!(e.contains("not a directory"), "{e}");
        assert!(e.contains("--artifact"), "{e}");
        assert!(e.contains("MANIFEST.json"), "{e}");
    }

    #[test]
    fn opening_a_directory_with_no_manifest_names_the_missing_document() {
        let d = std::env::temp_dir().join(format!("gircap-m9-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let e = Bundle::open(&d).unwrap_err();
        assert!(e.contains("MANIFEST.json"), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// `REDERIVED` and the spec's own list must not overlap, or "re-derived"
    /// would be claiming an assertion this command never looked at.
    #[test]
    fn the_rederived_ids_are_ones_this_command_actually_checks() {
        for id in REDERIVED {
            assert!(
                crate::emit::selfcheck::wording(id) != "an assertion this build does not describe",
                "A{id} has no wording, so re-deriving it would be meaningless"
            );
        }
        assert!(REDERIVED.contains(&"A7") && REDERIVED.contains(&"A8"));
    }

    /// The set is deliberately smaller than the seventeen.
    #[test]
    fn assertions_needing_the_api_are_not_claimed_as_re_derived() {
        for id in ["A1", "A2", "A3", "A4", "A5", "A9", "A10", "A12", "A14", "A15"] {
            assert!(!REDERIVED.contains(&id), "A{id} is listed as re-derived");
        }
    }

    #[test]
    fn a_blob_indexed_but_not_stored_is_red_rather_than_skipped() {
        let index = ChunkIndex::build(vec![]);
        assert!(check_blobs(Path::new("."), &index).is_empty());
        let props = crate::blobs::props::BlobProps::unmeasured(&crate::manifest::proto::SophonChunk {
            id: "aa00_".to_string() + &"0".repeat(32),
            uncompressed_md5: "0".repeat(32),
            offset: 0,
            compressed_size: 10,
            uncompressed_size: 100,
            unknown_field6: 0,
            compressed_md5: "1".repeat(32),
        });
        assert!(props.stored_path.is_none());
        let index = ChunkIndex::build(vec![(
            crate::blobs::IndexedBlob {
                props,
                referenced_by: Vec::new(),
                sets: Vec::new(),
            },
            Vec::new(),
        )]);
        let rows = check_blobs(Path::new("."), &index);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].ok, "a listed-but-unstored blob cannot be materialized");
        assert!(rows[0].actual.contains("nothing was stored"), "{}", rows[0].actual);
    }

    #[test]
    fn a_blob_whose_bytes_were_altered_turns_its_row_red() {
        let d = std::env::temp_dir().join(format!("gircap-m9-blob-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("chunks/ab")).unwrap();
        let body = vec![7_u8; 64];
        let raw = zstd::encode_all(body.as_slice(), 1).unwrap();
        std::fs::write(d.join("chunks/ab/cd"), &raw).unwrap();
        let chunk = crate::manifest::proto::SophonChunk {
            id: format!("ab{:0>29}", "cd"),
            uncompressed_md5: hash::md5_hex(&body),
            offset: 0,
            compressed_size: raw.len() as i64,
            uncompressed_size: 64,
            unknown_field6: 0,
            compressed_md5: hash::md5_hex(&raw),
        };
        let mut props = crate::blobs::props::BlobProps::measure(&raw, &chunk).unwrap();
        props.set_stored("chunks/ab/cd");
        let index = ChunkIndex::build(vec![(
            crate::blobs::IndexedBlob { props, referenced_by: Vec::new(), sets: Vec::new() },
            Vec::new(),
        )]);
        assert!(check_blobs(&d, &index)[0].ok, "the good bundle's row is green");

        // Flip one byte inside the frame: it still decodes, to different bytes.
        let mut bad = raw.clone();
        let last = bad.len() - 1;
        bad[last] ^= 0xFF;
        std::fs::write(d.join("chunks/ab/cd"), &bad).unwrap();
        let row = &check_blobs(&d, &index)[0];
        assert!(!row.ok, "one altered byte turns the row red");
        assert_eq!(row.subject, "chunks/ab/cd", "and names the blob");
        assert!(row.id == "A6", "and the assertion: {}", row.id);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_status_the_reader_cannot_agree_with_is_not_counted_as_ok() {
        assert!(crate::emit::selfcheck::MatrixRow::ok_status("captured"));
        assert!(crate::emit::selfcheck::MatrixRow::ok_status("covered_by"));
        assert!(crate::emit::selfcheck::MatrixRow::ok_status("unavailable"));
        assert!(!crate::emit::selfcheck::MatrixRow::ok_status("over_budget"));
        assert!(!crate::emit::selfcheck::MatrixRow::ok_status("something else"));
    }
}