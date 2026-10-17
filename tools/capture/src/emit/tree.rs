//! Writing the part of the tree that exists before any chunk arrives.
//!
//! **Lived inline in `run::capture`,** as ~34 lines between the selection and
//! the chunk store. All three writes happen even though the run is incomplete:
//! what was asked, what it cost, and what came back are the parts that stay
//! useful, and the bytes are the evidence behind every number in the summary.
//!
//! # Why this is one function
//!
//! Not because the four writes belong together aesthetically, but because they
//! share a **`written` list** that A14 and the close-out both read, and because
//! they share a rule about how that list is built: it is counted from what each
//! writer returned, never restated by hand. A hand-written count goes stale
//! silently the moment a document is added, and nothing fails when it does —
//! which is the kind of bug that survives every test in this crate.
//!
//! # Why `manifests` is borrowed mutably
//!
//! [`manifest::mark_stored`] has to run after each archive is written, because
//! only then is it known where the archive ended up. It records that on the
//! manifest, and the chunk store's [`crate::blobs::Sources`] reads it back to
//! build chunk URLs. A caller that wrote the manifests and forgot to mark them
//! would produce a chunk index whose URLs do not resolve — so the marking is a
//! step here rather than a suggestion in a doc comment.

use std::path::Path;

use crate::failure::{Failure, write_err};
use crate::http::probe::{ErrorShapes, TagProbe};
use crate::http::RequestLog;
use crate::hyp::Captured;
use crate::manifest::{self, CapturedManifest, diff};

/// Everything the pre-chunk tree needs.
pub struct TreeInput<'a> {
    /// The artifact root. Everything is written under it.
    pub dir: &'a Path,
    /// The captured API bodies.
    pub captured: &'a Captured,
    /// The tag probe's answer.
    pub probe: &'a TagProbe,
    /// The negative probes' answer.
    pub error_shapes: &'a ErrorShapes,
    /// Every field's manifests, marked in place as they are stored.
    pub manifests: &'a mut [Vec<CapturedManifest>],
    /// The request log, which is what can say where an archive ended up.
    pub log: &'a RequestLog,
    /// The `--gzip-threshold` for the manifest archives.
    pub gzip_threshold: u64,
    /// `--manifest-detail`: how much of each manifest to decode.
    pub manifest_detail: crate::cli::ManifestDetail,
    /// The aggregate the summary reports.
    pub agg: &'a diff::PathDiff,
    /// The per-field diffs, for the TSV.
    pub diffs: &'a [diff::FieldDiff],
    /// The selection, for `diff/candidates.json`.
    pub selection: &'a crate::select::Selection,
    /// `--per-class`, as the selection was given it.
    pub per_class: usize,
}

/// What the pre-chunk tree contains.
pub struct Tree {
    /// Every path written, in the order they were written.
    pub written: Vec<String>,
    /// How many of those are `api/` documents.
    ///
    /// Counted from what [`super::write_api`] returned, never restated.
    pub api_documents: u64,
}

impl Tree {
    /// Every path written, for appending to a longer run's list.
    pub fn into_paths(self) -> Vec<String> {
        self.written
    }
}

/// Write `api/`, the manifest archives, `diff/` and `diff/candidates.json`.
pub fn write(i: TreeInput<'_>) -> Result<Tree, Failure> {
    let mut written =
        super::write_api(i.dir, i.captured, i.probe, i.error_shapes).map_err(|e| write_err(i.dir, e))?;
    // Counted from what the writer returned, never restated: a hand-written
    // count goes stale silently the moment a document is added, and nothing
    // fails when it does.
    let api_documents = written.len() as u64;

    let manifests = i.manifests;
    for pair in manifests.iter_mut() {
        for m in pair.iter_mut() {
            let stored =
                manifest::dump::write(i.dir, m, i.gzip_threshold, i.manifest_detail)
                    .map_err(|e| write_err(i.dir, e))?;
            // `stored_as` is filled in here rather than by the writer: only now is
            // it known where the archive ended up, and only the request log can
            // say it.
            manifest::mark_stored(i.log, m, &stored[0].path);
            written.extend(stored.into_iter().map(|s| s.path));
        }
    }

    // `diffs[0].pool` is the reuse measurement the summary reports, so the pool
    // document and the summary cannot describe different measurements.
    written.extend(
        super::write_diff(i.dir, i.agg, &i.diffs[0].pool, i.diffs)
            .map_err(|e| write_err(i.dir, e))?,
    );
    written.extend(
        super::write_selection(
            i.dir,
            &crate::select::candidates_document(i.diffs),
            i.selection,
            i.per_class,
        )
        .map_err(|e| write_err(i.dir, e))?,
    );

    Ok(Tree {
        written,
        api_documents,
    })
}

// No unit tests here, deliberately.
//
// Every input is a captured HTTP response or a decoded manifest, so a test would
// be almost entirely fixture and would assert about a scratch directory rather
// than about this function's decisions. The two rules this module holds — the
// order of the four writes, and `api_documents` counted from the writer's
// return — are both covered end to end by the loopback integration suites in
// `tests/`, which run real captures against a mock API and compare the tree.
//
// An earlier draft of this file had a test that read its own source with
// `include_str!` and asserted a particular line was present. That is not a test
// of behaviour; it is a test that the file was not edited, dressed up as one,
// and it would have passed against a `write` that wrote nothing at all.

