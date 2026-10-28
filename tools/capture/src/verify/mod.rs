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
//!
//! A21-A23 are the bundle's **coverage** — whether it holds both sides of the
//! reuse, moved and size boundaries — and they are re-derived but never
//! *failed*: a bundle that does not cover the patch space is still internally
//! consistent, so it is reported as a finding and the bundle still verifies.

pub mod materialize;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::assemble::PatchMap;
use crate::blobs::ChunkIndex;
use crate::emit::index::{Checksums, Manifest};
use crate::emit::selfcheck::{Assertion, Selfcheck};
use crate::hash;
use crate::http::probe::ErrorShapes;

/// A18's wording, kept in one place because `verify` and the docs both quote it.
pub const A18: &str =
    "the bytes rebuilt from chunks/ are byte-for-byte the bytes game/ and expected/ hold";

/// The ids [`reverify`] re-derives **and** the capture also recorded.
///
/// The overlap, and the reason it is a `&'static` list rather than a computed
/// one: these are the rows `check_agreement` is entitled to compare, so putting
/// an id here silently promotes it from "re-derived" to "re-derived *and* checked
/// against the recording". A18 is re-derived but never recorded - the capture
/// cannot write it, because it is a statement about the bundle rather than about
/// the run - so it is deliberately absent.
pub const REDERIVED: [&str; 7] = ["A16", "A6", "A11", "A7", "A8", "A13", "A17"];

/// Re-derived here, and never recorded by any capture.
///
/// Kept apart from [`REDERIVED`] because that set means "re-derived *and*
/// written down", which is what makes `check_agreement` possible: there is
/// nothing to agree with for these. Putting them in `REDERIVED` would have the
/// summary claim a cross-check that cannot exist, and would drag them into a
/// comparison against a row that was never emitted.
pub const DERIVED_UNRECORDED: [&str; 2] = ["A18", "A20"];

/// Everything `verify` looks at, in the order the summary prints it.
pub fn derived_ids() -> Vec<&'static str> {
    let mut v = REDERIVED.to_vec();
    v.extend(DERIVED_UNRECORDED);
    v
}

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
    /// `api/error_shapes.json` - what the API did with the requests it would not
    /// serve. Required: a bundle without it cannot answer a question every
    /// consumer of the bundle eventually has to ask.
    pub error_shapes: ErrorShapes,
}

impl Bundle {
    /// Read every document a bundle has to have, naming the first that is absent.
    ///
    /// `MANIFEST.json` first, because the rest are located relative to the schema
    /// version it declares and a directory without one is not a bundle. Saying
    /// "no MANIFEST.json here" beats five separate "no such file" errors.
    ///
    /// **`api/error_shapes.json` is required, not additive.** A bundle recorded
    /// only successes contains no not-found evidence at all, so a consumer that
    /// wants to replay one would have to invent what a `404` looks like — and its
    /// claim would be unfounded. Treating the document as optional let a bundle
    /// drop the tool's only measurement of that and still verify clean, which
    /// makes "this bundle can tell you how the API fails" true of some bundles
    /// and not others with nothing to tell the reader which is which.
    ///
    /// It is read back through [`crate::http::probe::ErrorShapes`], the type that
    /// wrote it, so a document that is present and unusable fails here, by name,
    /// rather than at some consumer that happened to parse it.
    pub fn open(root: &Path) -> Result<Bundle, String> {
        if !root.is_dir() {
            return Err(format!(
                "{} is not a directory; --artifact takes the bundle root, the directory \
                 holding MANIFEST.json",
                root.display()
            ));
        }
        let manifest_path = root.join("MANIFEST.json");
        // The schema version is read **first, and on its own**, because it is
        // the one fact that decides whether the rest of the document means
        // anything.
        //
        // A bundle written by an older schema is **refused**, not read with
        // the new reader's defaults. The version-1 patchmap carried a `repair`
        // block and an `expectations.summary` block, and its class ids named
        // a repair tool's internal decisions; a reader that understood the
        // current shape would silently ignore all three and report a bundle
        // as consistent when it is not.
        //
        // Reading the version before the document matters as much as refusing
        // it: a version-1 `MANIFEST.json` does not have the fields the current
        // reader wants, so a reader that parsed the document first would fail
        // on a *missing field* and say nothing about the version that caused
        // it. The version is the diagnosis; the missing field is a symptom.
        let version = schema_version_of(&manifest_path)?;
        if version != crate::SCHEMA_VERSION {
            return Err(format!(
                "{}: schema version {} is not the current {}; this bundle was written \
                 by an older gircap and its patchmaps carry a different shape.\n\
                 \x20   capture a current bundle with:  cargo run -p gircap -- capture --out {}",
                manifest_path.display(),
                version,
                crate::SCHEMA_VERSION,
                root.display(),
            ));
        }
        let manifest: Manifest = read_json(&manifest_path)?;
        let index: ChunkIndex = read_json(&root.join("chunks/index.json"))?;
        let checksums: Checksums = read_json(&root.join("verify/checksums.json"))?;
        let recorded: Selfcheck = read_json(&root.join("verify/selfcheck.json"))?;
        let error_shapes: ErrorShapes = read_json(&root.join("api/error_shapes.json"))?;
        let patchmaps = read_patchmaps(root)?;
        // **The shape of every document-supplied path is checked at the door.**
        // `verify` is the command an operator runs on a bundle they may not have
        // produced, and every join below this point is `root.join(document
        // field)`. Checking at the individual joins caught only the sinks that
        // happened to remember; a field read by A6, by A18 or by `materialize`
        // through the patchmap was still a way out. One pass here names the file
        // and the field before any command reads a byte.
        validate_document_paths(&manifest, &index, &patchmaps)?;
        Ok(Bundle {
            root: root.to_path_buf(),
            manifest,
            index,
            patchmaps,
            checksums,
            recorded,
            error_shapes,
        })
    }

    /// One line naming the bundle, for the stderr narration.
    pub fn describe(&self) -> String {
        let v = &self.manifest.versions;
        format!(
            "{} {} {} ({}) -> {} ({}) - {} blobs, {} patchmaps, {} error shapes, \
             recorded {} rows {} failed",
            self.manifest.biz.id,
            v.old.tag,
            v.old.build_id,
            self.manifest.capture.id,
            v.new.tag,
            v.new.build_id,
            self.index.blobs.len(),
            self.patchmaps.len(),
            self.error_shapes.shapes.len(),
            self.recorded.counts.total,
            self.recorded.counts.failed,
        )
    }
}

/// Read every `patchmaps/*.json`, keyed by the id the document declares.
///
/// Keyed by `capture_id` rather than by filename: the filename is a convenience
/// and the field is the identity, so trusting it would let a renamed file
/// masquerade as a second capture. The filename is therefore **not** required to
/// equal the id, but two documents may not claim the same id: that is one capture
/// with two irreconcilable descriptions, and `BTreeMap::insert` used to drop the
/// second silently.
///
/// A directory entry that cannot be read, and two documents declaring one id, are
/// both errors naming the file(s). `verify` is the last place a bundle can be
/// told it is structurally wrong; a capture that vanished between two patchmaps
/// is exactly the defect it must not step over.
///
/// **A missing directory is an empty set, not an error.** `MANIFEST.json` is what
/// makes a directory a bundle; `patchmaps/` is where its captures live, and a
/// bundle can legitimately have none - `--no-chunk-blobs` admits captures and
/// stores no blobs, so there is nothing to map. Refusing to open such a bundle
/// would hide the finding a reader wants behind a layout complaint.
fn read_patchmaps(root: &Path) -> Result<BTreeMap<String, PatchMap>, String> {
    let dir = root.join("patchmaps");
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!(
                artifact = %root.display(),
                "no patchmaps/ directory: the bundle has no captures to verify"
            );
            return Ok(BTreeMap::new());
        }
        Err(e) => {
            return Err(format!("cannot read {}: {e}", dir.display()));
        }
    };
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in rd {
        let entry = entry.map_err(|e| format!("cannot read an entry of {}: {e}", dir.display()))?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            paths.push(path);
        }
    }
    paths.sort();
    let mut out = BTreeMap::new();
    let mut from: BTreeMap<String, PathBuf> = BTreeMap::new();
    for p in paths {
        let map: PatchMap = read_json(&p)?;
        let id = map.capture_id.clone();
        if let Some(first) = from.get(&id) {
            return Err(format!(
                "two patchmaps declare capture_id {id:?}: {} and {}",
                first.display(),
                p.display()
            ));
        }
        from.insert(id.clone(), p);
        out.insert(id, map);
    }
    Ok(out)
}

/// Refuse a bundle whose own documents name a path that leaves it.
///
/// Three fields become filesystem joins inside the offline commands:
///
/// - `chunks/index.json`'s `stored_path` is read by A6 and by every chunk
///   decode;
/// - `chunks/index.json`'s `references[].path` and `patchmaps/*.json`'s `path`
///   key the rebuild, and `materialize` turns the rebuilt path back into
///   `--out/<tree>/<path>`;
/// - `MANIFEST.json`'s `capture.id` names the default `materialize` output when
///   the artifact itself is a filesystem root.
///
/// A bundle is a document someone may have edited, so this is the read-side
/// twin of the check `manifest::capture` applies before writing a path under
/// `--out`. The rule is the shared one in [`crate::paths`]: no `\`, absolute
/// path, drive prefix, `..`, empty component or Windows alias can name a file
/// inside the bundle.
///
/// **Refused, not repaired.** A field that fails the rule is an error naming the
/// document, the field and the value; silently skipping it would let a tampered
/// bundle verify clean while omitting the one blob or capture that made it
/// interesting.
fn validate_document_paths(
    manifest: &Manifest,
    index: &ChunkIndex,
    patchmaps: &BTreeMap<String, PatchMap>,
) -> Result<(), String> {
    if let Some(problem) = crate::paths::field_component_problem(&manifest.capture.id) {
        return Err(format!("MANIFEST.json capture.id {problem}"));
    }
    for blob in &index.blobs {
        if let Some(rel) = blob.props.stored_path.as_deref()
            && let Some(problem) = crate::paths::relative_path_problem(rel)
        {
            return Err(format!(
                "chunks/index.json blob {} stored_path {problem}",
                blob.props.chunk_id
            ));
        }
        for reference in &blob.referenced_by {
            if let Some(problem) = crate::paths::relative_path_problem(&reference.path) {
                return Err(format!(
                    "chunks/index.json blob {} referenced_by path {problem}",
                    blob.props.chunk_id
                ));
            }
        }
    }
    for map in patchmaps.values() {
        if let Some(problem) = crate::paths::relative_path_problem(&map.path) {
            return Err(format!("patchmaps/{}.json path {problem}", map.capture_id));
        }
    }
    Ok(())
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

/// What a directory's `MANIFEST.json` says about whether it is a bundle.
///
/// Three outcomes, kept apart because conflating them loses a bundle:
/// **absence** and **a document that is not a bundle** are both "step over",
/// but a manifest the process **cannot read** is an entry the walk could not
/// see, and [`list`] must fail rather than answer with a shorter inventory.
enum ManifestProbe {
    /// No `MANIFEST.json` here: not a bundle.
    Missing,
    /// A manifest exists but declares no schema version this build can read:
    /// not a bundle.
    NotABundle(String),
    /// A readable manifest declaring this schema version.
    Version(u32),
}

/// Probe a `MANIFEST.json`, distinguishing absence from a read failure.
///
/// **Nothing but the version.** The document is read as a JSON value rather
/// than as a [`Manifest`], because a bundle written by an older schema does
/// not have the fields the current reader wants: parsed as a `Manifest` it
/// fails on a missing field, and the message names a symptom instead of the
/// cause. The version is the diagnosis.
///
/// A document with no readable version is **not a bundle** — a half-written
/// capture, or a directory that only looks like one — and says so. A document
/// that could not be read at all is not the same fact: it is returned as
/// `Err` naming the path and the I/O error, because a bundle may be behind it.
///
/// The reader is a parameter so [`list_with`] can inject a failing one; the
/// only other caller passes [`std::fs::read`].
fn probe_manifest(
    path: &Path,
    read: impl FnOnce(&Path) -> std::io::Result<Vec<u8>>,
) -> Result<ManifestProbe, String> {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => {}
        // A `MANIFEST.json` that is not a file is not a manifest: the directory
        // is not a bundle, and the walk keeps descending as `is_file() == false`
        // used to make it.
        Ok(_) => return Ok(ManifestProbe::Missing),
        // Nothing there means no bundle. Anything else is a directory the walk
        // could not see, which is not the same as an empty one.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ManifestProbe::Missing),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    }
    let raw = read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let value: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(value) => value,
        Err(e) => {
            return Ok(ManifestProbe::NotABundle(format!(
                "{}: {e}",
                path.display()
            )));
        }
    };
    match value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
    {
        Some(v) => Ok(ManifestProbe::Version(v as u32)),
        None => Ok(ManifestProbe::NotABundle(format!(
            "{}: no schema_version; this is not a bundle gircap wrote",
            path.display()
        ))),
    }
}

/// The schema version a bundle declares, read on its own.
///
/// The single-caller wrapper around [`probe_manifest`] for [`Bundle::open`],
/// which wants a version or a reason and has no "step over" arm.
fn schema_version_of(path: &Path) -> Result<u32, String> {
    match probe_manifest(path, |p| std::fs::read(p))? {
        ManifestProbe::Version(v) => Ok(v),
        ManifestProbe::Missing => Err(format!(
            "{}: not found; a bundle root holds a MANIFEST.json",
            path.display()
        )),
        ManifestProbe::NotABundle(why) => Err(why),
    }
}

/// What re-verification found.
///
/// The rebuilt bytes are kept rather than dropped, because [`materialize`] is the
/// same rebuild with somewhere to put them, and rebuilding twice on a 135 MB
/// bundle costs a minute of the operator's time for nothing.
///
/// Deliberately **not** [`Clone`]: `rebuilt` is the whole fixture, so a derived
/// `Clone` is a ~135 MB copy waiting for the caller that wanted a second opinion.
#[derive(Debug)]
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
        (
            self.rows.len() as u64,
            self.rows.len() as u64 - failed,
            failed,
        )
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
        let Some(map) = bundle.patchmaps.get(&p.capture_id) else {
            continue;
        };
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
            "every class captured | covered_by | over_budget | unavailable",
            &a17_detail,
        )
    });

    // 4b. The bundle's **coverage**: whether it holds both sides of the reuse
    // boundary, the moved boundary and the size boundary.
    //
    // A **finding**, not a failed check, and that is the whole distinction.
    // The coverage is a design property of the bundle - it is what the class
    // set was captured for - and a bundle that does not cover everything (a
    // narrow one, or a hand-built one) is still internally consistent. The
    // *capture* is where the coverage is a contract: a capture that does not
    // cover the patch space fails its own run (A21-A23 are fatal there).
    // The verifier's job is to prove the bundle still says what it said, and
    // "this bundle does not cover both sides of the reuse boundary" is a thing
    // it said.
    let shapes: Vec<crate::assemble::PatchShape> = bundle
        .patchmaps
        .values()
        .map(crate::assemble::patch_shape)
        .collect();
    for row in crate::emit::selfcheck::coverage(&shapes) {
        if !row.ok {
            findings.push(format!("{}: {}", row.id, row.actual));
            tracing::error!(
                assertion = %row.id,
                subject = %row.subject,
                actual = %row.actual,
                "the bundle does not cover the patch space"
            );
        }
    }

    // 5. The recording, last.
    let agrees = check_agreement(&bundle, &rows);
    Report {
        bundle,
        rows,
        rebuilt,
        findings,
        closure: None,
        agrees,
    }
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
        // Defence in depth: `Bundle::open` refuses this shape before a reader
        // gets here, but A6 is also callable through `reverify` on an index a
        // caller built itself. The join must never be the first check.
        if let Some(problem) = crate::paths::relative_path_problem(rel) {
            rows.push(Assertion::fail(
                "A6",
                crate::emit::selfcheck::wording("A6"),
                rel,
                "a stored blob at a path inside the bundle",
                &format!("unsafe path: {problem}"),
            ));
            continue;
        }
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
    // Both compressed digests in one pass, and the decoded pair streamed rather
    // than materialised — `describe_bytes` exists to *describe*, so a multi-megabyte
    // decoded buffer it then throws away was the wrong shape entirely.
    let compressed = hash::md5_sha256(raw);
    match hash::zstd_measure(raw, "stored blob") {
        Ok(m) => format!(
            "{} B md5 {} sha256 {} -> {} B md5 {}",
            compressed.bytes, compressed.md5, compressed.sha256, m.len, m.md5
        ),
        Err(e) => format!(
            "{} B md5 {} sha256 {}, {e}",
            compressed.bytes, compressed.md5, compressed.sha256
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
        let (Some(r), Some(d)) = (side, declared) else {
            continue;
        };
        let rel = format!("{tree}/{}", r.path);
        // Same defence as A6: the patchmap path was checked when the bundle was
        // opened, and this is the join that would read it.
        if let Some(problem) = crate::paths::relative_path_problem(&rel) {
            rows.push(Assertion::fail(
                A18_ID,
                A18,
                &rel,
                "a path inside the bundle",
                &format!("unsafe path: {problem}"),
            ));
            continue;
        }

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
                    &format!(
                        "{} B rebuilt, {} B shipped, byte-for-byte",
                        r.bytes.len(),
                        shipped.len()
                    ),
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
    rows.extend(check_patchmap_arithmetic(map));
    rows
}

/// A20, re-derived: a patchmap's `totals` and `expectations` are the arithmetic
/// of its own `plan`, and `verify` does that arithmetic again rather than
/// trusting the number.
///
/// Not belt-and-braces. The bundle that motivated it carried
/// `"plan": null`, `"totals": null` and `"expectations": { … "download_bytes": 0 }`
/// for its `added` capture, and `verify` passed it with 83 green rows - because
/// nothing looked. The one-chunk file in the M11 bundle costs 114 357 B to repair
/// and the artifact said zero. A figure a consumer compares against has to be the
/// sum over the plan's FETCH entries, so it is re-summed here.
///
/// Re-derivable offline, unlike A1-A5: it reads only the patchmap, which is why it
/// belongs in the derived set rather than the covered-by-recording one.
const A20_ID: &str = "A20";
const A20: &str = "a patchmap's totals and expectations are the arithmetic of its own plan";

fn check_patchmap_arithmetic(map: &PatchMap) -> Vec<Assertion> {
    let mut rows = Vec::new();
    let subject = format!("patchmaps/{}.json", map.capture_id);
    let Some(totals) = &map.totals else {
        // No totals is only correct when there is no target: a capture with a new
        // side must cost something to produce. Saying so here is what stops the
        // defect coming back quietly.
        if map.new.is_some() {
            rows.push(Assertion::fail(
                A20_ID,
                A20,
                &subject,
                "a capture with a new side carries totals",
                "no totals, and no plan to derive them from",
            ));
        }
        return rows;
    };
    let Some(plan) = &map.plan else {
        rows.push(Assertion::fail(
            A20_ID,
            A20,
            &subject,
            "totals imply a plan",
            "no plan to derive them from",
        ));
        return rows;
    };

    let fetch: Vec<_> = plan
        .iter()
        .filter(|e| e.decision == crate::assemble::Decision::Fetch)
        .collect();
    let reuse: Vec<_> = plan
        .iter()
        .filter(|e| e.decision == crate::assemble::Decision::Reuse)
        .collect();
    let bytes: i64 = fetch.iter().map(|e| e.compressed_size).sum();
    let moved = plan
        .iter()
        .filter(|e| {
            e.source_range
                .as_ref()
                .is_some_and(|s| s.start != e.target_range.start)
        })
        .count();

    for (label, got, want) in [
        ("chunks", totals.chunks as i64, plan.len() as i64),
        ("reuse", totals.reuse as i64, reuse.len() as i64),
        ("fetch", totals.fetch as i64, fetch.len() as i64),
        ("moved", totals.moved as i64, moved as i64),
        ("download_bytes", totals.download_bytes, bytes),
    ] {
        if got != want {
            rows.push(Assertion::fail(
                A20_ID,
                A20,
                &format!("{subject} ({label})"),
                &want.to_string(),
                &got.to_string(),
            ));
        }
    }

    // The expectation a consumer actually reads must be that same figure.
    if map.expectations.download_bytes != bytes {
        rows.push(Assertion::fail(
            A20_ID,
            A20,
            &format!("{subject} (expectations.download_bytes)"),
            &format!("{bytes}, the sum over the FETCH entries"),
            &map.expectations.download_bytes.to_string(),
        ));
    }
    // And the negative assertion must be the **same** set as the plan's REUSE
    // entries — stated twice, so it has to be checked twice. `must_not_fetch`
    // is the REUSE set a consumer asserts on ("none of these may be
    // requested"), and the plan's REUSE entries are the same fact derived from
    // the other end. One source of truth, two statements, one check.
    let reuse_ids: Vec<&str> = reuse.iter().map(|e| e.chunk_id.as_str()).collect();
    let forbidden: Vec<&str> = map
        .expectations
        .must_not_fetch
        .iter()
        .map(String::as_str)
        .collect();
    let mut want: Vec<&str> = reuse_ids.clone();
    want.sort_unstable();
    let mut got = forbidden.clone();
    got.sort_unstable();
    if got != want {
        rows.push(Assertion::fail(
            A20_ID,
            A20,
            &format!("{subject} (expectations.must_not_fetch)"),
            &format!("the {} REUSE chunk(s) of the plan", want.len()),
            &format!("{} id(s): {got:?}", got.len()),
        ));
    }
    // And it must not contradict the outcome the patchmap claims.
    if map.new.is_some() && map.expectations.absent_after {
        rows.push(Assertion::fail(
            A20_ID,
            A20,
            &format!("{subject} (absent_after)"),
            "false: the capture has a new side",
            "true",
        ));
    }

    if rows.is_empty() {
        rows.push(Assertion::pass(
            A20_ID,
            A20,
            &subject,
            &format!(
                "{} chunk(s): {} reused, {} fetched, {} B - totals and expectations re-derived",
                plan.len(),
                reuse.len(),
                fetch.len(),
                bytes
            ),
        ));
    }
    rows
}

/// A13, re-read from disk.
///
/// **Delegates to [`crate::emit::selfcheck::check_config_ini`]**, which the capture also
/// calls. This was a second copy of the same read → parse → compare, with
/// different wording in every branch: it said `readable` where the capture says
/// `game_version == <tag>`, and reported the value as `game_version == v` where
/// the capture writes `game_version is v`. Since `verify` cross-checks its rows
/// against the capture's recorded ones, two spellings means the check can
/// disagree with the record it is supposed to be re-deriving.
fn check_config_ini(root: &Path, manifest: &Manifest) -> Vec<Assertion> {
    vec![crate::emit::selfcheck::check_config_ini(
        root,
        &manifest.versions.old.tag,
    )]
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

/// What [`list`] found under a root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    /// The bundles that opened.
    pub bundles: Vec<Listed>,
    /// The directories that hold a `MANIFEST.json` this build will not read.
    ///
    /// **Not a record.** The spec says a directory that does not open is
    /// stderr-only, because a record would have to carry the failure instead
    /// of the facts. That stays true — but the operator's inventory is still
    /// incomplete, so these are counted and named rather than dropped.
    pub unreadable: Vec<Unreadable>,
}

/// A bundle under the root that this build will not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unreadable {
    /// The bundle root, relative to the `--root` that was walked.
    pub path: String,
    /// Why it was not read.
    pub reason: String,
}

/// Every bundle under `root`, sorted by relative path.
///
/// Recursive rather than a fixed depth, because the layout is
/// `<biz>/<old>__<new>/<capture_id>/` today and a root that points at
/// `artifacts/hk4e_global` and one that points at `artifacts/` should both work
/// without the caller knowing which. The order is **not** deepest first: the
/// walk is depth-first, but both lists are re-sorted by relative-path string
/// before they are returned, so a mixed-depth root is ordered by name at the
/// top level first. The spec does not promise an order; this one is stable and
/// decides the summary's `first` and `last`.
///
/// **A bundle that fails to open is not an error, and it is not dropped.**
/// `list` is an index, and a half-written capture next to a good one is
/// exactly what an operator wants to see listed — so a directory that is not a
/// bundle at all is stepped over silently. A directory that *is* a bundle this
/// build refuses — an older schema, which is a permanent condition with a
/// known fix — is named, with the reason, in [`Index::unreadable`], so the
/// operator's inventory is complete.
///
/// **A walk that cannot read a directory or a manifest is an error, not a
/// shorter inventory.** The `unreadable` list can name a bundle that does not
/// open, but it cannot name what it never saw: an entry error, an ACL-denied
/// directory, or a `MANIFEST.json` that cannot be read used to be dropped, so
/// `list` returned a confident, incomplete answer with exit `0`. An audit
/// command fails fast instead and names the directory, entry or manifest it
/// could not read.
pub fn list(root: &Path) -> Result<Index, String> {
    list_with(
        root,
        |d| {
            let rd = std::fs::read_dir(d)?;
            Ok(rd.map(|e| e.map(|e| e.path())).collect())
        },
        |p| std::fs::read(p),
    )
}

/// [`list`] with the directory and manifest readers injected, so a test can
/// make one read fail and see that the walk stops rather than shortening the
/// inventory.
fn list_with(
    root: &Path,
    mut read: impl FnMut(&Path) -> std::io::Result<Vec<std::io::Result<PathBuf>>>,
    mut read_manifest: impl FnMut(&Path) -> std::io::Result<Vec<u8>>,
) -> Result<Index, String> {
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()));
    }
    let mut found = Vec::new();
    let mut unreadable = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let manifest = dir.join("MANIFEST.json");
        let probe = probe_manifest(&manifest, &mut read_manifest)?;
        if !matches!(probe, ManifestProbe::Missing) {
            match Bundle::open(&dir) {
                Ok(b) => found.push(listed_of(root, &b)),
                Err(e) => {
                    // A directory that is not a bundle at all is stepped over;
                    // one that is a bundle this build refuses is named.
                    if matches!(probe, ManifestProbe::Version(_)) {
                        unreadable.push(Unreadable {
                            path: crate::emit::index::rel(root, &dir),
                            reason: e.clone(),
                        });
                    }
                    tracing::warn!(
                        dir = %dir.display(),
                        error = %e,
                        "a directory under --root looks like a bundle but does not open; skipped"
                    );
                }
            }
            // A bundle does not contain another bundle.
            continue;
        }
        // Sorted so the walk is the same on every filesystem; reversed because
        // the stack pops from the back.
        let subs = child_dirs(&dir, |d| read(d))?;
        stack.extend(subs.into_iter().rev());
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    unreadable.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Index {
        bundles: found,
        unreadable,
    })
}

/// The child directories of `dir`, sorted, or the first reason the walk could
/// not see all of them.
///
/// The reader is injected so the entry-error arm can be exercised without
/// platform-specific permissions: a test hands over a synthetic `Err` for one
/// entry, which is otherwise only reachable when a share disconnects between
/// `read_dir` and the stat. `metadata` follows symlinks, as the `is_dir` it
/// replaces did, and an entry that cannot be stat'ed is an error rather than a
/// file: losing it would be the same silent omission this walk is fixing.
fn child_dirs(
    dir: &Path,
    read: impl FnOnce(&Path) -> std::io::Result<Vec<std::io::Result<PathBuf>>>,
) -> Result<Vec<PathBuf>, String> {
    let entries = read(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut subs = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| format!("cannot read an entry of {}: {e}", dir.display()))?;
        let is_dir = std::fs::metadata(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?
            .is_dir();
        if is_dir {
            subs.push(path);
        }
    }
    subs.sort();
    Ok(subs)
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
        blobs: b
            .index
            .blobs
            .iter()
            .filter(|x| x.props.stored_path.is_some())
            .count(),
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
        let d = girpr_tempdir::TempDir::new("gircap-m9-empty", "no_manifest");
        let e = Bundle::open(&d).unwrap_err();
        assert!(e.contains("MANIFEST.json"), "{e}");
    }

    /// **A bundle written by an older schema is refused, and the refusal says
    /// what to do.**
    ///
    /// The schema bump exists precisely so that a version-1 bundle can be
    /// refused: its patchmaps carry a `repair` block and an
    /// `expectations.summary` block, and its class ids named a repair tool's
    /// internal decisions — a reader that understood the current shape would
    /// ignore all three and report the bundle as consistent when it is not.
    ///
    /// The manifest here is a version-1 manifest with **nothing else in it**,
    /// which is the point: the version is read on its own, so the refusal
    /// names the version rather than a missing field the version caused.
    #[test]
    fn a_v1_bundle_is_refused_with_the_command_that_makes_a_current_one() {
        let d = girpr_tempdir::TempDir::new("gircap-v1", "v1_bundle");
        std::fs::write(d.join("MANIFEST.json"), br#"{"schema_version": 1}"#)
            .expect("writes the manifest");
        let e = Bundle::open(&d).unwrap_err();
        assert!(e.contains("schema version 1"), "{e}");
        assert!(e.contains("is not the current"), "{e}");
        assert!(
            e.contains("cargo run -p gircap -- capture --out"),
            "the refusal must say how to make a current one: {e}"
        );
    }

    /// **And `list` does not let it vanish.**
    ///
    /// `list` is an index, and an index that quietly omits a bundle is wrong.
    /// A directory that is not a bundle at all is stepped over — a half-written
    /// capture is a normal transient — but one that is a bundle this build
    /// refuses is named, with the reason.
    #[test]
    fn a_v1_bundle_is_named_by_list_rather_than_dropped() {
        let d = girpr_tempdir::TempDir::new("gircap-v1-list", "v1_list");
        let old = d.join("old");
        std::fs::create_dir_all(&old).expect("makes the directory");
        std::fs::write(old.join("MANIFEST.json"), br#"{"schema_version": 1}"#)
            .expect("writes the manifest");

        let index = list(&d).expect("lists");
        assert!(
            index.bundles.is_empty(),
            "a bundle that does not open is not a row: {index:?}"
        );
        assert_eq!(index.unreadable.len(), 1, "{index:?}");
        assert_eq!(index.unreadable[0].path, "old");
        assert!(
            index.unreadable[0].reason.contains("schema version 1"),
            "and the reason is the version: {index:?}"
        );
    }

    /// A directory that is not a bundle at all is stepped over silently: it is
    /// a half-written capture, not something the operator has to act on.
    #[test]
    fn a_directory_that_is_not_a_bundle_is_not_named_by_list() {
        let d = girpr_tempdir::TempDir::new("gircap-not-a-bundle", "not_a_bundle");
        let broken = d.join("broken");
        std::fs::create_dir_all(&broken).expect("makes the directory");
        std::fs::write(broken.join("MANIFEST.json"), b"{ not json").expect("writes it");

        let index = list(&d).expect("lists");
        assert!(index.bundles.is_empty(), "{index:?}");
        assert!(
            index.unreadable.is_empty(),
            "a manifest that is not JSON is not a bundle this build refuses: {index:?}"
        );
    }

    /// A directory the walk cannot read is **named**, not skipped.
    ///
    /// `list` is the operator's inventory; dropping a subtree makes a confident
    /// answer that is short. The reader is real here — a file passed where a
    /// directory is expected is the cheapest read failure a test can make.
    #[test]
    fn a_walk_that_cannot_read_a_directory_names_it() {
        let d = girpr_tempdir::TempDir::new("gircap-list-unreadable", "unreadable");
        let file = d.join("not_a_directory");
        std::fs::write(&file, b"x").expect("writes a file");

        let e = child_dirs(&file, |p| {
            let rd = std::fs::read_dir(p)?;
            Ok(rd.map(|e| e.map(|e| e.path())).collect())
        })
        .expect_err("a file cannot be read as a directory");
        assert!(e.contains("cannot read"), "{e}");
        assert!(e.contains("not_a_directory"), "the path is named: {e}");
    }

    /// And the walk propagates it: `list` fails rather than returning the
    /// directories it managed to see before the unreadable one.
    #[test]
    fn a_list_walk_that_cannot_read_a_directory_fails_instead_of_answering() {
        let d = girpr_tempdir::TempDir::new("gircap-list-fails", "walk_fails");
        let e = list_with(
            &d,
            |_| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "share offline",
                ))
            },
            |p| std::fs::read(p),
        )
        .expect_err("the walk did not finish");
        assert!(e.contains("cannot read"), "{e}");
        assert!(
            e.contains(&d.display().to_string()),
            "the root is named: {e}"
        );
    }

    /// A manifest the walk cannot **read** is a bundle it could not see, not a
    /// directory that is not a bundle.
    ///
    /// A backup tool, an indexer or an editor can hold `MANIFEST.json` with an
    /// exclusive share. The file is still there and still a file, so the old
    /// `is_file()` probe passed and the failed read in `schema_version_of` was
    /// flattened to "not a bundle": the bundle vanished from the inventory and
    /// `list` exited `0`. The reader is injected because a share lock is
    /// platform-specific; the failure it simulates is the same `fs::read` arm
    /// the real lock takes.
    #[test]
    fn a_list_walk_that_cannot_read_a_manifest_fails_instead_of_dropping_the_bundle() {
        let d = girpr_tempdir::TempDir::new("gircap-list-manifest", "manifest_read");
        let bundle = d.join("hk4e_global").join("7.0.0__7.1.0").join("cap");
        std::fs::create_dir_all(&bundle).expect("makes the bundle directory");
        std::fs::write(bundle.join("MANIFEST.json"), br#"{"schema_version": 2}"#)
            .expect("writes a real manifest");

        let e = list_with(
            &d,
            |p| {
                let rd = std::fs::read_dir(p)?;
                Ok(rd.map(|e| e.map(|e| e.path())).collect())
            },
            |_| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "held by another process",
                ))
            },
        )
        .expect_err("a manifest that cannot be read is not a shorter inventory");
        assert!(e.contains("cannot read"), "{e}");
        assert!(e.contains("MANIFEST.json"), "the manifest is named: {e}");
        assert!(
            e.contains("held by another process"),
            "the cause is kept: {e}"
        );
    }

    /// One failed entry is the whole walk's failure: a child that vanished
    /// between `read_dir` and the stat may be a bundle, so it cannot be dropped.
    #[test]
    fn a_walk_that_loses_one_entry_names_the_directory() {
        let d = girpr_tempdir::TempDir::new("gircap-list-entry", "entry_error");
        let denied = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let e = child_dirs(&d, move |_| Ok(vec![Err(denied)]))
            .expect_err("an entry error is not a shorter listing");
        assert!(e.contains("cannot read an entry"), "{e}");
        assert!(e.contains("denied"), "the cause is kept: {e}");
    }

    /// An entry whose metadata cannot be read is an error too; `is_dir`'s
    /// `false` on a failed stat is the same silent omission one level down.
    #[test]
    fn a_walk_that_cannot_stat_one_entry_names_it() {
        let d = girpr_tempdir::TempDir::new("gircap-list-stat", "stat_error");
        let vanished = d.join("vanished");
        let e = child_dirs(&d, move |_| Ok(vec![Ok(vanished)]))
            .expect_err("a missing entry is not a file");
        assert!(e.contains("cannot read"), "{e}");
        assert!(e.contains("vanished"), "the path is named: {e}");
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
        // The two sets must not overlap. `check_agreement` compares REDERIVED
        // rows against what the capture recorded; a row that is never recorded
        // has nothing to agree with, and listing it in both would have the
        // summary claim a cross-check that cannot happen.
        for id in DERIVED_UNRECORDED {
            assert!(
                !REDERIVED.contains(&id),
                "A{id} is in both sets, so it would be checked against a row no capture writes"
            );
        }
        // And they are disjoint from the ids a capture does record, which is
        // what keeps `check_agreement` honest.
        for id in DERIVED_UNRECORDED {
            assert!(
                !crate::emit::selfcheck::WORDING
                    .iter()
                    .any(|(w, _)| *w == id),
                "A{id} is claimed as never recorded, but a capture records it"
            );
        }
        assert_eq!(
            derived_ids().len(),
            REDERIVED.len() + DERIVED_UNRECORDED.len()
        );
    }

    /// A20 re-derives a patchmap's arithmetic from the patchmap, so it needs no
    /// API body - which is the whole reason it can live here.
    #[test]
    fn a20_is_derived_from_bundle_documents_alone() {
        assert!(DERIVED_UNRECORDED.contains(&A20_ID));
        assert_eq!(A20_ID, "A20");
        assert!(!A20.is_empty());
    }

    /// The set is deliberately smaller than the seventeen.
    #[test]
    fn assertions_needing_the_api_are_not_claimed_as_re_derived() {
        for id in [
            "A1", "A2", "A3", "A4", "A5", "A9", "A10", "A12", "A14", "A15",
        ] {
            assert!(!REDERIVED.contains(&id), "A{id} is listed as re-derived");
        }
    }

    #[test]
    fn a_blob_indexed_but_not_stored_is_red_rather_than_skipped() {
        let index = ChunkIndex::build(vec![]);
        assert!(check_blobs(Path::new("."), &index).is_empty());
        let props =
            crate::blobs::props::BlobProps::unmeasured(&crate::manifest::proto::SophonChunk {
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
        assert!(
            !rows[0].ok,
            "a listed-but-unstored blob cannot be materialized"
        );
        assert!(
            rows[0].actual.contains("nothing was stored"),
            "{}",
            rows[0].actual
        );
    }

    #[test]
    fn a_blob_whose_bytes_were_altered_turns_its_row_red() {
        let d = girpr_tempdir::TempDir::new("gircap-m9-blob", "altered_bytes");
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
            crate::blobs::IndexedBlob {
                props,
                referenced_by: Vec::new(),
                sets: Vec::new(),
            },
            Vec::new(),
        )]);
        assert!(
            check_blobs(&d, &index)[0].ok,
            "the good bundle's row is green"
        );

        // Flip one byte inside the frame: it still decodes, to different bytes.
        let mut bad = raw.clone();
        let last = bad.len() - 1;
        bad[last] ^= 0xFF;
        std::fs::write(d.join("chunks/ab/cd"), &bad).unwrap();
        let row = &check_blobs(&d, &index)[0];
        assert!(!row.ok, "one altered byte turns the row red");
        assert_eq!(row.subject, "chunks/ab/cd", "and names the blob");
        assert!(row.id == "A6", "and the assertion: {}", row.id);
    }

    #[test]
    fn a_status_the_reader_cannot_agree_with_is_not_counted_as_ok() {
        assert!(crate::emit::selfcheck::MatrixRow::ok_status("captured"));
        assert!(crate::emit::selfcheck::MatrixRow::ok_status("covered_by"));
        assert!(crate::emit::selfcheck::MatrixRow::ok_status("over_budget"));
        assert!(crate::emit::selfcheck::MatrixRow::ok_status("unavailable"));
        assert!(!crate::emit::selfcheck::MatrixRow::ok_status(
            "something else"
        ));
    }
}
