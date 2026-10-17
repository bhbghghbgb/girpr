//! The close-out: the four documents that can only be written once the tree is
//! finished.
//!
//! **Lived inline in `run::capture`,** where it was 156 lines between the last
//! blob write and the summary. That put the one piece of the run with a real
//! ordering constraint — and the one piece with a two-pass fixpoint — in the
//! middle of a function whose job was to decide what a capture *does*.
//!
//! # Why this is a module and not three more `emit::` functions
//!
//! Every function here was already a function, and all of them could already be
//! called in the wrong order. The ordering is not a detail of any single call:
//!
//! ```text
//!   rows ──► selfcheck.json ──► A16 ──┐
//!           MANIFEST.json               │  checksums.json must cover
//!           README.md                   │  selfcheck.json, so it can
//!           checksums.json ◄────────────┘  only be written after A16 is known
//! ```
//!
//! `checksums.json` has to cover `selfcheck.json`; A16 asks whether
//! `checksums.json` covers everything; so A16's row changes `selfcheck.json`,
//! which invalidates `checksums.json`. [`close`] owns that loop. Split across
//! callers it would be four `emit::` calls in the right order, which is the same
//! thing said more weakly.

use std::collections::BTreeMap;
use std::path::Path;

use crate::failure::{Failure, apierr, write_err};
use crate::http::ApiError;

use super::index;
use super::selfcheck::{Assertion, Selfcheck};

/// Everything the close-out needs from the orchestrator.
///
/// `rows` is owned and the rest is borrowed, because the close-out is the last
/// thing that runs: nothing after it wants these, so nothing has to outlive it.
pub struct CloseoutInput<'a> {
    /// The artifact root. Everything below is written under it.
    pub dir: &'a Path,
    /// The rows the assembly phases produced. A13-A17 are appended here.
    pub rows: Vec<Assertion>,
    /// How to build `MANIFEST.json`, including the file listing.
    pub manifest: index::ManifestInput<'a>,
    /// The `--old-tag` A13 checks `config.ini` against.
    pub old_tag: &'a str,
    /// The `scan/` file A14 reads.
    pub scan_dir: &'a str,
    /// The selection, for A14's "nothing but" check.
    pub selection: &'a crate::select::Selection,
    /// The diffs, for A15.
    pub diffs: &'a [crate::manifest::diff::FieldDiff],
    /// Per-capture `(old, new)` whole-file digests, for A19.
    pub side_digests: &'a BTreeMap<String, (String, String)>,
    /// The run's A6-A12 and A19 rows.
    ///
    /// Appended to as each document lands, because A14 asserts that every file
    /// under `game/` was one the run planned — and a document the close-out
    /// itself wrote would fail that check if it were not declared first.
    pub written: Vec<String>,
    /// `--no-chunk-blobs`, which declares A6-A12 and A19 skipped rather than
    /// asserted.
    pub no_chunk_blobs: bool,
}

/// What the close-out produced.
pub struct Closed {
    /// `MANIFEST.json`, as written.
    pub manifest: index::Manifest,
    /// `verify/checksums.json`, as written by the **second** pass.
    pub checksums: index::Checksums,
    /// `verify/selfcheck.json`, final.
    pub selfcheck: Selfcheck,
    /// The documents written, in the order they were written.
    pub written: Vec<String>,
    /// How many rows the run had when the close-out received them — the
    /// `assembly_selfcheck_rows` the summary reports.
    ///
    /// Returned rather than kept, because the vector that produced it is moved
    /// into [`Selfcheck::build`] and the count is the only part the caller still
    /// wants.
    pub assembly_rows: u64,
}

/// Write `selfcheck.json`, `MANIFEST.json`, `README.md` and `checksums.json`,
/// and settle A16.
///
/// The order is load-bearing and is the only ordering that makes each document
/// complete:
///
/// - A13-A17 first, because they are about the tree that now exists;
/// - then `selfcheck.json`, `MANIFEST.json` and `README.md`, which are about
///   each other;
/// - then `checksums.json` last, because it must cover all of them.
///
/// **A16 is the one that cannot be settled in that order.** It asks whether
/// `checksums.json` covers everything, and `checksums.json` does not exist yet.
/// So it is checked after the first write and its row appended, which means
/// `selfcheck.json` and `checksums.json` are each written twice. Two passes
/// converge exactly: the second `checksums.json` is scanned after the final
/// `selfcheck.json`, so what it records is what is on disk. A third pass would
/// find nothing new, which is the proof it converged — and
/// `two_passes_converge` is that proof, as a test.
///
/// The second pass is **unconditional.** It used to be guarded by
/// `if !selfcheck.by_id().contains_key("A16")`, which could never be false:
/// `selfcheck` is built from `rows`, and A16 is pushed *inside* the guard. A
/// conditional that always fires is worse than none — it reads as "this might
/// converge in one pass", and the comment above the pair is what actually
/// guarantees it.
pub fn close(mut i: CloseoutInput<'_>) -> Result<Closed, Failure> {
    let mut written = std::mem::take(&mut i.written);
    // A13-A15: about the tree that just exists. A16 and A17 are added below,
    // because both are about documents that do not exist yet.
    i.rows.extend(super::selfcheck::closing(
        i.dir,
        i.old_tag,
        i.scan_dir,
        i.selection,
        i.diffs,
        i.side_digests,
        &written,
        i.no_chunk_blobs,
    ));
    super::selfcheck::report_failed(&i.rows);

    // The close-out's **one** full-tree digest pass. It used to be a fourth
    // one, in a private function that also re-implemented `Checksums::scan`'s
    // walk and inlined its `encoding_of` — so a third encoding would have
    // taught `MANIFEST.json` the wrong one while `checksums.json` learned the
    // right one.
    //
    // It runs *before* `build_manifest` rather than after. Nothing between here
    // and the call writes to the tree, so the pass sees the same bytes it
    // always did — but the manifest is now built whole, instead of being handed
    // back with an empty `files[]` for two later statements to fill in.
    let (listed, seeded) = super::write_manifest_files(i.dir).map_err(|e| write_err(i.dir, e))?;
    let manifest =
        index::build_manifest(&i.manifest, &listed).map_err(|e| {
            apierr("build MANIFEST.json", &ApiError::shape("manifest", e))
        })?;

    // A17 is about the matrix this document carries, so it is settled here and
    // appended to the rows the rest of the close-out will write.
    let (a17_ok, a17_detail) = manifest.a17();
    i.rows.push(Assertion::row(
        "A17",
        "MANIFEST.json matrix",
        "every class captured | covered_by | unavailable",
        a17_ok,
        &a17_detail,
    ));

    // A6 joins A7-A12 here: with no blobs there is nothing to measure, nothing
    // to assemble and nothing to prove. A declared gap is `incomplete`, not
    // `fail`. The list is `selfcheck::SKIPPED_WITHOUT_CHUNK_BLOBS` rather than a
    // literal, and `skipped_were_declared_not_asserted` is what keeps it equal
    // to what the document derives from the rows.
    let skipped: Vec<&str> = if i.no_chunk_blobs {
        super::selfcheck::SKIPPED_WITHOUT_CHUNK_BLOBS.to_vec()
    } else {
        Vec::new()
    };
    let matrix_rows: Vec<super::selfcheck::MatrixRow> = manifest
        .matrix
        .iter()
        .map(|m| super::selfcheck::MatrixRow {
            class: m.class.clone(),
            status: m.status.clone(),
            // One definition of "an acceptable state", not a second copy of the
            // literal: the A17 row and the selfcheck's matrix row must not be
            // able to disagree about what `ok` means.
            ok: super::selfcheck::MatrixRow::ok_status(&m.status),
        })
        .collect();

    // Counted here because the vector is **moved** below. It used to be
    // `rows.clone()`, so that one integer on the summary could read `rows.len()`
    // afterwards — a deep copy of every assertion id, subject, expected and
    // actual string, for one `usize`.
    let assembly_rows = i.rows.len() as u64;

    let mut selfcheck = Selfcheck::build(i.rows, matrix_rows, &skipped);
    super::write_selfcheck(i.dir, &selfcheck).map_err(|e| write_err(i.dir, e))?;
    written.push(super::write_manifest(i.dir, &manifest).map_err(|e| write_err(i.dir, e))?);
    written.push(
        super::write_readme(i.dir, &manifest, &selfcheck).map_err(|e| write_err(i.dir, e))?,
    );
    let (_, checksums) =
        super::write_checksums_reusing(i.dir, &seeded, &[]).map_err(|e| write_err(i.dir, e))?;
    // The independent half of A16: re-reads every listed file from disk and
    // compares. This is deliberately **not** folded into the scan above — a
    // re-hash that reuses the digests it is checking would be a tautology.
    let (a16_ok, a16_detail) = checksums.a16(i.dir);
    written.push("verify/checksums.json".to_string());

    // Second pass: record A16, then re-write the two documents it changed.
    // The re-scan sees the final `selfcheck.json`, so the second
    // `checksums.json` is the authoritative one.
    selfcheck.assertions.push(Assertion::row(
        "A16",
        "verify/checksums.json",
        "covers every file under the artifact root, and re-hashing matches",
        a16_ok,
        &a16_detail,
    ));
    selfcheck = Selfcheck::build(selfcheck.assertions, selfcheck.matrix, &skipped);
    super::write_selfcheck(i.dir, &selfcheck).map_err(|e| write_err(i.dir, e))?;
    // Only `selfcheck.json` moved since the scan above, so only it is
    // re-digested.
    let (_, checksums) =
        super::write_checksums_reusing(i.dir, &checksums.entries, &["verify/selfcheck.json"])
            .map_err(|e| write_err(i.dir, e))?;

    tracing::info!(
        selfcheck = ?selfcheck.result,
        total = selfcheck.counts.total,
        passed = selfcheck.counts.passed,
        failed = selfcheck.counts.failed,
        a16_ok,
        a16_detail,
        files = manifest.files.len(),
        checksummed = checksums.file_count,
        bytes_on_disk = manifest.totals.bytes_on_disk,
        "artifact written"
    );

    Ok(Closed {
        manifest,
        checksums,
        selfcheck,
        assembly_rows,
        written,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("gircap-closeout-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("game")).expect("scratch root");
        std::fs::create_dir_all(p.join("scan")).expect("scan dir");
        p
    }

    /// A tree that looks enough like a bundle's for the close-out to run: a
    /// `game/` with a config and one captured file, and a `scan/` file.
    ///
    /// Deliberately small. The convergence argument is about *which documents
    /// are written when*, and that does not depend on how many blobs there are -
    /// so a two-file tree tests it as well as a ninety-file one and runs in
    /// microseconds.
    fn seed(root: &Path) {
        std::fs::write(
            root.join("game").join("config.ini"),
            "[General]\ngame_version=7.0.0\n",
        )
        .unwrap();
        std::fs::write(root.join("game").join("data.bin"), b"patched body").unwrap();
        std::fs::write(root.join("scan").join("scan.txt"), "game\n").unwrap();
    }



    fn manifest_input<'a>(
        fixture: &'a Fixture,
    ) -> index::ManifestInput<'a> {
        index::ManifestInput {
            started: fixture.started,
            biz: &fixture.biz,
            endpoints: &fixture.endpoints,
            versions: &fixture.versions,
            diff_tags: &[],
            audio: &[],
            fields: &fixture.fields,
            gzip_threshold: 1024,
            diffs: &[],
            selection: &fixture.selection,
            index: &fixture.index,
            bytes_downloaded: 0,
            argv: &fixture.argv,
            complete: true,
        }
    }

    /// Everything the close-out borrows, owned.
    ///
    /// Spelled out rather than derived from `Default`, so a new field on
    /// `ManifestInput` or `Selection` is a compile error here instead of a
    /// silently-defaulted value in a test about write ordering.
    struct Fixture {
        started: chrono::DateTime<chrono::Utc>,
        biz: crate::biz::Biz,
        endpoints: crate::biz::Endpoints<'static>,
        versions: crate::config::VersionPair,
        selection: crate::select::Selection,
        index: crate::blobs::ChunkIndex,
        fields: Vec<String>,
        argv: Vec<String>,
    }

    fn fixture() -> Fixture {
        Fixture {
            // A fixed instant, so `capture.id` and `captured_at` are stable and a
            // failure is reproducible.
            started: chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
                .expect("fixture instant")
                .with_timezone(&chrono::Utc),
            biz: crate::biz::Biz::resolve(crate::cli::Biz::Hk4eGlobal),
            endpoints: crate::biz::Endpoints::owned(
                "https://hyp.example".to_string(),
                "https://sophon.example".to_string(),
            ),
            versions: crate::config::VersionPair {
                old: crate::config::VersionRef {
                    tag: "7.0.0".to_string(),
                    build_id: "old".to_string(),
                    source: "diff_tags[0]".to_string(),
                },
                new: crate::config::VersionRef {
                    tag: "7.1.0".to_string(),
                    build_id: "new".to_string(),
                    source: "main.tag".to_string(),
                },
                diff_tags: vec![],
            },
            selection: crate::select::Selection {
                admitted: Vec::new(),
                rejected: Vec::new(),
                forced_skipped: Vec::new(),
                matrix: Vec::new(),
                bytes_admitted: 0,
                budget: crate::select::Budget::unlimited(),
            },
            index: crate::blobs::ChunkIndex {
                schema_version: crate::SCHEMA_VERSION,
                blobs: Vec::new(),
                sets: BTreeMap::new(),
            },
            fields: vec!["game".to_string()],
            argv: vec!["gircap".to_string(), "capture".to_string()],
        }
    }

    /// Run the close-out over a freshly seeded tree.
    fn run(root: &Path) -> Closed {
        let f = fixture();
        close(CloseoutInput {
            dir: root,
            rows: Vec::new(),
            manifest: manifest_input(&f),
            old_tag: "7.0.0",
            scan_dir: "scan",
            selection: &f.selection,
            diffs: &[],
            side_digests: &BTreeMap::new(),
            written: vec!["game/config.ini".to_string()],
            no_chunk_blobs: true,
        })
        .expect("close-out succeeds on a seeded tree")
    }

    #[test]
    fn two_passes_converge_and_the_pass_the_comment_claims_would_change_nothing() {
        // **The claim this module exists to make checkable.** `close`'s doc says
        // two passes converge exactly and that a third would find nothing new.
        // While this was inline in `capture()` that was a sentence in a comment
        // with no way to test it — the only way to reach the second pass was to
        // run a whole capture against a live API.
        //
        // Two halves, because either alone would pass on a broken fixpoint:
        //
        // 1. What the second pass *recorded* must match the bytes on disk. This
        //    is `a16`, and it is the half that catches a stale `selfcheck.json`
        //    digest.
        // 2. A **fresh, non-reusing** rescan must produce exactly the entries
        //    that were recorded. Reuse is what a second pass does, so a rescan
        //    that also reused would agree with a stale document and prove
        //    nothing; hashing from scratch cannot.
        let root = scratch("converge");
        seed(&root);
        let closed = run(&root);

        // (1) The recorded digests match what is on disk.
        let (ok, detail) = closed.checksums.a16(&root);
        assert!(ok, "the second pass recorded digests that do not match disk: {detail}");

        // (2) Hashing the tree from scratch reproduces the recorded entries.
        let fresh = index::Checksums::scan(&root).expect("independent rescan");
        assert_eq!(
            fresh.entries, closed.checksums.entries,
            "an independent rescan disagrees with what the second pass recorded: \
             the fixpoint did not converge"
        );
        assert_eq!(
            fresh.total_bytes, closed.checksums.total_bytes,
            "and the byte total moved, so the document would need a third pass"
        );
        assert_eq!(
            fresh.file_count, closed.checksums.file_count,
            "and the file count moved"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a16_is_recorded_exactly_once_despite_two_passes() {
        // The second pass pushes A16 onto a `Selfcheck` that was already built
        // and re-builds it. If `Selfcheck::build` ever appended rather than
        // replacing, A16 would appear twice — and the guard that used to wrap the
        // second pass was, as its comment says, a conditional that could never be
        // false. This is the check that would have caught it.
        let root = scratch("a16-once");
        seed(&root);
        let closed = run(&root);

        let a16: Vec<&crate::emit::selfcheck::Assertion> = closed
            .selfcheck
            .assertions
            .iter()
            .filter(|a| a.id == "A16")
            .collect();
        assert_eq!(
            a16.len(),
            1,
            "A16 must appear once, not once per pass: {:?}",
            a16.iter().map(|a| a.actual.as_str()).collect::<Vec<_>>()
        );
        assert!(a16[0].ok, "A16 should pass on a tree nobody tampered with");

        // And it must be in the document on disk too, not only in the returned
        // struct — the two can diverge if the final write is ever skipped.
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("verify/selfcheck.json")).unwrap())
                .unwrap();
        let ids: Vec<&str> = written["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids.iter().filter(|i| **i == "A16").count(),
            1,
            "A16 on disk: {ids:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn checksums_covers_the_documents_the_close_out_itself_wrote() {
        // The ordering is the whole point of this module: `selfcheck.json`,
        // `MANIFEST.json`, `README.md` and `checksums.json` are all written
        // after the tree they describe, and `checksums.json` must list the first
        // three. If the order were wrong — checksums written first — the file
        // would be complete and consistent and still omit three documents,
        // which is precisely the failure A16 exists to catch.
        let root = scratch("coverage");
        seed(&root);
        let closed = run(&root);

        let listed: Vec<&str> = closed
            .checksums
            .entries
            .iter()
            .map(|e| e.path.as_str())
            .collect();
        for required in [
            "verify/selfcheck.json",
            "MANIFEST.json",
            "README.md",
        ] {
            assert!(
                listed.contains(&required),
                "checksums.json omits {required}; it lists {listed:?}"
            );
        }
        // `checksums.json` cannot list itself, for the same reason MANIFEST
        // cannot: it is still being written.
        assert!(
            !listed.contains(&"verify/checksums.json"),
            "a checksums document that lists its own digest cannot be verified"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_chunk_blobs_declares_the_gap_rather_than_asserting_it() {
        // `--no-chunk-blobs` must produce `incomplete`, not `fail`. The row set
        // is what a reader sees, so a declared gap that showed up as a red row
        // would tell them the bundle is broken when it is honestly incomplete.
        let root = scratch("skip");
        seed(&root);
        let closed = run(&root);

        assert_eq!(
            closed.selfcheck.result,
            super::super::selfcheck::Result_::Incomplete,
            "a declared gap is incomplete"
        );
        let skipped: Vec<&str> = closed.selfcheck.skipped();
        for id in ["A6", "A7", "A8", "A9", "A10", "A11", "A12", "A19"] {
            assert!(skipped.contains(&id), "{id} should be declared skipped");
        }
        // A declared gap must not also appear as a failing row.
        let failing: Vec<&str> = closed
            .selfcheck
            .assertions
            .iter()
            .filter(|a| !a.ok)
            .map(|a| a.id.as_str())
            .collect();
        assert!(failing.is_empty(), "no row should fail: {failing:?}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_tampered_file_after_the_close_out_is_caught_by_a_second_a16() {
        // A16's value is that it re-reads from disk rather than trusting the
        // scan that produced it. Here the tamper happens *after* `close`, which
        // is the only moment it can be caught — so the test re-runs A16 against
        // the recorded checksums and requires a finding.
        let root = scratch("tamper");
        seed(&root);
        let closed = run(&root);

        let before = closed.checksums.a16(&root);
        assert!(before.0, "clean tree: {before:?}");

        std::fs::write(root.join("game").join("data.bin"), b"tampered").unwrap();
        let after = closed.checksums.a16(&root);
        assert!(!after.0, "a changed file must fail A16");
        assert!(
            after.1.contains("game/data.bin"),
            "the finding must name the file: {}",
            after.1
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
