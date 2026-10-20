//! The A1–A17 assertion log.
//!
//! **M8.** One row per assertion *instance*, not per assertion kind — 71 chunks
//! produce 71 A6 rows — and a failing row is never dropped. A capture that failed
//! an assertion leaves the failure in place to be read, because the useful
//! artifact in that case is the one that says exactly which byte disagreed.
//!
//! ## Which failures end the run
//!
//! The spec says `capture` exits non-zero for A1–A14. A15–A17 are about the
//! *documents this tool just wrote* — the diff counts, the checksum coverage and
//! the matrix's completeness — and a failure in any of them means the bundle is
//! internally inconsistent, which is exactly as untrustworthy as a wrong digest.
//! So all seventeen are fatal here, and [`is_fatal`] records why the spec's split
//! is kept in the table rather than quietly widened.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One assertion, checked once against one subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assertion {
    /// `A1` … `A17`.
    pub id: String,
    /// What was checked, in the spec's words.
    pub assertion: String,
    /// What it was checked against: a blob path, a chunk id, a file name.
    pub subject: String,
    /// What the manifest or the plan said it should be.
    pub expected: String,
    /// What it actually is.
    pub actual: String,
    /// Whether they agree.
    pub ok: bool,
}

impl Assertion {
    /// A green row.
    pub fn pass(
        id: &'static str,
        assertion: &'static str,
        subject: &str,
        value: &str,
    ) -> Assertion {
        Assertion {
            id: id.to_string(),
            assertion: assertion.to_string(),
            subject: subject.to_string(),
            expected: value.to_string(),
            actual: value.to_string(),
            ok: true,
        }
    }

    /// One selfcheck row, pass or fail, with the wording looked up from the id.
    ///
    /// `expected` is stated even on a pass, because `selfcheck.json` records
    /// what was being checked and a reader who has to infer it from the id has
    /// to go and look it up. A17 and A16 were each spelled out at their own call
    /// sites; they differ only in their subject and their expectation, so the
    /// difference was three lines that had to be kept in agreement by hand.
    ///
    /// **Lived in `run.rs`**, where every caller had to remember to pass
    /// `wording(id)` rather than `id` — a mistake that would have compiled and
    /// produced a row whose `assertion` text was the id.
    pub fn row(
        id: &'static str,
        subject: &'static str,
        expected: &'static str,
        ok: bool,
        detail: &str,
    ) -> Assertion {
        if ok {
            Assertion::pass(id, wording(id), subject, detail)
        } else {
            Assertion::fail(id, wording(id), subject, expected, detail)
        }
    }

    /// A red row.
    pub fn fail(
        id: &'static str,
        assertion: &'static str,
        subject: &str,
        expected: &str,
        actual: &str,
    ) -> Assertion {
        Assertion {
            id: id.to_string(),
            assertion: assertion.to_string(),
            subject: subject.to_string(),
            expected: expected.to_string(),
            actual: actual.to_string(),
            ok: false,
        }
    }

    /// From a checked pair, which is the shape almost every assertion has.
    pub fn check(
        id: &'static str,
        assertion: &'static str,
        subject: &str,
        expected: &str,
        actual: &str,
    ) -> Assertion {
        Assertion {
            id: id.to_string(),
            assertion: assertion.to_string(),
            subject: subject.to_string(),
            expected: expected.to_string(),
            actual: actual.to_string(),
            ok: expected == actual,
        }
    }
}

/// The spec's wording for each assertion, so the log reads without the docs.
///
/// A reader looking at a failed bundle should not have to open another file to
/// find out what A11 means, and a one-line table is cheap.
///
/// `A19` is **M11's**, added after running the whole thing against the live API:
/// it is the converse of A14, and without it a capture whose two variant files
/// came out identical would pass every other assertion while holding no evidence
/// of the patch it was taken for.
pub const WORDING: [(&str, &str); 21] = [
    ("A1", "every API response captured has retcode == 0"),
    (
        "A2",
        "getBuild(new).data.tag == --new-tag; getBuild(old).data.tag == --old-tag",
    ),
    ("A3", "manifest blob md5(decompressed) == manifest.checksum"),
    (
        "A4",
        "manifest.checksum == id 32-hex suffix; id 16-hex prefix recorded as opaque",
    ),
    (
        "A5",
        "decoded manifest file_count/chunk_count/uncompressed_size reconcile with getBuild.stats",
    ),
    (
        "A6",
        "len(raw) == compressed_size; md5(raw) == compressed_md5; md5(zstd(raw)) == uncompressed_md5; len(decoded) == uncompressed_size",
    ),
    ("A7", "assemble(materialize_old) has md5 == old entry.md5"),
    ("A8", "assemble(materialize_new) has md5 == new entry.md5"),
    (
        "A9",
        "assemble(REUSE-from-old union patch_fetch) byte-equals assemble(materialize_new)",
    ),
    (
        "A10",
        "every reused chunk's old-file slice MD5 == uncompressed_md5",
    ),
    (
        "A11",
        "chunk offsets gapless from 0, ascending, sum(uncompressed_size) == size",
    ),
    (
        "A12",
        "patch_fetch disjoint from the reuse set, and a subset of materialize_new",
    ),
    (
        "A13",
        "game/config.ini re-parses with game_version == --old-tag",
    ),
    (
        "A14",
        "game/ contains no path outside expected/'s manifest set + the tool-read config files",
    ),
    ("A15", "diff/ counts equal the recomputed set differences"),
    (
        "A16",
        "checksums.json covers every file written under the artifact root, and re-hashing matches",
    ),
    (
        "A17",
        "each required matrix class is captured | covered_by | over_budget | unavailable - never absent",
    ),
    (
        "A19",
        "each two-sided captured file differs between game/ and expected/ exactly when its class says it should",
    ),
    (
        "A21",
        "the bundle covers both trim outcomes: at least one eager and at least one deferred",
    ),
    (
        "A22",
        "the bundle covers both write directions: at least one ascending and at least one descending",
    ),
    (
        "A23",
        "the bundle covers a file that grows: at least one capture with growth_bytes > 0",
    ),
];

/// The wording for an id, or a placeholder for one this build does not know.
pub fn wording(id: &str) -> &'static str {
    WORDING
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, w)| *w)
        .unwrap_or("an assertion this build does not describe")
}

/// Log every selfcheck row that did not pass, with what it expected.
///
/// **The fuller of the two spellings this replaced.** One call site logged
/// `detail` and omitted `expected`; the other logged both. The one that dropped
/// a field was the strict subset, so keeping the fuller form means no log line
/// anywhere in the crate is less informative than it used to be.
///
/// Next to the rows rather than in the dispatcher, so a caller cannot add a
/// red row and forget to say so.
/// A21, A22 and A23: the coverage the class set exists to buy.
///
/// The three properties are otherwise only held **by accident**. A bundle that
/// happened to contain a file with a deferred trim would say nothing about the
/// eager one, and a reader would have to take the class names on trust — which
/// is exactly the trust this tool exists to replace with a measurement.
///
/// The coverage is read out of the patchmaps' `repair` blocks, which say what
/// the in-place path would do with each file: its `trim`, its `direction` and
/// its `growth_bytes`.
///
/// It lives here rather than in `closeout.rs` so that the two places that
/// build these rows build the **same** rows: the capture's and the verifier's
/// have to agree, or `check_agreement` would report a disagreement that does
/// not exist.
pub fn coverage(repairs: &[crate::assemble::Repair]) -> Vec<Assertion> {
    use crate::assemble::{Direction, Trim};

    let mut rows = Vec::new();

    // A21 — both trim outcomes.
    let eager = repairs.iter().filter(|r| r.trim == Trim::Eager).count();
    let deferred = repairs.iter().filter(|r| r.trim == Trim::Deferred).count();
    rows.push(Assertion::row(
        "A21",
        "patchmaps/",
        "at least one eager trim and at least one deferred trim",
        eager > 0 && deferred > 0,
        &format!("{eager} eager trim(s), {deferred} deferred trim(s)"),
    ));

    // A22 — both write directions.
    let ascending = repairs
        .iter()
        .filter(|r| r.direction == Direction::Ascending)
        .count();
    let descending = repairs
        .iter()
        .filter(|r| r.direction == Direction::Descending)
        .count();
    rows.push(Assertion::row(
        "A22",
        "patchmaps/",
        "at least one ascending write order and at least one descending one",
        ascending > 0 && descending > 0,
        &format!("{ascending} ascending, {descending} descending"),
    ));

    // A23 — a file that grows.
    let growing = repairs.iter().filter(|r| r.growth_bytes > 0).count();
    rows.push(Assertion::row(
        "A23",
        "patchmaps/",
        "at least one capture whose file on disk is shorter than the target",
        growing > 0,
        &format!("{growing} capture(s) grow"),
    ));

    rows
}

/// A21, A22 and A23 are about the **bundle's coverage**, so the close-out
/// records them and `verify` re-derives them from the patchmaps.
pub fn report_failed(rows: &[Assertion]) {
    for a in rows.iter().filter(|a| !a.ok) {
        tracing::error!(
            assertion = a.id,
            subject = %a.subject,
            expected = %a.expected,
            actual = %a.actual,
            "selfcheck row failed"
        );
    }
}

/// A1 through A6: what the API and the manifests said, and what arrived.
///
/// One row per *instance* — six bodies, two manifests per field, one row per
/// stored chunk — so a capture with 38 blobs carries 38 A6 rows. That is what
/// makes the row count checkable against the blob count rather than aspirational.
///
/// **Lived in `run.rs`.** It produces rows and it consumed none of the
/// orchestrator's state, so it had no business being there; it was here only
/// because `Assertion::check` and `wording` were not yet on the type it builds.
pub fn api_and_manifest(
    captured: &crate::hyp::Captured,
    versions: &crate::config::VersionPair,
    manifests: &[Vec<crate::manifest::CapturedManifest>],
    index: &crate::blobs::ChunkIndex,
    no_chunk_blobs: bool,
) -> Vec<Assertion> {
    let mut rows = Vec::new();

    // A1: every captured response reported success.
    for body in captured.bodies() {
        let retcode = body.retcode();
        rows.push(Assertion::check(
            "A1",
            wording("A1"),
            &format!("api/{}", body.file),
            "0",
            &retcode.to_string(),
        ));
    }

    // A2: the two builds echo the tags they were asked for.
    for (file, want, got) in [
        (
            format!("api/getBuild.{}.json", versions.new.tag),
            versions.new.tag.as_str(),
            versions.new.tag.as_str(),
        ),
        (
            format!("api/getBuild.{}.json", versions.old.tag),
            versions.old.tag.as_str(),
            versions.old.tag.as_str(),
        ),
    ] {
        rows.push(Assertion::check("A2", wording("A2"), &file, want, got));
    }

    // A3, A4, A5: per manifest.
    for m in manifests.iter().flatten() {
        let subject = format!("manifests/{}/{}", m.tag, m.matching_field);
        rows.push(Assertion::check(
            "A3",
            wording("A3"),
            &subject,
            &m.meta.manifest.checksum,
            &m.meta.stored.md5_decompressed,
        ));
        rows.push(Assertion::check(
            "A4",
            wording("A4"),
            &subject,
            &m.meta.manifest.checksum,
            &m.meta.id_parts.suffix_32hex,
        ));
        // The 16-hex prefix is *recorded* as opaque and never verified, so the
        // row says that rather than implying it was checked and passed.
        rows.push(Assertion::pass(
            "A4",
            wording("A4"),
            &format!("{subject} (id prefix)"),
            &format!(
                "{} recorded as {}",
                m.meta.id_parts.prefix_16hex, m.meta.id_parts.prefix_semantics
            ),
        ));
        for (name, declared, actual) in
            crate::manifest::reconcile(&m.meta.decoded, m.meta.declared.stats.as_ref())
        {
            rows.push(Assertion::check(
                "A5",
                wording("A5"),
                &format!("{subject} ({name})"),
                &format!("{declared:?}"),
                &actual.to_string(),
            ));
        }
    }

    // A6: one row per stored chunk, four checks folded into one verdict with the
    // failing ones named - a blob failing three of four is one finding about
    // one blob, not three findings about three blobs.
    //
    // `--no-chunk-blobs` emits **no** A6 rows at all and `A6` joins the skipped
    // list; otherwise a metadata-only run could never be green. The distinction
    // `BlobProps` already draws - `verified: null` means "not measured", a
    // different kind of fact from four `false` - has to reach the selfcheck too,
    // or a declared gap reads as a broken artifact.
    if !no_chunk_blobs {
        for b in &index.blobs {
            let subject =
                b.props.stored_path.clone().unwrap_or_else(|| {
                    format!("chunks/blobs/{}.zst (not stored)", b.props.chunk_id)
                });
            let v = b.props.verified;
            rows.push(match v {
                Some(v) if v.all_green() && b.props.decode_error.is_none() => {
                    Assertion::pass("A6", wording("A6"), &subject, &b.props.md5_decompressed)
                }
                Some(v) => Assertion::fail(
                    "A6",
                    wording("A6"),
                    &subject,
                    "all four checks hold",
                    &format!(
                        "{}; md5(decompressed) {}",
                        v.failures().join(", "),
                        b.props.md5_decompressed
                    ),
                ),
                // Reachable only when a blob is *wanted* but neither fetched nor
                // measured - a hole the store reported rather than one the run
                // chose. `verified: null` on a stored blob is a different thing
                // and does not get here, because the flag suppresses the rows.
                None => Assertion::fail(
                    "A6",
                    wording("A6"),
                    &subject,
                    "measured",
                    b.props
                        .decode_error
                        .as_deref()
                        .unwrap_or("wanted but never measured"),
                ),
            });
        }
    }
    rows
}

/// A13, A14 and A15: the assertions that are about the tree, not a blob.
///
/// **Lived in `run.rs`** as a 174-line function the orchestrator called once.
/// It reads no orchestrator state, produces rows, and ended by returning one
/// `Vec<Assertion>` for the caller to append -- so it belonged next to the
/// `Assertion` it builds, with `check_config_ini` beside it.
///
#[allow(clippy::too_many_arguments)]
pub fn closing(
    dir: &std::path::Path,
    old_tag: &str,
    scan_dir: &str,
    selection: &crate::select::Selection,
    diffs: &[crate::manifest::diff::FieldDiff],
    side_digests: &BTreeMap<String, (String, String)>,
    written: &[String],
    no_chunk_blobs: bool,
) -> Vec<Assertion> {
    let mut rows = Vec::new();

    // A13: `game/config.ini` re-parses with `game_version == --old-tag`.
    //
    // Re-read from disk rather than from the string that produced it: the
    // question is whether a *reader* can get the version out, and comparing
    // against the same input would pass even if the format were unreadable.
    rows.push(check_config_ini(dir, old_tag));

    // A14: `game/` holds nothing but the tool-read config and the variant
    // files. Anything else is a file the tool under test would build a plan
    // against, which is the failure this bundle exists to rule out.
    let captured: std::collections::BTreeSet<&str> =
        selection.admitted.iter().map(|a| a.path.as_str()).collect();
    let game_files: std::collections::BTreeSet<&str> = written
        .iter()
        .filter_map(|p| p.strip_prefix("game/"))
        .collect();
    let strays: Vec<&str> = game_files
        .iter()
        .copied()
        .filter(|p| *p != "config.ini" && *p != scan_dir && !captured.contains(p))
        .collect();
    rows.push(if strays.is_empty() {
        Assertion::pass(
            "A14",
            wording("A14"),
            "game/",
            &format!(
                "{} files, all of them config.ini, the scan file, or a captured path",
                game_files.len()
            ),
        )
    } else {
        Assertion::fail(
            "A14",
            wording("A14"),
            "game/",
            "config.ini, the audio scan file, and captured paths only",
            &format!("{} stray path(s): {}", strays.len(), strays.join(", ")),
        )
    });

    // A19: the *converse* of A14 - every captured file differs between the two
    // trees exactly when its class says it should.
    //
    // A14 asks "is `game/` free of paths the tool under test would plan
    // against", which is one direction. It cannot see the other failure: a
    // capture admitted for `kept_in_place` whose two variant files came
    // out byte-identical. That bundle would satisfy A14, satisfy A7/A8 (each
    // side matches its own declared MD5), and be useless - the fixture would
    // hold no evidence of the patch it was captured for. M11 found the gap by
    // comparing the two trees by hand, which is the argument for checking it.
    //
    // A capture with only one side - `added` or `removed` - is skipped: its file
    // is absent from one tree *by construction*, which is the whole point of the
    // class rather than a disagreement.
    // Skipped under `--no-chunk-blobs`, for the reason A6 is: there are no
    // assembled sides to compare, so the check would pass vacuously and a green
    // row would be a claim about nothing. Exactly the failure mode A6 had.
    let mut agree: Vec<&str> = Vec::new();
    let mut disagree: Vec<String> = Vec::new();
    if !no_chunk_blobs {
        for a in &selection.admitted {
            let Some((old, new)) = side_digests.get(&a.capture_id) else {
                continue;
            };
            if old.is_empty() || new.is_empty() {
                // One-sided: absent from the other tree by design.
                continue;
            }
            let differs = old != new;
            let should_differ = !a.primary_class.is_identical();
            if differs == should_differ {
                agree.push(a.path.as_str());
            } else {
                disagree.push(format!(
                    "{} ({}) {} but its class says it should {}",
                    a.path,
                    a.capture_id,
                    if differs { "differs" } else { "is unchanged" },
                    if should_differ {
                        "differ"
                    } else {
                        "be unchanged"
                    }
                ));
            }
        }
        rows.push(if disagree.is_empty() {
            Assertion::pass(
                "A19",
                wording("A19"),
                "game/ vs expected/",
                &format!(
                    "{} two-sided capture(s): each file differs exactly when its class says \
                     it should",
                    agree.len()
                ),
            )
        } else {
            Assertion::fail(
                "A19",
                wording("A19"),
                "game/ vs expected/",
                "each two-sided file differs exactly when its class says it should",
                &format!("{} disagree: {}", disagree.len(), disagree.join("; ")),
            )
        });
    }
    // data it counted is caught rather than copied.
    for f in diffs {
        let recomputed_common = f
            .rows
            .iter()
            .filter(|r| {
                matches!(
                    r.state,
                    crate::manifest::diff::State::Identical | crate::manifest::diff::State::Changed
                )
            })
            .count() as u64;
        let recomputed_identical = f
            .rows
            .iter()
            .filter(|r| r.state == crate::manifest::diff::State::Identical)
            .count() as u64;
        let recomputed_added = f
            .rows
            .iter()
            .filter(|r| r.state == crate::manifest::diff::State::Added)
            .count() as u64;
        let recomputed_removed = f
            .rows
            .iter()
            .filter(|r| r.state == crate::manifest::diff::State::Removed)
            .count() as u64;
        let subject = format!("diff/{} counts", f.field);
        for (name, recorded, actual) in [
            ("common", f.counts.common, recomputed_common),
            ("identical", f.counts.identical, recomputed_identical),
            ("added", f.counts.added, recomputed_added),
            ("removed", f.counts.removed, recomputed_removed),
        ] {
            rows.push(Assertion::check(
                "A15",
                wording("A15"),
                &format!("{subject} ({name})"),
                &recorded.to_string(),
                &actual.to_string(),
            ));
        }
    }

    // `--no-chunk-blobs` contributes **no** A7-A12 rows. A declared gap is not a
    // failure: it is declared the way the design declares everything else — by
    // listing the ids in `skipped`, which is what makes `Selfcheck::build` answer
    // `incomplete` rather than `fail`. A row that says "not asserted" and is marked
    // `ok: false` tells a reader the bundle is broken when it is honestly
    // incomplete.
    if no_chunk_blobs {
        tracing::debug!("--no-chunk-blobs: A7-A12 are declared skipped, not asserted");
    }
    rows
}

/// A13: read `game/config.ini` back and check the version a reader would find.
///
/// **One implementation, two callers.** `verify` ran its own copy — same file,
/// same parse, same comparison, and *different wording* for every branch, which
/// is the worst kind of duplication: the two can disagree about what a passing
/// A13 looks like on paper while agreeing about whether it passes.
///
/// The capture's wording is the one kept, because `verify` compares its rows
/// against the capture's recorded ones, and the recorded row is what a reader
/// has already seen. `verify` no longer says `readable` where the capture says
/// `game_version == <tag>`, nor `game_version == v` where the capture writes
/// `game_version is v`.
pub fn check_config_ini(dir: &std::path::Path, old_tag: &str) -> Assertion {
    const SUBJECT: &str = "game/config.ini";
    let cfg = dir.join("game").join("config.ini");
    let expected = format!("game_version == {old_tag}");
    match std::fs::read_to_string(&cfg) {
        Err(e) => Assertion::fail(
            "A13",
            wording("A13"),
            SUBJECT,
            &expected,
            &format!("unreadable: {e}"),
        ),
        Ok(text) => match crate::emit::gamedir::parse_game_version(&text) {
            Err(e) => Assertion::fail("A13", wording("A13"), SUBJECT, &expected, &e),
            Ok(got) if got == old_tag => Assertion::pass(
                "A13",
                wording("A13"),
                SUBJECT,
                &format!("game_version is {got}"),
            ),
            Ok(got) => Assertion::fail(
                "A13",
                wording("A13"),
                SUBJECT,
                &expected,
                &format!("game_version is {got}, the old tag is {old_tag}"),
            ),
        },
    }
}

/// The ids a `--no-chunk-blobs` run declares **skipped** rather than asserted.
///
/// **Derived, and stated here so the derivation is checkable.** A run that
/// stores no chunk blobs cannot establish A6 (every stored chunk's digests
/// reconcile) or A7-A12 (every capture assembles from stored chunks) or A19 (the
/// two sides differ), so it records those as not-proven and exits `0` with
/// `result: incomplete`.
///
/// It is the complement of what a `--no-chunk-blobs` run *does* assert — A1-A5,
/// A13-A17 — within [`WORDING`], and that is exactly what [`Selfcheck::skipped`]
/// computes from the rows. This constant used to be a bare `vec![]` at the
/// call site with the derived method beside it and no connection between them:
/// two sources of truth for one fact, where a change to `WORDING` would silently
/// change what the document said without changing what the run recorded.
///
/// `skipped_were_declared_not_asserted` is the check. It builds a selfcheck the
/// way a `--no-chunk-blobs` run does and requires the two to be identical.
pub const SKIPPED_WITHOUT_CHUNK_BLOBS: [&str; 11] = [
    "A6", "A7", "A8", "A9", "A10", "A11", "A12", "A19", "A21", "A22", "A23",
];

/// Whether a failure of this assertion ends the run.
///
/// All seventeen, and the split in the spec is kept as a table rather than a
/// branch so that widening it later is a one-line change with the reasoning
/// still attached.
pub fn is_fatal(id: &str) -> bool {
    matches!(
        id,
        "A1" | "A2"
            | "A3"
            | "A4"
            | "A5"
            | "A6"
            | "A7"
            | "A8"
            | "A9"
            | "A10"
            | "A11"
            | "A12"
            | "A13"
            | "A14"
            | "A15"
            | "A16"
            | "A17"
            | "A19"
            | "A21"
            | "A22"
            | "A23"
    )
}

/// How a run ended, as the selfcheck says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Result_ {
    /// Every assertion held.
    Pass,
    /// At least one did not.
    Fail,
    /// Assertions that could not be run at all — `--no-chunk-blobs` skips A7–A12,
    /// which is a known and declared gap rather than a failure.
    Incomplete,
}

/// `verify/selfcheck.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Selfcheck {
    /// The artifact schema version.
    pub schema_version: u32,
    /// `pass` | `fail` | `incomplete`.
    pub result: Result_,
    /// How many rows, and how they came out.
    pub counts: Counts,
    /// One row per assertion instance.
    pub assertions: Vec<Assertion>,
    /// The matrix, so the two documents cannot disagree.
    pub matrix: Vec<MatrixRow>,
}

/// The three numbers, kept consistent with the rows by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    /// How many rows.
    pub total: u64,
    /// How many held.
    pub passed: u64,
    /// How many did not.
    pub failed: u64,
}

/// A matrix row as the selfcheck repeats it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixRow {
    /// The class.
    pub class: String,
    /// What happened to it.
    pub status: String,
    /// Whether that is one of the three acceptable answers.
    pub ok: bool,
}

impl MatrixRow {
    /// Whether a `status` string is one A17 accepts.
    ///
    /// Takes the string rather than a `&MatrixRow` because M9's `verify` has a
    /// status it read out of `MANIFEST.json` and no row to hang the answer on.
    /// Every state is acceptable: A17 is about the matrix being **complete**,
    /// and `over_budget` is a complete answer — "a candidate existed and this
    /// run's budget would not pay for it". The *requirement* is
    /// `--require-class`'s business, and it excludes `over_budget` on its own.
    pub fn ok_status(status: &str) -> bool {
        matches!(
            status,
            "captured" | "covered_by" | "over_budget" | "unavailable"
        )
    }
}

impl Selfcheck {
    /// Build the document, deriving `counts` and `result` from the rows.
    ///
    /// Derived rather than passed in, because a selfcheck whose counts disagree
    /// with its own rows is the one document nobody can use.
    pub fn build(
        assertions: Vec<Assertion>,
        matrix: Vec<MatrixRow>,
        skipped: &[&str],
    ) -> Selfcheck {
        let failed = assertions.iter().filter(|a| !a.ok).count() as u64;
        let total = assertions.len() as u64;
        let result = if failed > 0 {
            Result_::Fail
        } else if !skipped.is_empty() {
            Result_::Incomplete
        } else {
            Result_::Pass
        };
        Selfcheck {
            schema_version: crate::SCHEMA_VERSION,
            result,
            counts: Counts {
                total,
                passed: total - failed,
                failed,
            },
            assertions,
            matrix,
        }
    }

    /// Whether every assertion held and none was skipped.
    pub fn is_pass(&self) -> bool {
        self.result == Result_::Pass
    }

    /// The failing rows, for the run log and the refusal message.
    pub fn failures(&self) -> Vec<&Assertion> {
        self.assertions.iter().filter(|a| !a.ok).collect()
    }

    /// How many rows each assertion id contributed.
    ///
    /// This is the number that makes "one row per *instance*" checkable: a
    /// capture with 38 blobs must report 38 A6 rows, not one.
    pub fn by_id(&self) -> BTreeMap<&str, u64> {
        let mut m: BTreeMap<&str, u64> = BTreeMap::new();
        for a in &self.assertions {
            *m.entry(a.id.as_str()).or_default() += 1;
        }
        m
    }

    /// The ids that could not be run at all, and why they were skipped.
    pub fn skipped(&self) -> Vec<&'static str> {
        let present: std::collections::BTreeSet<&str> =
            self.assertions.iter().map(|a| a.id.as_str()).collect();
        WORDING
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| !present.contains(id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_were_declared_not_asserted() {
        // `SKIPPED_WITHOUT_CHUNK_BLOBS` is what a `--no-chunk-blobs` run
        // *records*; `Selfcheck::skipped()` is what the document *derives* from the
        // rows. Two sources of truth for one fact would mean a change to `WORDING`
        // moving what the document says without moving what the run records.
        //
        // Both directions, because either alone passes on a one-sided change: a
        // constant that gained an id the rows still assert, and a `WORDING` that
        // gained an id the constant never learned about.
        let asserted: Vec<Assertion> = [
            "A1", "A2", "A3", "A4", "A5", "A13", "A14", "A15", "A16", "A17",
        ]
        .iter()
        .map(|id| Assertion::row(id, "s", "e", true, "a"))
        .collect();
        let s = Selfcheck::build(asserted, Vec::new(), &SKIPPED_WITHOUT_CHUNK_BLOBS);

        assert_eq!(
            s.skipped(),
            SKIPPED_WITHOUT_CHUNK_BLOBS.to_vec(),
            "the declared skips and the derived skips must be the same set"
        );
        assert_eq!(
            s.result,
            Result_::Incomplete,
            "and a declared gap is incomplete, never a pass"
        );
        assert_eq!(s.counts.failed, 0, "a skip is not a failure");
    }

    #[test]
    fn a_full_run_declares_nothing_skipped() {
        // The other direction: with every id asserted, the derived set is empty,
        // so the flag's list must not be used when the flag is off. A constant
        // applied unconditionally would make every complete run `incomplete`.
        let asserted: Vec<Assertion> = WORDING
            .iter()
            .map(|(id, _)| Assertion::row(id, "s", "e", true, "a"))
            .collect();
        let s = Selfcheck::build(asserted, Vec::new(), &[]);
        assert!(s.skipped().is_empty());
        assert_eq!(s.result, Result_::Pass);
    }

    fn row(id: &'static str, ok: bool) -> Assertion {
        if ok {
            Assertion::pass(id, wording(id), "s", "v")
        } else {
            Assertion::fail(id, wording(id), "s", "v", "other")
        }
    }

    #[test]
    fn counts_and_result_are_derived_from_the_rows_and_cannot_disagree() {
        let s = Selfcheck::build(
            vec![row("A6", true), row("A6", false), row("A7", true)],
            Vec::new(),
            &[],
        );
        assert_eq!(
            s.counts,
            Counts {
                total: 3,
                passed: 2,
                failed: 1
            }
        );
        assert_eq!(s.result, Result_::Fail);
        assert!(!s.is_pass());
        assert_eq!(s.failures().len(), 1);
    }

    #[test]
    fn an_all_green_run_passes_and_a_skipped_assertion_is_incomplete_not_failed() {
        let s = Selfcheck::build(vec![row("A6", true), row("A7", true)], Vec::new(), &[]);
        assert_eq!(s.result, Result_::Pass);
        assert!(s.is_pass());

        // `--no-chunk-blobs` skips A7-A12: declared, not failed.
        let s = Selfcheck::build(vec![row("A1", true)], Vec::new(), &["A7", "A8"]);
        assert_eq!(s.result, Result_::Incomplete);
        assert_eq!(s.counts.failed, 0, "a skip is not a failure");
    }

    /// One row per *instance*, which is the property that makes "38 blobs, 38 A6
    /// rows" checkable rather than aspirational.
    #[test]
    fn a_row_per_instance_is_what_by_id_reports() {
        let rows: Vec<Assertion> = (0..38).map(|_| row("A6", true)).collect();
        let s = Selfcheck::build(rows, Vec::new(), &[]);
        assert_eq!(s.by_id().get("A6"), Some(&38));
        assert_eq!(s.counts.total, 38);
    }

    /// A `skipped` list derived from the rows rather than from what the caller
    /// passed in, so the document cannot claim coverage it does not have.
    #[test]
    fn skipped_is_derived_from_the_rows_that_are_present() {
        let s = Selfcheck::build(vec![row("A1", true), row("A13", true)], Vec::new(), &[]);
        let sk = s.skipped();
        // Everything in `WORDING` that produced no row. Derived, so the count is
        // `WORDING.len() - 2` rather than a second literal to keep in step.
        assert_eq!(sk.len(), WORDING.len() - 2);
        assert!(!sk.contains(&"A1") && !sk.contains(&"A13"));
        assert!(sk.contains(&"A9"));
        // The ids are also unique, so a count is not hiding a duplicate.
        let mut dedup = sk.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(dedup.len(), sk.len(), "a duplicated id in the skipped list");
    }

    /// The twenty-one ids (seventeen from the spec plus M11's A19 and the three
    /// coverage assertions) each have wording and each is fatal.
    ///
    /// Twenty-one rather than twenty is the point of asserting the length: a new
    /// assertion added without wording would otherwise pass, and a wording entry
    /// added without a producer would be a promise nothing keeps.
    #[test]
    fn every_assertion_id_has_wording_and_is_fatal() {
        for (id, w) in WORDING {
            assert!(!w.is_empty(), "{id} has no wording");
            assert_eq!(wording(id), w);
            assert!(is_fatal(id), "{id}");
        }
        assert_eq!(WORDING.len(), 21);
        assert_eq!(wording("A99"), "an assertion this build does not describe");
    }

    /// The three coverage assertions are about the bundle, not about one file,
    /// and they are the only ones that say so.
    #[test]
    fn the_three_coverage_assertions_are_present_and_named() {
        for id in ["A21", "A22", "A23"] {
            assert!(WORDING.iter().any(|(w, _)| *w == id), "{id}");
            assert!(is_fatal(id), "{id} is fatal");
        }
        // And they are **not** the spec's own ids: A18 and A20 are the two
        // `verify`-only ones.
        assert!(!WORDING.iter().any(|(id, _)| *id == "A18"));
        assert!(!WORDING.iter().any(|(id, _)| *id == "A20"));
    }

    /// **The negative case**, and the reason the three assertions exist: a
    /// bundle that does not cover both trim outcomes **fails A21**.
    ///
    /// The properties the class set buys are otherwise only held *by accident*:
    /// a bundle that happened to contain a file with a deferred trim would say
    /// nothing about the eager one, and a reader would have to take the class
    /// names on trust.
    #[test]
    fn a_bundle_without_both_trims_fails_a21() {
        use crate::assemble::{Direction, Mode, Repair, Trim};

        let repair = |trim| Repair {
            kept: 1,
            moved: 0,
            fetch: 0,
            kept_bytes: 10,
            payload_bytes: 0,
            write_bytes: 0,
            direction: Direction::NotApplicable,
            keeps_ascending: 0,
            keeps_descending: 0,
            trim,
            growth_bytes: 0,
            mode: Mode::Inplace,
        };

        // Only the eager trim: A21 is red.
        let rows = coverage(&[repair(Trim::Eager)]);
        let a21 = rows.iter().find(|r| r.id == "A21").expect("a row");
        assert!(!a21.ok, "one trim outcome is not both: {a21:?}");
        assert!(
            a21.actual.contains("0 deferred"),
            "the detail says so: {a21:?}"
        );

        // Only the deferred one: still red.
        let rows = coverage(&[repair(Trim::Deferred)]);
        let a21 = rows.iter().find(|r| r.id == "A21").expect("a row");
        assert!(!a21.ok);

        // Both: green.
        let rows = coverage(&[repair(Trim::Eager), repair(Trim::Deferred)]);
        let a21 = rows.iter().find(|r| r.id == "A21").expect("a row");
        assert!(a21.ok, "both trims: {a21:?}");
    }

    /// The same shape for the other two: both write directions, and a file that
    /// grows.
    #[test]
    fn a_bundle_without_both_directions_fails_a22_and_one_that_never_grows_fails_a23() {
        use crate::assemble::{Direction, Mode, Repair, Trim};

        let repair = |direction, growth_bytes| Repair {
            kept: 1,
            moved: 1,
            fetch: 0,
            kept_bytes: 10,
            payload_bytes: 10,
            write_bytes: 10,
            direction,
            keeps_ascending: 1,
            keeps_descending: 0,
            trim: Trim::None,
            growth_bytes,
            mode: Mode::Inplace,
        };

        let rows = coverage(&[repair(Direction::Ascending, 0)]);
        assert!(
            !rows.iter().find(|r| r.id == "A22").expect("a row").ok,
            "one direction is not both"
        );
        assert!(
            !rows.iter().find(|r| r.id == "A23").expect("a row").ok,
            "and a file that does not grow is not a growth"
        );

        let rows = coverage(&[
            repair(Direction::Ascending, 0),
            repair(Direction::Descending, 0),
            repair(Direction::NotApplicable, 64),
        ]);
        assert!(rows.iter().find(|r| r.id == "A22").expect("a row").ok);
        assert!(rows.iter().find(|r| r.id == "A23").expect("a row").ok);
    }

    /// A19 and A18 are the two that are not the spec's, and both are named.
    #[test]
    fn the_two_added_assertions_are_present_and_distinct() {
        assert!(WORDING.iter().any(|(id, _)| *id == "A19"));
        // A18 is verify-only and deliberately has no capture-side wording: a
        // capture cannot assert it, because it is a statement about the bundle.
        assert!(
            !WORDING.iter().any(|(id, _)| *id == "A18"),
            "A18 belongs to `verify`, not to a capture's selfcheck"
        );
    }

    /// A red row must say both numbers, or a reader cannot tell which side
    /// disagreed — and that is the whole question a failed capture poses.
    #[test]
    fn a_failed_row_carries_both_sides_of_the_disagreement() {
        let a = Assertion::check("A7", wording("A7"), "game/x.dll", "d992a789", "1e210ff1");
        assert!(!a.ok);
        assert_eq!(a.expected, "d992a789");
        assert_eq!(a.actual, "1e210ff1");
        assert_eq!(a.subject, "game/x.dll");
        let j = serde_json::to_value(&a).unwrap();
        for k in ["id", "assertion", "subject", "expected", "actual", "ok"] {
            assert!(j.get(k).is_some(), "{k} is missing");
        }
    }

    #[test]
    fn the_result_serialises_lowercase_the_schema_doc_writes() {
        assert_eq!(serde_json::to_string(&Result_::Pass).unwrap(), "\"pass\"");
        assert_eq!(serde_json::to_string(&Result_::Fail).unwrap(), "\"fail\"");
        assert_eq!(
            serde_json::to_string(&Result_::Incomplete).unwrap(),
            "\"incomplete\""
        );
    }
}
