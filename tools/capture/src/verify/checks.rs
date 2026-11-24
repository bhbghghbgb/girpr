//! Per-blob and per-patchmap re-verification checks: A6, A7, A8, A11, A18 and A20.
//!
//! `P1`/`P2` below are the optional persistent records' own checks. They are
//! absent for a bundle written before the records existed, and byte-exact for
//! one that carries them.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use rayon::prelude::*;

use crate::assemble::{Decision, PatchMap};
use crate::blobs::chunks::Set;
use crate::blobs::props::BlobProps;
use crate::blobs::{ChunkIndex, IndexedBlob};
use crate::emit::index::Manifest;
use crate::emit::selfcheck::Assertion;
use crate::hash;

use super::bundle::Bundle;
use super::materialize;
use super::materialize::ChunksByFile;
use super::report::{A18, A18_ID, REDERIVED};

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
pub fn check_blobs(root: &Path, index: &ChunkIndex, ceiling: u64) -> Vec<Assertion> {
    // Every blob is independent: A6 re-derives one row from one stored file, with
    // no shared state and no dependence on any other blob. The MD5 + SHA-256 +
    // zstd measure is CPU-bound, so spread it over rayon's bounded pool. Each
    // worker opens its own file handle (`check_one_blob` calls `std::fs::read`,
    // which opens and closes its own descriptor); nothing is shared but the
    // read-only index and root.
    //
    // Completion order is not output order. Collect `(id, row)` and sort by id:
    // `ChunkIndex::blobs` is documented and built as ordered by id, so this
    // reproduces the serial loop's row order exactly for every bundle's index.
    // `assertions_from_parallel_matches_serial` pins that byte-for-byte equality.
    let mut rows: Vec<(&str, Assertion)> = index
        .blobs
        .par_iter()
        .map(|b| (b.props.chunk_id.as_str(), check_one_blob(root, b, ceiling)))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(b.0));
    rows.into_iter().map(|(_, row)| row).collect()
}

/// A6 for one indexed blob: the single row the serial loop used to push inline.
fn check_one_blob(root: &Path, b: &IndexedBlob, ceiling: u64) -> Assertion {
    let id = b.props.chunk_id.as_str();
    let Some(rel) = b.props.stored_path.as_deref() else {
        let why = match b.props.why_unusable() {
            Some(reason) => {
                format!("no path: the index lists it but nothing was stored ({reason})")
            }
            None => "no path: the index lists it but nothing was stored".into(),
        };
        return Assertion::fail(
            "A6",
            crate::emit::selfcheck::wording("A6"),
            id,
            "a stored blob at a path",
            &why,
        );
    };
    // Defence in depth: `Bundle::open` refuses this shape before a reader
    // gets here, but A6 is also callable through `reverify` on an index a
    // caller built itself. The join must never be the first check.
    if let Some(problem) = crate::paths::relative_path_problem(rel) {
        return Assertion::fail(
            "A6",
            crate::emit::selfcheck::wording("A6"),
            rel,
            "a stored blob at a path inside the bundle",
            &format!("unsafe path: {problem}"),
        );
    }
    // **Bytes at a path are not a usable blob.** Version 3 keeps a partial
    // fetch and a blob whose measurement did not return, both with
    // `stored_path` set. A6 is about a chunk that can be assembled from, so
    // it asks the same predicate assembly does rather than only whether the
    // file exists.
    if let Some(why) = b.props.why_unusable() {
        return Assertion::fail(
            "A6",
            crate::emit::selfcheck::wording("A6"),
            rel,
            "a complete, measured blob with a valid fetch",
            &why,
        );
    }
    let path = root.join(rel);
    match std::fs::read(&path) {
        Err(e) => Assertion::fail(
            "A6",
            crate::emit::selfcheck::wording("A6"),
            rel,
            &describe_indexed(&b.props),
            &format!("unreadable: {e}"),
        ),
        Ok(raw) => Assertion::check(
            "A6",
            crate::emit::selfcheck::wording("A6"),
            rel,
            &describe_indexed(&b.props),
            &describe_bytes(&raw, ceiling),
        ),
    }
}

/// How `chunks/index.json` describes a stored blob, in one comparable string.
fn describe_indexed(p: &BlobProps) -> String {
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
fn describe_bytes(raw: &[u8], ceiling: u64) -> String {
    if !hash::is_zstd(raw) {
        return format!("{} B, not a zstd frame", raw.len());
    }
    // Both compressed digests in one pass, and the decoded pair streamed rather
    // than materialised — `describe_bytes` exists to *describe*, so a multi-megabyte
    // decoded buffer it then throws away was the wrong shape entirely.
    let compressed = hash::md5_sha256(raw);
    match hash::zstd_measure(raw, "stored blob", ceiling) {
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
pub fn check_pair(root: &Path, pair: &materialize::Pair, map: &PatchMap) -> Vec<Assertion> {
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
        // opened, and this is the join that would read it. A device component is
        // legal in this *logical* path and was written under an escaped name, so
        // the read follows the same escape the capture used.
        if let Some(problem) = crate::paths::logical_path_problem(&rel) {
            rows.push(Assertion::fail(
                A18_ID,
                A18,
                &rel,
                "a path inside the bundle",
                &format!("unsafe path: {problem}"),
            ));
            continue;
        }
        let on_disk = crate::paths::escape_path(&rel);

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
        match std::fs::read(root.join(&on_disk)) {
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
pub const A20_ID: &str = "A20";
pub const A20: &str = "a patchmap's totals and expectations are the arithmetic of its own plan";

pub fn check_patchmap_arithmetic(map: &PatchMap) -> Vec<Assertion> {
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
        .filter(|e| e.decision == Decision::Fetch)
        .collect();
    let reuse: Vec<_> = plan
        .iter()
        .filter(|e| e.decision == Decision::Reuse)
        .collect();
    // **The cost is a sum over untrusted documents.** A plain `.sum::<i64>()`
    // aborted on two `i64::MAX` entries in debug and wrapped to a negative
    // number in release — which a negative `expectations.download_bytes` then
    // matched, turning "costs -2 bytes" into a pass. One unrepresentable cost is
    // enough: the remaining arithmetic would only be arithmetic on a number that
    // is not a byte count.
    let mut bytes: i64 = 0;
    for e in &fetch {
        if e.compressed_size < 0 {
            rows.push(Assertion::fail(
                A20_ID,
                A20,
                &format!("{subject} (download_bytes)"),
                "every FETCH compressed_size to be a non-negative byte count",
                &format!(
                    "chunk {} declares compressed_size {}",
                    e.chunk_id, e.compressed_size
                ),
            ));
            return rows;
        }
        match bytes.checked_add(e.compressed_size) {
            Some(next) => bytes = next,
            None => {
                rows.push(Assertion::fail(
                    A20_ID,
                    A20,
                    &format!("{subject} (download_bytes)"),
                    "the FETCH compressed sizes to fit i64",
                    &format!(
                        "the sum overflows i64 at chunk {} (compressed_size {}, running \
                         total {bytes} B)",
                        e.chunk_id, e.compressed_size
                    ),
                ));
                return rows;
            }
        }
    }
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

/// The index's view of one patch, in the three roles the plan talks about.
///
/// Built once per capture and shared by every plan entry *and* the converse
/// walk, so a chunk placed twice in one file is matched by offset rather than
/// by id alone. The keys are offsets: the plan names a chunk by where it lands,
/// and the same id may legitimately land in more than one place.
struct IndexedPatch<'i> {
    /// New-side chunk id at each target offset.
    new_by_offset: BTreeMap<i64, &'i str>,
    /// Target offsets the index retains for the patch's fetch set.
    fetch_offsets: BTreeSet<i64>,
    /// Old-side chunk id at each source offset.
    old_by_offset: BTreeMap<i64, &'i str>,
}

/// Group one capture's index references into the view the plan is checked
/// against. [`materialize::ChunksByFile`] already holds the grouping; this only
/// picks the three runs out of it once.
fn indexed_patch<'i>(map: &PatchMap, by_file: &ChunksByFile<'i>) -> IndexedPatch<'i> {
    let tag_new = map.new.as_ref().map_or("", |s| s.tag.as_str());
    let tag_old = map.old.as_ref().map_or("", |s| s.tag.as_str());
    let mut new_by_offset = BTreeMap::new();
    for (id, offset) in by_file
        .get(tag_new, &map.capture_id, &map.path, Set::MaterializeNew)
        .unwrap_or_default()
    {
        new_by_offset.entry(offset).or_insert(id);
    }
    let fetch_offsets = by_file
        .get(tag_new, &map.capture_id, &map.path, Set::PatchFetch)
        .unwrap_or_default()
        .into_iter()
        .map(|(_, offset)| offset)
        .collect();
    let mut old_by_offset = BTreeMap::new();
    for (id, offset) in by_file
        .get(tag_old, &map.capture_id, &map.path, Set::MaterializeOld)
        .unwrap_or_default()
    {
        old_by_offset.entry(offset).or_insert(id);
    }
    IndexedPatch {
        new_by_offset,
        fetch_offsets,
        old_by_offset,
    }
}

/// A20-family: a patchmap's `plan` and the blob closure in `chunks/index.json`
/// must describe the same patch.
///
/// [`check_patchmap_arithmetic`] re-derives a plan's arithmetic from the plan
/// itself. That proves the plan agrees with *itself*, which is exactly what a
/// stale but tidy plan does. This check holds the plan against the index the
/// rebuilt sides came from: the target offsets and chunk ids, the decision the
/// index recorded with its `patch_fetch` role, the old-side source of each
/// reuse, the sizes the index measured, and the converse — no new-side
/// reference is left without a plan entry.
///
/// No network: the URL of a FETCH is only checked for naming its chunk id.
pub fn check_plan_against_index(
    map: &PatchMap,
    by_file: &ChunksByFile<'_>,
    index: &ChunkIndex,
) -> Vec<Assertion> {
    let mut rows = Vec::new();
    let subject = format!("patchmaps/{}.json", map.capture_id);
    let Some(plan) = map.plan.as_ref() else {
        // `removed` has no target and so no plan; there is nothing to compare.
        return rows;
    };
    if map.new.is_none() {
        rows.push(Assertion::fail(
            A20_ID,
            A20,
            &subject,
            "a plan implies a new side",
            "a plan with no new side to target",
        ));
        return rows;
    }

    let view = indexed_patch(map, by_file);
    let tag_old = map.old.as_ref().map_or("", |s| s.tag.as_str());
    let mut planned_offsets: BTreeMap<i64, usize> = BTreeMap::new();

    for (pi, e) in plan.iter().enumerate() {
        let where_ = format!("{subject} [plan {pi}]");
        let offset = e.target_range.start;

        // Duplicate target offsets are never a legitimate placement: two
        // entries writing the same offset tell two different stories about one
        // byte range. A repeated *chunk id* at different offsets is fine.
        if let Some(first) = planned_offsets.insert(offset, pi) {
            rows.push(Assertion::fail(
                A20_ID,
                A20,
                &format!("{where_} (target_range.start)"),
                "one plan entry per target offset",
                &format!("offset {offset} is also the target of plan entry {first}"),
            ));
        }

        // The chunk id a plan places at an offset must be the one the index
        // places there. Matching on offset (not on id alone) is what keeps a
        // repeated placement honest.
        match view.new_by_offset.get(&offset) {
            None => rows.push(Assertion::fail(
                A20_ID,
                A20,
                &format!("{where_} (target_range.start)"),
                "a new-side index reference at the target offset",
                &format!("the index has none at offset {offset}"),
            )),
            Some(indexed_id) if *indexed_id != e.chunk_id => rows.push(Assertion::fail(
                A20_ID,
                A20,
                &format!("{where_} (chunk_id)"),
                &format!("{} (index, at offset {offset})", indexed_id),
                &e.chunk_id,
            )),
            Some(_) => {}
        }

        // Decision, and the fields that decision brings with it.
        let indexed_fetch = view.fetch_offsets.contains(&offset);
        match e.decision {
            Decision::Fetch => {
                if e.source_range.is_some() {
                    rows.push(Assertion::fail(
                        A20_ID,
                        A20,
                        &format!("{where_} (source_range)"),
                        "no source range for a FETCH",
                        "a source range",
                    ));
                }
                if !indexed_fetch {
                    rows.push(Assertion::fail(
                        A20_ID,
                        A20,
                        &format!("{where_} (decision)"),
                        "FETCH: the index retains this offset for patch_fetch",
                        &format!("FETCH, but the index has no patch_fetch reference at {offset}"),
                    ));
                }
                if !fetch_url_names(&e.chunk_id, e.url.as_deref()) {
                    rows.push(Assertion::fail(
                        A20_ID,
                        A20,
                        &format!("{where_} (url)"),
                        "a URL naming the fetched chunk id",
                        e.url.as_deref().unwrap_or("no URL"),
                    ));
                }
            }
            Decision::Reuse => {
                if indexed_fetch {
                    rows.push(Assertion::fail(
                        A20_ID,
                        A20,
                        &format!("{where_} (decision)"),
                        "REUSE: the index does not retain this offset for patch_fetch",
                        &format!("REUSE, but the index has a patch_fetch reference at {offset}"),
                    ));
                }
                let Some(src) = e.source_range.as_ref() else {
                    rows.push(Assertion::fail(
                        A20_ID,
                        A20,
                        &format!("{where_} (source_range)"),
                        "a source range for a REUSE",
                        "no source range",
                    ));
                    continue;
                };
                if src.file_tag != tag_old {
                    rows.push(Assertion::fail(
                        A20_ID,
                        A20,
                        &format!("{where_} (source_range.file_tag)"),
                        tag_old,
                        &src.file_tag,
                    ));
                }
                match view.old_by_offset.get(&src.start) {
                    None => rows.push(Assertion::fail(
                        A20_ID,
                        A20,
                        &format!("{where_} (source_range.start)"),
                        "an old-side index reference at the source offset",
                        &format!("the index has none at offset {}", src.start),
                    )),
                    Some(old_id) => {
                        let len = by_file.size(old_id);
                        let span = src.end.saturating_sub(src.start);
                        if src.end <= src.start || span != len {
                            rows.push(Assertion::fail(
                                A20_ID,
                                A20,
                                &format!("{where_} (source_range.end)"),
                                &format!("{} B, the indexed size of {old_id}", len),
                                &format!("{span} B from offset {}", src.start),
                            ));
                        }
                        // A reuse is a claim about bytes, not about the opaque
                        // id prefix: two builds may publish the same content
                        // under different ids. Accept the same id or the same
                        // decompressed digest on the old side.
                        let content_matches = *old_id == e.chunk_id
                            || index
                                .get(old_id)
                                .is_some_and(|b| b.props.md5_decompressed == e.uncompressed_md5);
                        if !content_matches {
                            rows.push(Assertion::fail(
                                A20_ID,
                                A20,
                                &format!("{where_} (source_range chunk)"),
                                "the old-side source holds the reused bytes",
                                &format!(
                                    "old chunk {old_id} at {} is not {} ({})",
                                    src.start, e.chunk_id, e.uncompressed_md5
                                ),
                            ));
                        }
                    }
                }
            }
        }

        // Sizes, against the blob the index holds for this chunk. An unmeasured
        // row's compressed length is a placeholder zero, so only the declared
        // length is compared there.
        match index.get(&e.chunk_id) {
            None => rows.push(Assertion::fail(
                A20_ID,
                A20,
                &format!("{where_} (chunk_id)"),
                "a blob in chunks/index.json",
                "no indexed blob with that id",
            )),
            Some(blob) => {
                if let Ok(indexed) = i64::try_from(blob.props.uncompressed_size)
                    && indexed != e.uncompressed_size
                {
                    rows.push(Assertion::fail(
                        A20_ID,
                        A20,
                        &format!("{where_} (uncompressed_size)"),
                        &format!("{indexed} (index)"),
                        &e.uncompressed_size.to_string(),
                    ));
                }
                if blob.props.verified.is_some()
                    && let Ok(indexed) = i64::try_from(blob.props.bytes)
                    && indexed != e.compressed_size
                {
                    rows.push(Assertion::fail(
                        A20_ID,
                        A20,
                        &format!("{where_} (compressed_size)"),
                        &format!("{indexed} (index)"),
                        &e.compressed_size.to_string(),
                    ));
                }
            }
        }
    }

    // The converse: a plan that leaves an index reference out is as wrong as
    // one that invents a chunk.
    for (offset, id) in &view.new_by_offset {
        if !planned_offsets.contains_key(offset) {
            rows.push(Assertion::fail(
                A20_ID,
                A20,
                &format!("{subject} (new-side coverage)"),
                &format!("a plan entry for chunk {id} at offset {offset}"),
                "no plan entry names that reference",
            ));
        }
    }

    if rows.is_empty() {
        rows.push(Assertion::pass(
            A20_ID,
            A20,
            &subject,
            &format!(
                "{} plan entry/entries match the {} new-side and {} old-side index references",
                plan.len(),
                view.new_by_offset.len(),
                view.old_by_offset.len()
            ),
        ));
    }
    rows
}

/// Whether a fetch URL names the chunk it is for.
///
/// A URL is `{prefix}/{id}{suffix}` — the suffix is published by the API and
/// may be empty, and a query string may follow. Checking the last path segment
/// starts with the id catches a URL for a different chunk without prescribing
/// the prefix or forbidding a suffix.
fn fetch_url_names(chunk_id: &str, url: Option<&str>) -> bool {
    url.and_then(|u| u.split(['?', '#']).next())
        .and_then(|path| path.rsplit('/').next())
        .is_some_and(|last| last.starts_with(chunk_id))
}

/// Read the optional dispatch record, reporting why P1 cannot hold rather than
/// returning early: P2 is an independent assertion and still needs its row.
fn read_dispatch_record(
    path: &Path,
) -> Result<crate::persistent::DispatchRecord, (String, String)> {
    let raw = std::fs::read(path).map_err(|e| ("readable".into(), e.to_string()))?;
    let record: crate::persistent::DispatchRecord =
        serde_json::from_slice(&raw).map_err(|e| ("parseable".into(), e.to_string()))?;
    if record.schema_version != crate::SCHEMA_VERSION {
        return Err((
            format!("schema {}", crate::SCHEMA_VERSION),
            format!("schema {}", record.schema_version),
        ));
    }
    Ok(record)
}

/// A13, re-read from disk.
///
/// **Delegates to [`crate::emit::selfcheck::check_config_ini`]**, which the
/// capture also calls — one spelling of the read → parse → compare, so
/// `verify`'s rows cannot disagree with the capture's recorded ones.
/// P1 and P2: the optional persistent records, **verified byte-exactly when
/// present and skipped when absent**.
///
/// A16 already re-hashes every file in `checksums.json`, including these; these
/// rows check the *records' own claims* about those files — the envelope's
/// fingerprint and the manifests'/blobs' digests — so a record that was edited
/// to name different bytes cannot pass just because its edit was re-checksummed.
pub fn check_persistent_records(root: &Path, manifest: &Manifest) -> Vec<Assertion> {
    const P1: &str = "the dispatch record's digests match its byte-exact envelope";
    const P2: &str = "the persistent index's digests match its raw manifests and blobs";
    let mut rows = Vec::new();

    let record_path = root.join("dispatch").join("record.json");
    if record_path.is_file() {
        match read_dispatch_record(&record_path) {
            Err((expected, actual)) => rows.push(Assertion::fail(
                "P1",
                P1,
                "dispatch/record.json",
                &expected,
                &actual,
            )),
            Ok(record) => {
                // The envelope is a path from a fixture someone may have edited.
                // Refuse `..`, absolute, backslash and drive paths before the
                // join, exactly as the replay server does.
                if let Some(problem) = crate::paths::relative_path_problem(&record.envelope) {
                    rows.push(Assertion::fail(
                        "P1",
                        P1,
                        "dispatch/record.json",
                        "a bundle-relative envelope path",
                        &problem,
                    ));
                } else {
                    let envelope = root.join(&record.envelope);
                    let mut problems = Vec::new();
                    let actual = match std::fs::read(&envelope) {
                        Ok(b) => {
                            if b.len() as u64 != record.ciphertext_bytes {
                                problems.push(format!(
                                    "ciphertext_bytes {} != envelope length {}",
                                    record.ciphertext_bytes,
                                    b.len()
                                ));
                            }
                            format!("{} {}", hash::md5_hex(&b), hash::sha256_hex(&b))
                        }
                        Err(e) => {
                            problems.push(format!("envelope unreadable: {e}"));
                            format!("unreadable: {e}")
                        }
                    };
                    let expected = format!("{} {}", record.envelope_md5, record.envelope_sha256);
                    let listed = manifest
                        .files
                        .iter()
                        .find(|f| f.path == record.envelope)
                        .map(|f| f.md5.as_str())
                        .unwrap_or("<not listed in MANIFEST.json>");
                    if listed != record.envelope_md5 {
                        problems.push(format!("envelope md5 listed as {listed}"));
                    }
                    rows.push(if actual == expected && problems.is_empty() {
                        Assertion::pass("P1", P1, &record.envelope, &expected)
                    } else {
                        if actual != expected {
                            problems.push(format!("digests {actual} instead of {expected}"));
                        }
                        Assertion::fail("P1", P1, &record.envelope, &expected, &problems.join("; "))
                    });
                }
            }
        }
    }

    let index_path = root.join("persistent").join("index.json");
    if index_path.is_file() {
        let raw = match std::fs::read(&index_path) {
            Ok(b) => b,
            Err(e) => {
                rows.push(Assertion::fail(
                    "P2",
                    P2,
                    "persistent/index.json",
                    "readable",
                    &e.to_string(),
                ));
                return rows;
            }
        };
        let index: crate::persistent::PersistentIndex = match serde_json::from_slice(&raw) {
            Ok(i) => i,
            Err(e) => {
                rows.push(Assertion::fail(
                    "P2",
                    P2,
                    "persistent/index.json",
                    "parseable",
                    &e.to_string(),
                ));
                return rows;
            }
        };
        let mut problems = Vec::new();
        if index.schema_version != crate::SCHEMA_VERSION {
            problems.push(format!(
                "schema {} instead of {}",
                index.schema_version,
                crate::SCHEMA_VERSION
            ));
        }
        let mut files: Vec<(&String, u64, &String)> = index
            .manifests
            .iter()
            .map(|f| (&f.path, f.bytes, &f.md5))
            .collect();
        files.extend(index.blobs.iter().map(|f| (&f.path, f.bytes, &f.md5)));
        for (path, bytes, md5) in files {
            match std::fs::read(root.join(path)) {
                Ok(body) => {
                    if body.len() as u64 != bytes || hash::md5_hex(&body) != *md5 {
                        problems.push(format!("{path} does not match its recorded size/md5"));
                    }
                }
                Err(e) => problems.push(format!("{path}: {e}")),
            }
            if !manifest
                .files
                .iter()
                .any(|f| f.path == *path && f.md5 == *md5)
            {
                problems.push(format!(
                    "{path} is not listed with that md5 in MANIFEST.json"
                ));
            }
        }
        rows.push(if problems.is_empty() {
            Assertion::pass(
                "P2",
                P2,
                "persistent/index.json",
                &format!(
                    "{} manifests, {} blobs",
                    index.manifests.len(),
                    index.blobs.len()
                ),
            )
        } else {
            Assertion::fail(
                "P2",
                P2,
                "persistent/index.json",
                "every recorded digest matches",
                &problems.join("; "),
            )
        });
    }
    rows
}

pub fn check_config_ini(root: &Path, manifest: &Manifest) -> Vec<Assertion> {
    vec![crate::emit::selfcheck::check_config_ini(
        root,
        &manifest.versions.old.tag,
    )]
}

/// Does the recomputation agree with what the capture recorded?
///
/// One direction only, and the direction that matters: a row the capture
/// recorded as *holding* that no longer holds is a bundle that has changed since
/// it was written. A row that previously failed and now holds is an improvement,
/// and is reported through the row counts rather than as a disagreement.
pub fn check_agreement(b: &Bundle, rows: &[Assertion]) -> bool {
    // One value per id: whether *all* rows carrying it are green. `all` over an
    // empty iterator is true, so an id with no recomputed rows defaults to green
    // — exactly what the old per-id scan returned for an id it never matched.
    let mut recomputed: HashMap<&str, bool> = HashMap::new();
    for r in rows {
        recomputed
            .entry(r.id.as_str())
            .and_modify(|all_green| *all_green &= r.ok)
            .or_insert(r.ok);
    }
    let mut ok = true;
    for a in &b.recorded.assertions {
        if !REDERIVED.contains(&a.id.as_str()) || !a.ok {
            continue;
        }
        if recomputed.get(a.id.as_str()).copied().unwrap_or(true) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blobs::props::BlobProps;
    use crate::manifest::proto::SophonChunk;

    fn chunk(id: &str, raw: &[u8], body: &[u8]) -> SophonChunk {
        SophonChunk {
            id: id.to_string(),
            uncompressed_md5: hash::md5_hex(body),
            offset: 0,
            compressed_size: raw.len() as i64,
            uncompressed_size: body.len() as i64,
            unknown_field6: 0,
            compressed_md5: hash::md5_hex(raw),
        }
    }

    fn stored(root: &std::path::Path, id: &str, rel: &str, body: &[u8]) -> IndexedBlob {
        let raw = zstd::encode_all(body, 1).unwrap();
        let full = root.join(rel);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, &raw).unwrap();
        let mut props = BlobProps::measure(&raw, &chunk(id, &raw, body)).unwrap();
        props.set_stored(rel);
        IndexedBlob {
            props,
            referenced_by: Vec::new(),
            sets: Vec::new(),
        }
    }

    fn no_path(id: &str) -> IndexedBlob {
        let body = vec![3u8; 8];
        let raw = zstd::encode_all(&body[..], 1).unwrap();
        IndexedBlob {
            props: BlobProps::unmeasured(&chunk(id, &raw, &body)),
            referenced_by: Vec::new(),
            sets: Vec::new(),
        }
    }

    fn unsafe_path(id: &str) -> IndexedBlob {
        let body = vec![4u8; 8];
        let raw = zstd::encode_all(&body[..], 1).unwrap();
        let mut props = BlobProps::unmeasured(&chunk(id, &raw, &body));
        props.set_stored("../../escape.zst");
        IndexedBlob {
            props,
            referenced_by: Vec::new(),
            sets: Vec::new(),
        }
    }

    /// The hard gate for the parallelization: the ordered rows the parallel
    /// walk emits are byte-for-byte the rows the serial walk emitted, across
    /// every A6 branch (green, unreadable, no path, unsafe path).
    #[test]
    fn assertions_from_parallel_matches_serial() {
        let root = girpr_tempdir::TempDir::new("gircap-a6-parallel", "order");
        // Ids in ascending order, as `ChunkIndex::blobs` is specified to be.
        let blobs = vec![
            stored(&root, "aa", "chunks/aa/01", &[1u8; 64]),
            stored(&root, "bb", "chunks/bb/02", &[2u8; 64]),
            no_path("cc"),
            unsafe_path("dd"),
            stored(&root, "ee", "chunks/ee/05", &[5u8; 64]),
        ];
        // Make "bb" unreadable without changing its index row: the unreadable
        // branch must land in the same slot as it did serially.
        std::fs::remove_file(root.join("chunks/bb/02")).unwrap();
        let index = ChunkIndex::build(blobs);
        let ceiling = crate::config::DEFAULT_MAX_DECOMPRESSED_BYTES;

        let parallel = check_blobs(&root, &index, ceiling);
        let serial: Vec<Assertion> = index
            .blobs
            .iter()
            .map(|b| check_one_blob(&root, b, ceiling))
            .collect();

        assert_eq!(parallel, serial, "parallel A6 rows drifted from serial");
        assert_eq!(parallel.len(), 5);
        assert!(
            parallel[0].ok && parallel[4].ok,
            "the two stored blobs pass"
        );
        assert!(!parallel[1].ok, "the unreadable blob fails");
        assert!(!parallel[2].ok, "the no-path blob fails");
        assert!(!parallel[3].ok, "the unsafe-path blob fails");
    }

    /// The collection must remain correct under repeated runs with a genuinely
    /// parallel pool: completion order cannot leak into the emitted rows.
    #[test]
    fn parallel_a6_rows_are_stable_across_runs() {
        let root = girpr_tempdir::TempDir::new("gircap-a6-parallel", "stable");
        let blobs: Vec<_> = (0..24u8)
            .map(|i| {
                let id = format!("k{i:02}");
                let rel = format!("chunks/k{i:02}/{i:02}");
                stored(&root, &id, &rel, &[i; 32])
            })
            .collect();
        let index = ChunkIndex::build(blobs);
        let ceiling = crate::config::DEFAULT_MAX_DECOMPRESSED_BYTES;
        let first = check_blobs(&root, &index, ceiling);
        for _ in 0..8 {
            assert_eq!(check_blobs(&root, &index, ceiling), first);
        }
    }

    fn read_manifest(root: &Path) -> Manifest {
        let raw = std::fs::read(root.join("MANIFEST.json")).expect("read MANIFEST.json");
        serde_json::from_slice(&raw).expect("parse MANIFEST.json")
    }

    /// A record whose envelope digests match, for the cases where the guard is
    /// the *only* thing standing between a bad path and a green P1 row.
    fn valid_record(envelope: &str, body: &[u8]) -> crate::persistent::DispatchRecord {
        use crate::persistent::{Decoded, Descriptor};

        let (bytes, md5, sha256) = crate::persistent::digest(body);
        let envelope_md5 = md5.clone();
        let envelope_sha256 = sha256.clone();
        let descriptor = |kind: &str, name: &str| Descriptor {
            kind: kind.into(),
            remote_name: name.into(),
            url: format!("http://127.0.0.1:1/{name}"),
            md5: md5.clone(),
            file_size: bytes,
        };
        crate::persistent::DispatchRecord {
            schema_version: crate::SCHEMA_VERSION,
            version: "9.9".into(),
            query_version: "OSRELWin9.9.0".into(),
            key_fingerprint: "test-fingerprint".into(),
            seed: "test-seed".into(),
            envelope: envelope.into(),
            envelope_md5,
            envelope_sha256,
            ciphertext_bytes: bytes,
            decoded: Decoded {
                game_version: "9.9".into(),
                channel_name: "OSRELWin".into(),
                codename: "hk4e_global".into(),
                res_revision: 1,
                data_revision: 2,
                silence_revision: 3,
                audio_revision: 4,
                res_hash: "aa".into(),
                data_hash: "bb".into(),
                silence_hash: "cc".into(),
                audio_hash: "dd".into(),
                repo_res_version_url: "http://127.0.0.1:1/res".into(),
                repo_design_data_url: "http://127.0.0.1:1/data".into(),
                base_revision: "1 aa".into(),
                descriptors: [
                    descriptor("res", "res_versions_external"),
                    descriptor("data", "data_versions"),
                    descriptor("silence_data", "silence_data_versions"),
                ],
            },
        }
    }

    fn write_dispatch(
        root: &Path,
        manifest: &mut Manifest,
        record: &crate::persistent::DispatchRecord,
        body: &[u8],
    ) {
        write_dispatch_raw(
            root,
            &serde_json::to_vec_pretty(record).expect("serialize record"),
        );
        let full = root.join(&record.envelope);
        std::fs::create_dir_all(full.parent().expect("has a parent")).expect("envelope dir");
        std::fs::write(&full, body).expect("write envelope");
        manifest.files.push(crate::emit::index::FileEntry {
            path: record.envelope.clone(),
            bytes: body.len() as u64,
            md5: record.envelope_md5.clone(),
            sha256: record.envelope_sha256.clone(),
            encoding: hash::Encoding::Identity,
        });
    }

    fn write_dispatch_raw(root: &Path, raw: &[u8]) {
        std::fs::create_dir_all(root.join("dispatch")).expect("dispatch dir");
        std::fs::write(root.join("dispatch/record.json"), raw).expect("write record");
    }

    /// Make P2 green so the test says something about its *presence*, not about
    /// a second failure masking the first.
    fn write_persistent_index(root: &Path, manifest: &mut Manifest) {
        use crate::persistent::{ManifestFile, PersistentIndex};

        std::fs::create_dir_all(root.join("persistent")).expect("persistent dir");
        let mut files = Vec::new();
        for (kind, name) in [
            ("res", "res_versions_persist"),
            ("data", "data_versions_persist"),
            ("silence_data", "silence_data_versions_persist"),
        ] {
            let body = format!("{kind} manifest bytes\n").into_bytes();
            let path = format!("persistent/{name}");
            std::fs::write(root.join(&path), &body).expect("write manifest body");
            let md5 = hash::md5_hex(&body);
            manifest.files.push(crate::emit::index::FileEntry {
                path: path.clone(),
                bytes: body.len() as u64,
                md5: md5.clone(),
                sha256: hash::sha256_hex(&body),
                encoding: hash::Encoding::Identity,
            });
            files.push(ManifestFile {
                kind: kind.into(),
                path,
                bytes: body.len() as u64,
                md5,
            });
        }
        let index = PersistentIndex {
            schema_version: crate::SCHEMA_VERSION,
            manifests: files.try_into().expect("three manifests"),
            blobs: Vec::new(),
        };
        std::fs::write(
            root.join("persistent/index.json"),
            serde_json::to_vec_pretty(&index).expect("serialize index"),
        )
        .expect("write index");
    }

    /// A bad `dispatch/record.json` used to `return` a single P1 row, hiding
    /// the independent P2 verdict. Both rows must always be present.
    #[test]
    fn malformed_dispatch_record_still_reports_both_persistent_rows() {
        let body = b"envelope bytes";
        let mut wrong_schema = serde_json::to_value(valid_record("dispatch/response.json", body))
            .expect("serialize record");
        wrong_schema["schema_version"] = serde_json::json!(crate::SCHEMA_VERSION + 1);
        let bad_raws: Vec<Vec<u8>> = vec![
            b"{".to_vec(),
            serde_json::to_vec(&wrong_schema).expect("serialize bad schema"),
        ];

        for raw in bad_raws {
            let (_scratch, root) = gircap_test_support::minimal_bundle("gircap-p1-p2-malformed");
            let mut manifest = read_manifest(&root);
            write_persistent_index(&root, &mut manifest);
            write_dispatch_raw(&root, &raw);

            let rows = check_persistent_records(&root, &manifest);
            assert_eq!(
                rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
                ["P1", "P2"],
                "a broken P1 document must not suppress the P2 row"
            );
            assert!(!rows[0].ok && rows[1].ok, "P1 red, P2 green: {rows:?}");
        }
    }

    /// Every digest matches and the file exists — but *outside* the bundle.
    /// The shared containment guard, not a digest, is what makes this red.
    #[test]
    fn traversal_envelope_is_refused() {
        let (_scratch, root) = gircap_test_support::minimal_bundle("gircap-p1-traversal");
        let mut manifest = read_manifest(&root);
        write_persistent_index(&root, &mut manifest);
        let body = b"escaped envelope bytes";
        write_dispatch(
            &root,
            &mut manifest,
            &valid_record("../escape.bin", body),
            body,
        );

        let rows = check_persistent_records(&root, &manifest);
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["P1", "P2"],
            "the guard must not suppress the P2 row: {rows:?}"
        );
        assert!(!rows[0].ok, "an escaping envelope cannot pass P1: {rows:?}");
        assert!(rows[1].ok, "P2 is independent: {rows:?}");
        assert!(
            rows[0].actual.contains("escapes the bundle root"),
            "the refusal should name the path problem: {:?}",
            rows[0].actual
        );
    }

    /// A record can re-checksum its edited bytes; the byte-count claim is the
    /// part that catches a body that was swapped for one of a different size.
    #[test]
    fn ciphertext_bytes_must_match_the_envelope_length() {
        let (_scratch, root) = gircap_test_support::minimal_bundle("gircap-p1-ciphertext");
        let mut manifest = read_manifest(&root);
        write_persistent_index(&root, &mut manifest);
        let body = b"envelope bytes";
        let mut record = valid_record("dispatch/response.json", body);
        record.ciphertext_bytes += 1;
        write_dispatch(&root, &mut manifest, &record, body);

        let rows = check_persistent_records(&root, &manifest);
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["P1", "P2"],
            "the byte-count failure must not suppress P2: {rows:?}"
        );
        assert!(
            !rows[0].ok,
            "a wrong ciphertext_bytes cannot pass P1: {rows:?}"
        );
        assert!(rows[1].ok, "P2 is independent: {rows:?}");
        assert!(
            rows[0].actual.contains("ciphertext_bytes"),
            "the failure should name the mismatched claim: {:?}",
            rows[0].actual
        );
    }
}
