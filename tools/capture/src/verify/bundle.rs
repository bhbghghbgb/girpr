//! Loading a bundle from disk: `MANIFEST.json`, the chunk index, the patchmaps,
//! and every diagnostics document, with the path-shape rules checked at the door.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::assemble::PatchMap;
use crate::blobs::ChunkIndex;
use crate::emit::index::{Checksums, Manifest};
use crate::emit::selfcheck::Selfcheck;
use crate::http::probe::ErrorShapes;
use crate::pathmap::PathMapDocument;
use crate::rejections::RejectionsDocument;

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
    /// `api/error_responses.json` - the incidental non-2xx bodies the run saw,
    /// with the bytes that were retained in `api/error_bodies/`. Required for the
    /// same reason as `error_shapes.json`: a bundle that recorded only successes
    /// has lost what the origin said when it refused.
    pub error_responses: crate::http::ErrorResponses,
    /// `diagnostics/rejections.json` — every value a shape rule refused, with
    /// its source and rule. Required: a bundle that recorded only what it wrote
    /// has lost the API's own attempts to name things it would not hold.
    pub rejections: RejectionsDocument,
    /// `diagnostics/path_map.json` — every logical path written under an
    /// escaped on-disk name, such as a Windows device component. Required: a
    /// reader that materializes `game/`/`expected/` must follow it.
    pub path_map: PathMapDocument,
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
        // Arithmetic on the index's offsets and sizes is checked before any
        // command assembles a byte: a crafted `offset` must not reach
        // `Layout::of` as a panic. See `ChunkIndex::validate_arithmetic`.
        index.validate_arithmetic()?;
        let checksums: Checksums = read_json(&root.join("verify/checksums.json"))?;
        let recorded: Selfcheck = read_json(&root.join("verify/selfcheck.json"))?;
        let error_shapes: ErrorShapes = read_json(&root.join("api/error_shapes.json"))?;
        let error_responses: crate::http::ErrorResponses =
            read_json(&root.join("api/error_responses.json"))?;
        let rejections: RejectionsDocument = read_json(&root.join("diagnostics/rejections.json"))?;
        let path_map: PathMapDocument = read_json(&root.join(crate::pathmap::DOCUMENT))?;
        let patchmaps = read_patchmaps(root)?;
        // **The shape of every document-supplied path is checked at the door.**
        // `verify` is the command an operator runs on a bundle they may not have
        // produced, and every join below this point is `root.join(document
        // field)`. Checking at the individual joins caught only the sinks that
        // happened to remember; a field read by A6, by A18 or by `materialize`
        // through the patchmap was still a way out. One pass here names the file
        // and the field before any command reads a byte.
        validate_document_paths(&manifest, &index, &patchmaps, &error_responses, &path_map)?;
        Ok(Bundle {
            root: root.to_path_buf(),
            manifest,
            index,
            patchmaps,
            checksums,
            recorded,
            error_shapes,
            error_responses,
            rejections,
            path_map,
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
/// with two irreconcilable descriptions, and `BTreeMap::insert` would drop the
/// second silently, so this is an explicit error.
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
    error_responses: &crate::http::ErrorResponses,
    path_map: &PathMapDocument,
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
            // A reference is a **logical** path: the game's spelling, which may
            // name a device component. The on-disk twin is the escaped
            // `stored_path`, checked above.
            if let Some(problem) = crate::paths::logical_path_problem(&reference.path) {
                return Err(format!(
                    "chunks/index.json blob {} referenced_by path {problem}",
                    blob.props.chunk_id
                ));
            }
        }
    }
    for map in patchmaps.values() {
        if let Some(problem) = crate::paths::logical_path_problem(&map.path) {
            return Err(format!("patchmaps/{}.json path {problem}", map.capture_id));
        }
    }
    for row in &error_responses.responses {
        if let Some(rel) = row.stored_path.as_deref()
            && let Some(problem) = crate::paths::relative_path_problem(rel)
        {
            return Err(format!(
                "api/error_responses.json seq {} stored_path {problem}",
                row.seq
            ));
        }
    }
    // The mapping document is the reader's authority for escaped names, so it
    // must itself be safe and agree with the rule the writer used. A logical
    // path may name a device; the on-disk path may not, and must be exactly the
    // escape of the logical one — a map that redirected a read elsewhere would
    // be a way past every join above.
    for entry in &path_map.entries {
        if let Some(problem) = crate::paths::logical_path_problem(&entry.logical) {
            return Err(format!("diagnostics/path_map.json logical path {problem}"));
        }
        if let Some(problem) = crate::paths::relative_path_problem(&entry.on_disk) {
            return Err(format!("diagnostics/path_map.json on_disk path {problem}"));
        }
        let expected = crate::paths::escape_path(&entry.logical);
        if entry.on_disk != expected {
            return Err(format!(
                "diagnostics/path_map.json maps {:?} to {:?}, but the rule gives {:?}",
                entry.logical, entry.on_disk, expected
            ));
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
#[derive(Debug)]
pub enum ManifestProbe {
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
pub fn probe_manifest(
    path: &Path,
    read: impl FnOnce(&Path) -> std::io::Result<Vec<u8>>,
) -> Result<ManifestProbe, String> {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => {}
        // A `MANIFEST.json` that is not a file is not a manifest: the directory
        // is not a bundle, and the walk keeps descending.
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
