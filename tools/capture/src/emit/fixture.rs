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

use rayon::prelude::*;

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
    /// The run's `--max-decompressed-bytes`, for every chunk decode.
    pub max_decompressed_bytes: u64,
    /// Where a device-name path is recorded under its escaped on-disk name.
    pub path_map: &'a crate::pathmap::PathMap,
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
    /// The patch shape of each capture, for A21-A23.
    ///
    /// The patchmaps' own arithmetic, in the order the captures assembled.
    /// They are the bundle's account of its coverage, and the three assertions
    /// read them.
    pub shapes: Vec<crate::assemble::PatchShape>,
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

/// Assemble one admitted capture. Shared by the parallel collector and its
/// serial reference test so the two cannot drift apart.
fn assemble_one(
    a: &crate::select::Admission,
    pair: &crate::blobs::chunks::FilePair,
    sources: &crate::blobs::Sources,
    chunks: &ChunkSource<'_>,
) -> Result<assemble::Assembled, String> {
    let mut classes = vec![a.primary_class];
    classes.extend(a.also_satisfies.iter().copied());
    classes.sort();
    classes.dedup();
    assemble::assemble(&assemble::AssemblyInput {
        pair,
        matching_field: a.matching_field.as_str(),
        classes: &classes,
        sources,
        chunks,
    })
}

/// Build `game/`, `expected/` and `scan/`, and settle A1-A12.
///
/// The independent per-capture assemblies run on rayon's bounded pool and are
/// collected in admission order; `game/` and `expected/` are then written one
/// capture at a time, on this thread, in that order. Writes stay serial so the
/// artifact bytes, the `written` list order and the close-out fixpoint are
/// exactly what the serial loop produced. The trade-off is that all assembled
/// results are resident until the writes finish — the whole fixture is briefly
/// held at once instead of one capture at a time. This is a dev tool with no
/// overall RAM ceiling (the per-chunk `--max-decompressed-bytes` still applies),
/// and the write side remains the serial, deterministic path.
///
/// **Aborts on any A7-A12 failure.** The rows are reported first — `report_failed`
/// prints each one, so the operator sees which comparison failed rather than
/// only that something did — and the run then fails as `assertion`.
pub fn build(i: FixtureInput<'_>) -> Result<BuiltFixture, Failure> {
    let chunk_source = ChunkSource::from_index(i.dir, i.index, i.max_decompressed_bytes);
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
        shapes: Vec::new(),
        written: Vec::new(),
        reused: 0,
        fetched: 0,
        moved: 0,
        failures: 0,
    };

    if !i.no_chunk_blobs {
        // Each capture's assembly is independent: zstd decode + MD5 + the patch
        // flow, reading only the shared, read-only store and chunk source. Each
        // worker opens its own blob file handle through `ChunkSource::decoded`,
        // which streams the frame through a buffered reader. Spread the CPU work
        // over rayon's bounded pool, collecting into a Vec in admission order.
        //
        // **Only the assembly runs on the pool.** `record` — the ordered writes
        // of `patchmaps/<id>.json`, `game/`, `expected/`, the selfcheck rows, the
        // digests, `path_map` and the tracing lines — stays here on the caller
        // thread and in admission order, exactly as it did when assembly was
        // serial. That is what keeps artifact bytes, the documented write order
        // and the close-out fixpoint (which runs only after this function
        // returns) unchanged. It also means `path_map` is never shared across
        // workers; it is verified `Send + Sync` in its own module regardless.
        let assembled: Vec<Result<assemble::Assembled, String>> = i
            .admitted
            .par_iter()
            .zip(i.pairs.par_iter())
            .map(|(a, pair)| assemble_one(a, pair, i.store.sources(), &chunk_source))
            .collect();

        for (a, got) in i.admitted.iter().zip(assembled) {
            // The six steps below must happen in this order for each capture.
            // A caller that could reorder them would produce a bundle that
            // satisfied A14 and had no patchmap, which is the exact shape of
            // "looks fine, is useless".
            record(
                i.dir,
                a,
                got.map_err(|e| {
                    assertion(
                        &format!("assemble {}", a.path),
                        &ApiError::shape("assemble", e),
                    )
                })?,
                &mut out,
                i.path_map,
            )?;
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
    let mut prelude = gamedir::write_prelude(i.dir, i.biz, i.old_tag, &out.scan_dir, i.audio)
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
                format!("{failures} of {} selfcheck rows failed", out.rows.len()),
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
    path_map: &crate::pathmap::PathMap,
) -> Result<(), Failure> {
    // **No artifact may be replaced by a later one.** `patchmaps/<id>.json` is
    // named from the capture id, so two admissions with one id would silently
    // lose the first capture while the counts still said two. Selection now
    // numbers forced paths per class; this is the writer's own refusal, so a
    // caller that built a `Selection` itself gets the same failure instead of a
    // bundle whose documents disagree. It runs before the first write.
    if out.assembled_ids.contains(&a.capture_id) {
        return Err(assertion(
            &format!("assemble {}", a.path),
            &ApiError::shape(
                "capture ids",
                format!(
                    "capture id {:?} was already assembled; a second write would \
                     replace the first capture's patchmap",
                    a.capture_id
                ),
            ),
        ));
    }
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
    // The patch's own shape, which is what A21-A23 read.
    out.shapes.push(crate::assemble::patch_shape(&got.map));
    out.side_digests.insert(
        a.capture_id.clone(),
        (
            got.old.as_ref().map(|s| s.md5.clone()).unwrap_or_default(),
            got.new.as_ref().map(|s| s.md5.clone()).unwrap_or_default(),
        ),
    );
    out.written
        .push(super::write_patchmap(dir, &a.capture_id, &got.map).map_err(|e| write_err(dir, e))?);
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
                path_map,
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
                path_map,
            )
            .map_err(|e| write_err(dir, e))?,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blobs::chunks::{FilePair, NewFile};
    use crate::blobs::{Sources, chunks::OldFile};
    use crate::hash;
    use crate::manifest::diff::State;
    use crate::manifest::proto::{SophonChunk, SophonChunkFile};
    use crate::select::{Admission, Class};
    use std::collections::BTreeMap;

    /// Compile-time proof that everything shared with a rayon worker is safe to
    /// hand across threads and that a result can travel back. `ChunkSource` in
    /// particular holds the verify memo (`BTreeMap<String, OnceLock<Vec<u8>>>`),
    /// which is sound to share because `OnceLock` synchronises its single fill.
    #[test]
    fn parallel_assembly_types_are_send_and_sync() {
        fn send_sync<T: Send + Sync>() {}
        fn send<T: Send>() {}
        send_sync::<ChunkSource<'static>>();
        send_sync::<crate::blobs::Store>();
        send_sync::<Sources>();
        send::<assemble::Assembled>();
    }

    fn payload(seed: u8, n: usize) -> Vec<u8> {
        (0..n)
            .map(|i| seed.wrapping_mul(31).wrapping_add(i as u8))
            .collect()
    }

    fn chunk(seed: u8, offset: i64, len: usize) -> SophonChunk {
        let body = payload(seed, len);
        let raw = zstd::encode_all(body.as_slice(), 1).unwrap();
        SophonChunk {
            id: format!("{seed:02x}{offset:016x}_{}", hash::md5_hex(&body)),
            uncompressed_md5: hash::md5_hex(&body),
            offset,
            compressed_size: raw.len() as i64,
            uncompressed_size: len as i64,
            unknown_field6: 0,
            compressed_md5: hash::md5_hex(&raw),
        }
    }

    fn new_file(path: &str, chunks: Vec<SophonChunk>) -> SophonChunkFile {
        let mut bytes = Vec::new();
        for c in &chunks {
            let at = c.offset as usize;
            if bytes.len() < at + c.uncompressed_size as usize {
                bytes.resize(at + c.uncompressed_size as usize, 0);
            }
            let seed = u8::from_str_radix(&c.id[..2], 16).unwrap_or(0);
            bytes[at..at + c.uncompressed_size as usize]
                .copy_from_slice(&payload(seed, c.uncompressed_size as usize));
        }
        SophonChunkFile {
            file: path.into(),
            chunks,
            is_folder: false,
            size: bytes.len() as i64,
            md5: hash::md5_hex(&bytes),
        }
    }

    /// Three independent `added` captures, deliberately not all the same size,
    /// so a wrong collection order could not pass by accident.
    fn fixture() -> (Vec<Admission>, Vec<FilePair>, ChunkSource<'static>) {
        let mut admitted = Vec::new();
        let mut pairs = Vec::new();
        let mut memory = BTreeMap::new();
        for (i, (path, len)) in [("a.bin", 32usize), ("b.bin", 64), ("c.bin", 16)]
            .into_iter()
            .enumerate()
        {
            let seed = 10 + i as u8;
            let c = chunk(seed, 0, len);
            let body = payload(seed, len);
            memory.insert(c.id.clone(), body);
            let capture_id = format!("cap{i}");
            admitted.push(Admission {
                capture_id: capture_id.clone(),
                primary_class: Class::Added,
                path: path.to_string(),
                matching_field: "game".to_string(),
                state: State::Added,
                also_satisfies: Vec::new(),
                budget_cost: 0,
                patch_bytes: len as u64,
            });
            pairs.push(FilePair {
                capture_id,
                path: path.to_string(),
                old: None::<OldFile>,
                new: Some(NewFile {
                    tag: "7.1.0".to_string(),
                    build_id: "build".to_string(),
                    entry: new_file(path, vec![c]),
                }),
            });
        }
        (admitted, pairs, ChunkSource::from_memory(memory))
    }

    /// The hard gate for assembly: the ordered results the parallel collector
    /// returns are equal to the serial results, element for element. Because
    /// `record` consumes this Vec in order and performs every ordered write,
    /// equal Vecs mean byte-identical artifacts and write order.
    #[test]
    fn parallel_assembly_matches_serial_in_order() {
        let (admitted, pairs, chunks) = fixture();
        let sources = Sources::from_pairs([("7.1.0".to_string(), "https://cdn/new".to_string())]);
        let serial: Vec<_> = admitted
            .iter()
            .zip(pairs.iter())
            .map(|(a, p)| assemble_one(a, p, &sources, &chunks))
            .collect();
        let parallel: Vec<_> = admitted
            .par_iter()
            .zip(pairs.par_iter())
            .map(|(a, p)| assemble_one(a, p, &sources, &chunks))
            .collect();
        assert_eq!(parallel.len(), 3);
        assert_eq!(parallel, serial, "parallel assembly drifted from serial");
        assert!(parallel.iter().all(|r| r.is_ok()));
    }
}
