//! The fixture: `game/`, `expected/` and `scan/`, built from the stored chunks.
//!
//! **Lived inline in `run::capture`,** where it was 153 lines and the largest
//! single phase. Three things were tangled in it: assembling each admitted
//! capture, writing the two sides out, and deciding whether the resulting rows
//! are allowed to be a failure. Only the last is a decision about the *run*.
//!
//! # Why one function and not three
//!
//! `assemble` already exists and already returns everything a capture needs;
//! this is the part around it. The loop body is six steps that must happen per
//! capture in one order — trace, accumulate, extend rows, record the digests,
//! write the patchmap, write the two sides — and a caller that got the order
//! wrong would produce a bundle that passed A14 and had no patchmap. So the
//! per-capture body is a private function here rather than being left to the
//! orchestrator.
//!
//! # Why it aborts
//!
//! A capture that cannot rebuild its own files has nothing worth keeping, and
//! the operator should find out at capture time rather than at patch time. A
//! failure here is `assertion` (exit 3), not `write_err`: the network was fine,
//! the chunks arrived, and the bytes did not reassemble into what the manifests
//! said they would.

use std::collections::BTreeMap;
use std::path::Path;

use crate::assemble::{self, ChunkSource};
use crate::blobs::chunks::FilePair;
use crate::failure::{Failure, apierr, assertion, write_err};
use crate::http::ApiError;
use crate::hyp;

use super::gamedir;
use super::selfcheck;

/// Everything the fixture build needs from the orchestrator.
pub struct FixtureInput<'a> {
    /// The artifact root. Everything is written under it.
    pub dir: &'a Path,
    /// The captured API bodies, for A1-A5 and for `getGameConfigs`.
    pub captured: &'a hyp::Captured,
    /// The resolved version pair, for A1-A5.
    pub versions: &'a crate::config::VersionPair,
    /// Every field's manifests, for A1-A5.
    pub manifests: &'a [Vec<crate::manifest::CapturedManifest>],
    /// The chunk index, for A1-A5 and as the chunk source's backing store.
    pub index: &'a crate::blobs::ChunkIndex,
    /// The admitted captures, in admission order.
    pub admitted: &'a [crate::select::Admission],
    /// One pair per admitted capture, in the same order.
    pub pairs: &'a [FilePair],
    /// The chunk store, for the `url` of a FETCH entry.
    pub store: &'a crate::blobs::Store,
    /// The region, for `config.ini`.
    pub biz: &'a crate::biz::Biz,
    /// The tag `config.ini` must declare.
    pub old_tag: &'a str,
    /// The `--audio` codes, written into the scan file.
    pub audio: &'a [String],
    /// `--no-chunk-blobs`, which suppresses the chunk blobs but **not** the
    /// prelude.
    pub no_chunk_blobs: bool,
}

/// What the fixture build produced.
pub struct BuiltFixture {
    /// Where the game keeps its audio scan file, read from `getGameConfigs`.
    ///
    /// Returned rather than recomputed by the caller because A14 re-reads the
    /// file from it, and a caller that derived the path a second way would be
    /// testing its own derivation rather than the bundle.
    pub scan_dir: String,
    /// A1-A12.
    pub rows: Vec<selfcheck::Assertion>,
    /// The captures that assembled, in order.
    pub assembled_ids: Vec<String>,
    /// `(old md5, new md5)` per capture, for A19.
    ///
    /// The digests, not the bytes: A19 only asks whether the two sides differ,
    /// and holding the assembled files here would keep a whole patch resident
    /// twice.
    pub side_digests: BTreeMap<String, (String, String)>,
    /// The paths written here, to append to the run's `written` list.
    pub written: Vec<String>,
    /// Chunks reused from the old side, summed.
    pub reused: u64,
    /// Chunks fetched to build a patchmap, summed.
    pub fetched: u64,
    /// Chunks moved by those patchmaps, summed.
    pub moved: u64,
    /// How many A1-A12 rows failed.
    ///
    /// **Always 0.** `build` returns `Err` when any row fails, so there is no
    /// path on which a non-zero value reaches a caller. It is carried rather than
    /// recomputed at the summary because the summary has always reported
    /// `assembly_selfcheck_failures`, and a key that quietly changed meaning
    /// because its computation moved would be worse than a key that is
    /// structurally always zero.
    pub failures: u64,
}

/// Build `game/`, `expected/` and `scan/`, and settle A1-A12.
///
/// `game/` and `expected/` are written one capture at a time, as each
/// assembles, so the fixture is never fully resident: the whole thing is the
/// largest object this tool ever holds.
///
/// **Aborts on any A7-A12 failure.** The rows are reported first — `report_failed`
/// prints each one, so the operator sees which comparison failed rather than
/// only that something did — and the run then fails as `assertion`.
pub fn build(i: FixtureInput<'_>) -> Result<BuiltFixture, Failure> {
    let chunk_source = ChunkSource::from_index(i.dir, i.index);
    // Where the game puts the audio scan file. Never hardcoded: it is the
    // launcher's path, and the fixture has to write it where the tool under
    // test looks for it.
    let scan_dir = i
        .captured
        .configs
        .data::<hyp::GameConfigs>()
        .map_err(|e| apierr("parse getGameConfigs", &e))?
        .audio_pkg_scan_dir()
        .map_err(|e| apierr("read getGameConfigs.audio_pkg_scan_dir", &e))?
        .to_string();

    let mut out = BuiltFixture {
        scan_dir,
        rows: selfcheck::api_and_manifest(
            i.captured,
            i.versions,
            i.manifests,
            i.index,
            i.no_chunk_blobs,
        ),
        assembled_ids: Vec::new(),
        side_digests: BTreeMap::new(),
        written: Vec::new(),
        reused: 0,
        fetched: 0,
        moved: 0,
        failures: 0,
    };

    if !i.no_chunk_blobs {
        for (a, pair) in i.admitted.iter().zip(i.pairs) {
            let mut classes = vec![a.primary_class];
            classes.extend(a.also_satisfies.iter().copied());
            classes.sort();
            classes.dedup();
            let got = assemble::assemble(&assemble::AssemblyInput {
                pair,
                matching_field: a.matching_field.as_str(),
                classes: &classes,
                sources: i.store.sources(),
                chunks: &chunk_source,
            });
            // The six steps below must happen in this order for each capture.
            // A caller that could reorder them would produce a bundle that
            // satisfied A14 and had no patchmap, which is the exact shape of
            // "looks fine, is useless".
            record(i.dir, a, got.map_err(|e| {
                assertion(
                    &format!("assemble {}", a.path),
                    &ApiError::shape("assemble", e),
                )
            })?, &mut out)?;
        }
    }

    // `write_prelude` always runs, including under `--no-chunk-blobs`.
    //
    // `config.ini` and the audio scan file are derived from `getGameConfigs` and
    // the `--audio` flag, not from a single chunk. Putting them behind the flag
    // made a metadata-only bundle have no `game/` at all, so A13 - "the config
    // re-parses with `game_version == --old-tag`" - failed on a tree that simply
    // had no config in it, and reported a missing file as a broken assertion.
    // The flag suppresses *chunk blobs*, not the launcher metadata.
    let mut prelude = gamedir::write_prelude(
        i.dir,
        i.biz,
        i.old_tag,
        &out.scan_dir,
        i.audio,
    )
    .map_err(|e| write_err(i.dir, e))?;
    out.written.append(&mut prelude);

    let failures = out.rows.iter().filter(|a| !a.ok).count();
    selfcheck::report_failed(&out.rows);
    tracing::info!(
        rows = out.rows.len(),
        failures,
        patchmaps = out.assembled_ids.len(),
        "assembly assertions complete"
    );
    if failures > 0 {
        return Err(assertion(
            "assemble the admitted captures",
            &ApiError::shape(
                "A7-A12",
                format!(
                    "{failures} of {} selfcheck rows failed",
                    out.rows.len()
                ),
            ),
        ));
    }
    Ok(out)
}

/// One capture's six steps: trace, accumulate, extend rows, record digests,
/// write the patchmap, write both sides.
///
/// Split out of [`build`] so the ordering is enforced by the type system — a
/// caller cannot assemble and then forget half of what the result implies.
fn record(
    dir: &Path,
    a: &crate::select::Admission,
    got: assemble::Assembled,
    out: &mut BuiltFixture,
) -> Result<(), Failure> {
    let p = got.patch.as_ref();
    tracing::info!(
        capture_id = %a.capture_id,
        path = %a.path,
        old_md5_ok = got.old.as_ref().is_none_or(|o| o.md5_matches()),
        new_md5_ok = got.new.as_ref().is_none_or(|n| n.md5_matches()),
        a9 = p.is_none_or(|p| p.equals_new),
        reuse = p.map(|p| p.reused).unwrap_or(0),
        fetch = p.map(|p| p.fetched).unwrap_or(0),
        moved = p.map(|p| p.moved).unwrap_or(0),
        download_bytes = got.map.totals.map(|t| t.download_bytes).unwrap_or(0),
        "assembled"
    );
    if let Some(p) = &got.patch {
        out.reused += p.reused as u64;
        out.fetched += p.fetched as u64;
        out.moved += p.moved as u64;
    }
    // The assembler already produces selfcheck rows - both sides of each
    // comparison, with the spec's wording - so they are carried through rather
    // than re-derived here.
    out.rows.extend(got.selfcheck_rows());
    out.assembled_ids.push(a.capture_id.clone());
    out.side_digests.insert(
        a.capture_id.clone(),
        (
            got.old.as_ref().map(|s| s.md5.clone()).unwrap_or_default(),
            got.new.as_ref().map(|s| s.md5.clone()).unwrap_or_default(),
        ),
    );
    out.written.push(
        super::write_patchmap(dir, &a.capture_id, &got.map).map_err(|e| write_err(dir, e))?,
    );
    // `game/` is the fixture, `expected/` the ground truth. A `removed` capture
    // contributes only to the first.
    if let Some(o) = &got.old {
        out.written.push(
            gamedir::write_variant(
                dir,
                "game",
                &gamedir::VariantFile {
                    rel_path: &a.path,
                    bytes: &o.bytes,
                    capture_id: &a.capture_id,
                },
            )
            .map_err(|e| write_err(dir, e))?,
        );
    }
    if let Some(n) = &got.new {
        out.written.push(
            gamedir::write_variant(
                dir,
                "expected",
                &gamedir::VariantFile {
                    rel_path: &a.path,
                    bytes: &n.bytes,
                    capture_id: &a.capture_id,
                },
            )
            .map_err(|e| write_err(dir, e))?,
        );
    }
    Ok(())
}
