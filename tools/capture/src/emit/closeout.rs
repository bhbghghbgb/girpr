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

use crate::failure::{Failure, apierr, assertion, write_err};
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
    /// The patch shape of each capture, for A21-A23.
    pub shapes: &'a [crate::assemble::PatchShape],
    /// `--no-chunk-blobs`, which declares A6-A12, A19 and A21-A23 skipped
    /// rather than asserted.
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
    /// The failure this bundle's own rows earn, if any.
    ///
    /// **Returned rather than raised, deliberately.** By the time this is known
    /// the bundle is complete and internally consistent, and the caller has not
    /// yet built the run's summary - which is where the numbers about the *run*
    /// live, as opposed to the numbers about the artifact that are in the
    /// bundle. Raising here would discard the run's own account of itself at
    /// exactly the moment it became interesting.
    ///
    /// So `close` reports and `capture` decides, after the summary exists: the
    /// summary is attached to the failure and both are printed. See
    /// [`crate::failure::Failure::preceded_by`].
    pub fatal: Option<Failure>,
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
/// `selfcheck.json`, `README.md` and `checksums.json` are each written twice.
/// Two passes converge exactly: the second `checksums.json` is scanned after the
/// final `selfcheck.json` and `README.md`, so what it records is what is on
/// disk. A third pass would find nothing new, which is the proof it converged —
/// and `two_passes_converge` is that proof, as a test.
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
    // A21-A23: the bundle's coverage. They are about the patchmaps, and the
    // patchmaps are written, so they are settled here with the rest of the
    // closing rows.
    if !i.no_chunk_blobs {
        i.rows.extend(super::selfcheck::coverage(i.shapes));
    }
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
    let manifest = index::build_manifest(&i.manifest, &listed)
        .map_err(|e| apierr("build MANIFEST.json", &ApiError::shape("manifest", e)))?;

    // A17 is about the matrix this document carries, so it is settled here and
    // appended to the rows the rest of the close-out will write.
    let (a17_ok, a17_detail) = manifest.a17();
    i.rows.push(Assertion::row(
        "A17",
        "MANIFEST.json matrix",
        "every class captured | covered_by | over_budget | unavailable",
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
    written
        .push(super::write_readme(i.dir, &manifest, &selfcheck).map_err(|e| write_err(i.dir, e))?);
    let (_, checksums) =
        super::write_checksums_reusing(i.dir, &seeded, &[]).map_err(|e| write_err(i.dir, e))?;
    // The independent half of A16: re-reads every listed file from disk and
    // compares. This is deliberately **not** folded into the scan above — a
    // re-hash that reuses the digests it is checking would be a tautology.
    let (a16_ok, a16_detail) = checksums.a16(i.dir);
    written.push("verify/checksums.json".to_string());

    // Second pass: record A16, then re-write the two documents it changed — and
    // **only** those two, because they are the only two the A16 row moves.
    //
    // `README.md` is one of them because it renders the selfcheck's own counts,
    // and writing it before the A16 row exists put "115 of 115" in a bundle whose
    // `selfcheck.json` said 116. Its final size does not depend on anything the
    // second scan measures — the counts are a row tally, not a byte total — so
    // the fixpoint below is still exact rather than converging in practice.
    selfcheck.assertions.push(Assertion::row(
        "A16",
        "verify/checksums.json",
        "covers every file under the artifact root, and re-hashing matches",
        a16_ok,
        &a16_detail,
    ));
    selfcheck = Selfcheck::build(selfcheck.assertions, selfcheck.matrix, &skipped);
    super::write_selfcheck(i.dir, &selfcheck).map_err(|e| write_err(i.dir, e))?;
    super::write_readme(i.dir, &manifest, &selfcheck).map_err(|e| write_err(i.dir, e))?;
    let (_, checksums) = super::write_checksums_reusing(
        i.dir,
        &checksums.entries,
        &["verify/selfcheck.json", "README.md"],
    )
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

    // **After every write, deliberately, and reported rather than raised.** A
    // failed row is settled by checking the bundle against itself, so aborting
    // before `checksums.json` landed would leave a tree where A16 had failed
    // *because the close-out stopped*, and the operator would read a cascade as
    // a cause. The bundle on disk is the evidence; exit 3 says it is not
    // trustworthy, not that it was withheld.
    let fatal = fatal_failure(&selfcheck).map(|f| {
        assertion(
            "the assertions this bundle makes about itself",
            &ApiError::shape(&f.id, f_detail(f)),
        )
    });

    Ok(Closed {
        manifest,
        checksums,
        selfcheck,
        assembly_rows,
        written,
        fatal,
    })
}

/// The first failing row that ends the run, if there is one.
///
/// **`is_fatal` is the decision, not a lookup with no effect.** The function was
/// documented as deciding whether an assertion's failure ends the run and was
/// called from nowhere, while [`close`] made that decision by hand — which meant
/// the table could not be narrowed without changing nothing at all, and a
/// reader had no way to tell a table that governed behaviour from a comment.
///
/// `None` here is the `exit 0` with `result: incomplete` case: a *skipped* id
/// has no row and so cannot fail, and a run that skipped A6-A12 under
/// `--no-chunk-blobs` is honest rather than broken.
fn fatal_failure(s: &Selfcheck) -> Option<&Assertion> {
    s.assertions
        .iter()
        .find(|a| !a.ok && super::selfcheck::is_fatal(a.id.as_str()))
}

/// The failure's own words, for the `ApiError` shape.
fn f_detail(a: &Assertion) -> String {
    format!(
        "{} on {}: expected {}, got {}",
        a.id, a.subject, a.expected, a.actual
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> girpr_tempdir::TempDir {
        let p = girpr_tempdir::TempDir::new("gircap-closeout", name);
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

    fn manifest_input<'a>(fixture: &'a Fixture) -> index::ManifestInput<'a> {
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
            shapes: &[],
            written: vec!["game/config.ini".to_string()],
            no_chunk_blobs: true,
        })
        .expect("close-out succeeds on a seeded tree")
    }

    #[test]
    fn the_generated_readme_agrees_with_the_selfcheck_it_renders() {
        // **The defect this is here for.** `README.md` is a rendering of
        // `selfcheck.json`, and it was written *before* A16 was appended — so a
        // bundle said "115 of 115 assertions passed" in one document and `116`
        // in the other. A generated file that disagrees with its source is worse
        // than no file, because it is the one a reader opens first.
        //
        // Read both off disk, not off the returned struct: the pair that
        // disagreed was the bytes written, and a test that compared the two
        // in-memory values would have passed throughout.
        let root = scratch("readme-counts");
        seed(&root);
        let closed = run(&root);

        let readme = std::fs::read_to_string(root.join("README.md")).unwrap();
        let on_disk: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("verify/selfcheck.json")).unwrap())
                .unwrap();

        let counts = &on_disk["counts"];
        let total = counts["total"].as_u64().expect("counts.total");
        let passed = counts["passed"].as_u64().expect("counts.passed");
        assert_eq!(
            closed.selfcheck.counts.total, total,
            "the returned struct and the document already disagree"
        );
        assert!(
            readme.contains(&format!("{passed} of {total} assertions passed")),
            "README.md must carry the document's own counts:\n{readme}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_generated_readme_names_the_flag_that_verifies_the_bundle() {
        // The re-verify instruction is the one line in a bundle that a reader is
        // most likely to type verbatim, and it was naming a positional argument
        // that does not exist. `gircap verify <dir>` exits 1.
        let root = scratch("readme-flag");
        seed(&root);
        run(&root);
        let readme = std::fs::read_to_string(root.join("README.md")).unwrap();
        assert!(
            readme.contains("gircap verify --artifact <this directory>"),
            "the re-verify line must be runnable as written:\n{readme}"
        );
        let _ = std::fs::remove_dir_all(&root);
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
        assert!(
            ok,
            "the second pass recorded digests that do not match disk: {detail}"
        );

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

        // And its detail must not name a byte total. The row lives inside
        // `selfcheck.json`, which `total_bytes` sums, so any total written here
        // describes an earlier and smaller tree — the reference bundle said
        // "208307441 B" where `checksums.json` said "208307799 B". The count is
        // safe because no file appears or disappears between the passes.
        assert!(
            !a16[0].actual.contains(" B,"),
            "A16 must not restate a byte total it cannot know: {:?}",
            a16[0].actual
        );
        let count = closed.checksums.file_count;
        assert!(
            a16[0].actual.contains(&format!("{count} files")),
            "A16 must still state the file count it checked: {:?}",
            a16[0].actual
        );

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
        for required in ["verify/selfcheck.json", "MANIFEST.json", "README.md"] {
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
    fn a_failed_row_ends_the_run_with_exit_3() {
        // The behaviour change this commit makes. A bundle whose own rows did
        // not hold used to be written and reported as a success: `result: "fail"`
        // in the document, exit `0` from the process. A caller reading only the
        // exit code was told the capture worked.
        let s = Selfcheck::build(
            vec![Assertion::row(
                "A14",
                "game/",
                "nothing else",
                false,
                "a stray file",
            )],
            Vec::new(),
            &[],
        );
        let f = fatal_failure(&s).expect("a failing row ends the run");
        assert_eq!(f.id, "A14");
        let d = f_detail(f);
        for want in ["A14", "game/", "a stray file"] {
            assert!(d.contains(want), "{want} missing from: {d}");
        }
    }

    #[test]
    fn a_row_whose_id_is_not_fatal_does_not_end_the_run() {
        // **Proves `is_fatal` is consulted** rather than the code testing
        // `!ok` and calling the table decoration. A18 and A20 are verify-only,
        // so a capture can never produce them — which makes them the right probe
        // here: if the table stopped being consulted and the code became
        // `!a.ok`, this test fails.
        let s = Selfcheck::build(
            vec![Assertion::row(
                "A18",
                "verify-only",
                "nothing",
                false,
                "did not hold",
            )],
            Vec::new(),
            &[],
        );
        assert!(
            fatal_failure(&s).is_none(),
            "a non-fatal id must not end a capture"
        );
        assert!(!super::super::selfcheck::is_fatal("A18"));
    }

    #[test]
    fn a_skipped_id_is_not_a_failed_row() {
        // The `--no-chunk-blobs` case, and the reason exit 0 with
        // `result: incomplete` is honest rather than broken: a skipped id has no
        // row, so there is nothing to fail.
        let asserted: Vec<Assertion> = ["A1", "A13"]
            .iter()
            .map(|id| Assertion::row(id, "s", "e", true, "a"))
            .collect();
        let s = Selfcheck::build(
            asserted,
            Vec::new(),
            &super::super::selfcheck::SKIPPED_WITHOUT_CHUNK_BLOBS,
        );
        assert!(fatal_failure(&s).is_none());
        assert_eq!(s.result, super::super::selfcheck::Result_::Incomplete);
    }

    #[test]
    fn a_clean_close_out_returns_ok() {
        let root = scratch("clean");
        seed(&root);
        run(&root);
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
