//! Assembling a file side from chunks, and the patch flow (REUSE / FETCH).

use std::collections::BTreeMap;

use crate::blobs::Sources;
use crate::blobs::chunks::FilePair;
use crate::hash;
use crate::manifest::proto::SophonChunkFile;
use crate::select::Class;

use super::ranges::{Layout, Range};
use super::source::ChunkSource;
use super::types::{
    Assembled, Decision, Expectations, FileResult, PatchMap, PatchOutcome, PatchShape, PlanCounts,
    PlanEntry, Side, SourceRange, Totals,
};

/// The largest file this tool will try to assemble, in bytes.
///
/// A file's `size` arrives straight off the wire as an `i64`, so a manifest can
/// declare anything at all, including `i64::MAX`. The `.max(0)` this replaces
/// guarded the *sign*, which is why it was there — but not the magnitude, so
/// `size = 2^40` was a 1 TB allocation attempt. That is not a panic and so not
/// catchable: it is an abort the tool cannot report and cannot recover from,
/// which is the worst of the three outcomes.
///
/// 1 GiB is ~47x the largest file in the reference bundle (22 662 068 B), so no
/// legitimate fixture comes near it, and past this line "this manifest is wrong"
/// is a more useful answer than "this machine ran out of memory".
///
/// **One guard, both callers.** `assemble_side` and `materialize::rebuild` used
/// to each carry their own `vec![0u8; size.max(0) as usize]`, so a hostile
/// manifest reached the same abort by two routes. They share [`tile`], so this
/// is checked once.
pub const MAX_ASSEMBLY_BYTES: u64 = 1 << 30;

/// Tile one file from its chunks, and say why it could not.
///
/// **One implementation for both callers.** A capture tiles the manifest entry
/// it was handed; `gircap materialize` tiles the references recorded in
/// `chunks/index.json`. Same algorithm, same three ways to fail — and they were
/// written out twice, in two phrasings, so a reader of one had to learn a second
/// vocabulary to read the other.
///
/// ## What carries the difference: the label
///
/// The only thing that genuinely differs is *which side* is being rebuilt, and
/// that belongs in the label rather than in three parallel sentence templates:
///
/// ```text
/// 7.0.0 GenshinImpact_Data/Plugins/HoYoChannel.dll                (capture)
/// 7.0.0 GenshinImpact_Data/Plugins/HoYoChannel.dll (materialize_old)  (materialize)
/// ```
///
/// So the three messages are the same in both paths, and carry both meanings,
/// because the words describe the *fact* rather than the caller:
///
/// | failure | one wording for both |
/// |---|---|
/// | the layout breaks A11 | `does not satisfy A11 — …` |
/// | a chunk's range is not inside the file | `range {r} is outside the {total} B file` |
/// | a chunk decoded to the wrong length | `decoded to {n} B but its range is {r}` |
///
/// "is outside the *N* B file" replaced two older phrasings: capture said the
/// range "does not fit a *N* B **buffer**", which describes the tool's
/// allocation rather than the fact; materialize said it "is outside the file",
/// which is right but omits the size a reader would want. The new sentence is
/// true of both and says more.
///
/// These strings are **failure diagnostics, not artifact bytes**: an assembly
/// error aborts the capture before the bundle is finished, so nothing here
/// reaches a written file. That is why changing them is a deliberate act rather
/// than a band-3 concern.
///
/// The range is checked **before** the chunk is decoded, so a rebuild that is
/// going to be refused does not first inflate every blob to find out. That was
/// already capture's order; `materialize` used the other one, and the only
/// observable difference is which message a file that is wrong in both ways at
/// once reports.
pub fn tile<'a>(
    label: &str,
    layout: &Layout,
    total: i64,
    ids: &[&str],
    decoded: impl Fn(&str) -> Result<std::borrow::Cow<'a, [u8]>, String>,
) -> Result<Vec<u8>, String> {
    if !layout.is_clean() {
        return Err(format!(
            "{label}: does not satisfy A11 — {}",
            layout.violations.join("; ")
        ));
    }
    if !(0..=MAX_ASSEMBLY_BYTES as i64).contains(&total) {
        return Err(format!(
            "{label}: declares {total} B, past the {MAX_ASSEMBLY_BYTES} B this tool will assemble"
        ));
    }
    let mut bytes = vec![0_u8; total as usize];
    for (id, r) in ids.iter().zip(&layout.ranges) {
        let dst = bytes
            .get_mut(r.start as usize..r.end as usize)
            .ok_or_else(|| {
                format!("{label}: chunk {id} range {r} is outside the {total} B file")
            })?;
        let d = decoded(id)?;
        if d.len() as i64 != r.len() {
            return Err(format!(
                "{label}: chunk {id} decoded to {} B but its range is {r}",
                d.len()
            ));
        }
        dst.copy_from_slice(&d);
    }
    Ok(bytes)
}

/// Assemble one file's side from its retained chunks.
///
/// The layout is checked *before* any byte is placed, so a manifest that
/// violates A11 cannot produce a file that merely happens to have the right MD5
/// by accident — and the error says which of A11's rules broke.
fn assemble_side(
    entry: &SophonChunkFile,
    tag: &str,
    src: &ChunkSource,
) -> Result<FileResult, String> {
    let layout = Layout::of(
        entry.chunks.iter().map(|c| (c.offset, c.uncompressed_size)),
        entry.size,
    );
    let ids: Vec<&str> = entry.chunks.iter().map(|c| c.id.as_str()).collect();
    let label = format!("{tag} {}", entry.file);
    let bytes = tile(&label, &layout, entry.size, &ids, |id| src.decoded(id))?;
    Ok(FileResult {
        tag: tag.to_string(),
        md5: hash::md5_hex(&bytes),
        bytes,
        declared_md5: entry.md5.clone(),
        nchunks: entry.chunks.len(),
        layout,
    })
}

/// Everything one file pair's assembly needs, as one argument.
///
/// Six parameters is past the point where a reader can hold them in their head,
/// and three of them are always the same three things.
pub struct AssemblyInput<'a, 'i> {
    /// The pair to assemble.
    pub pair: &'a FilePair,
    /// The `matching_field` to stamp on the map.
    pub matching_field: &'a str,
    /// The classes the capture answers.
    pub classes: &'a [Class],
    /// Where chunks are fetched from, for the `url` of a FETCH entry.
    pub sources: &'a Sources,
    /// Where decoded bytes come from.
    pub chunks: &'a ChunkSource<'i>,
}

/// Assemble one file pair and return everything this module produces.
///
/// The flow, per new-side chunk, in the order the spec states it:
///
/// 1. If this *old file* holds a chunk with the same
///    `(uncompressed_md5, uncompressed_size)`, try to REUSE its bytes.
/// 2. Verify the local slice against the declared decompressed MD5 (A10).
/// 3. If that fails, FETCH instead — and say so, because a silently downgraded
///    REUSE is indistinguishable from one that was never a candidate.
pub fn assemble(input: &AssemblyInput<'_, '_>) -> Result<Assembled, String> {
    let pair = input.pair;
    let old_entry = pair.old.as_ref().map(|o| &o.entry);
    let new_entry = pair.new.as_ref().map(|n| &n.entry);
    let old_tag = pair.old.as_ref().map(|o| o.tag.clone()).unwrap_or_default();
    let new_tag = pair.new.as_ref().map(|n| n.tag.clone()).unwrap_or_default();

    // **The numbers are validated before the layout.** `assemble_side` builds a
    // `Layout::of` from the declaration, and a captured manifest has already
    // been through this at its decode boundary; a hand-built `FilePair` (tests,
    // a future replay path) has not. One error naming the chunk and the value is
    // better than an A11 row that only says the tiling is wrong.
    for entry in [old_entry, new_entry].into_iter().flatten() {
        entry
            .validate_arithmetic()
            .map_err(|why| format!("{}: {why}", entry.file))?;
    }

    let old = old_entry
        .map(|e| assemble_side(e, &old_tag, input.chunks))
        .transpose()?;
    let new = new_entry
        .map(|e| assemble_side(e, &new_tag, input.chunks))
        .transpose()?;

    // The patch flow needs the old file's *bytes* to verify a REUSE against, so
    // it is assembled once here and handed in rather than rebuilt per chunk.
    //
    // The flow's own bytes come back beside its outcome rather than inside it,
    // because **they are needed for exactly one comparison and then never
    // again**. Keeping them on the outcome would hold them for the rest of the
    // capture — alongside `old.bytes` and `new.bytes`, ~68 MB of peak residency
    // for a file the bundle then writes to disk and stops caring about.
    let (plan, patch, patch_bytes) = match (new_entry, &old) {
        (Some(ne), Some(o)) => {
            let (plan, outcome, bytes) = patch_flow(input, ne, o, &old_tag, &new_tag)?;
            (Some(plan), Some(outcome), Some(bytes))
        }
        // `added`: no old side, so there is nothing to *patch from* - but there
        // is still a plan, and every entry in it is FETCH. A consumer handed
        // this path has to download every new-side chunk, so `plan: null` /
        // `totals: null` would tell it nothing about that. Leaving them null makes
        // `expectations.download_bytes` fall back to a defaulted `0`, which is
        // not "unrecorded" but *false*: the one-chunk `added` file costs
        // 114 357 B to repair and the artifact would claim zero.
        (Some(ne), None) => (Some(fetch_all_plan(input, ne, &new_tag)?), None, None),
        // `removed`: no new side, so there is no plan and no target.
        (None, _) => (None, None, None),
    };

    // A9 is a comparison between two results that only exist together, so it is
    // settled here rather than inside the flow — and `patch_bytes` dies on the
    // next line, so the ~22 MB is not resident while the bundle is written.
    //
    // **A byte comparison, not a digest comparison**, because `docs/00-spec.md` §6
    // specifies A9 as byte equality. Both buffers are therefore necessarily alive
    // for the compare; what this buys is that they are not alive afterwards.
    //
    // **Two things worth knowing about that verdict.**
    //
    // First, `equals_new` reaches production only through the `assembled` log
    // line in `emit::fixture`. The *artifact's* A9 row is built by
    // `selfcheck_rows` from `p.md5` against `n.md5` — a digest comparison. So the
    // spec's byte-equality assertion and the bundle's A9 row are not the same
    // claim; closing that gap is artifact-visible and is not done here.
    //
    // Second, that gap is why this **cannot** be covered by a negative test. Every
    // reachable disagreement also changes the digests, so the row goes red either
    // way, and replacing this comparison with a constant `true` passes the entire
    // suite — verified by mutation, twice.
    // `a9_is_red_when_the_patch_flow_and_the_direct_assembly_disagree` pins the
    // part that *is* reachable: that a disagreement is visible at all.
    let mut patch = patch;
    if let (Some(n), Some(p)) = (&new, &mut patch) {
        p.equals_new = patch_bytes.as_ref().is_some_and(|b| b == &n.bytes);
    }
    drop(patch_bytes);

    // The plan is **moved**, not borrowed: `build_map` only ever reads it, and a
    // `to_vec()` on the way in would deep-copy every entry's ids, URLs, notes and
    // sizes — ~300 allocations per capture — to hand it a copy nobody mutates.
    let map = build_map(input, old.as_ref(), new.as_ref(), plan)?;
    Ok(Assembled {
        map,
        old,
        new,
        patch,
    })
}

/// The patch flow proper.
fn patch_flow(
    input: &AssemblyInput<'_, '_>,
    new_entry: &SophonChunkFile,
    old: &FileResult,
    old_tag: &str,
    new_tag: &str,
) -> Result<(Vec<PlanEntry>, PatchOutcome, Vec<u8>), String> {
    let pair = input.pair;
    let Some(old_entry) = &pair.old else {
        return Err(format!("{}: no old entry to reuse from", pair.path));
    };
    let mut plan = Vec::with_capacity(new_entry.chunks.len());
    let mut reused = 0_usize;
    let mut fetched = 0_usize;
    let mut moved = 0_usize;
    let mut reuse_failures = Vec::new();
    let mut rewrites = Vec::new();
    let mut out = Vec::with_capacity(new_entry.size.max(0) as usize);

    // Where this old file holds each identity, so a REUSE candidate is one
    // lookup rather than a scan per chunk. `or_insert` keeps the *first* one:
    // two chunks of the same file with the same content are interchangeable,
    // and taking the earliest keeps the choice deterministic.
    let mut old_by_identity: BTreeMap<(String, i64), &crate::manifest::proto::SophonChunk> =
        BTreeMap::new();
    for c in &old_entry.entry.chunks {
        old_by_identity
            .entry((c.uncompressed_md5.clone(), c.uncompressed_size))
            .or_insert(c);
    }

    for (i, c) in new_entry.chunks.iter().enumerate() {
        let target = Range::at(c.offset, c.uncompressed_size);
        let key = (c.uncompressed_md5.clone(), c.uncompressed_size);

        // Each arm yields (decision, source_range, note, body). The REUSE arm is
        // tried whenever the old file holds the same content; whether it is
        // *kept* depends on the MD5 of the local slice, which is A10 and the
        // reason a REUSE is safe at all.
        let (decision, source_range, note, body) = match old_by_identity.get(&key).copied() {
            None => (
                Decision::Fetch,
                None,
                None,
                input.chunks.decoded(&c.id)?.into_owned(),
            ),
            Some(oc) => {
                let src_range = Range::at(oc.offset, c.uncompressed_size);
                let verified = src_range
                    .slice(&old.bytes)
                    .ok_or_else(|| format!("old slice {src_range} is outside the old file"))
                    .and_then(|slice| {
                        let got = hash::md5_hex(slice);
                        if got == c.uncompressed_md5 {
                            Ok(slice.to_vec())
                        } else {
                            Err(format!(
                                "the old file's bytes at {src_range} are md5 {got}, but the new \
                                 chunk {} declares {}",
                                c.id, c.uncompressed_md5
                            ))
                        }
                    });
                match verified {
                    Ok(bytes) => {
                        let note = if src_range.start != target.start {
                            Some(format!(
                                "moved: {} in the old file, {} in the new one",
                                src_range.start, target.start
                            ))
                        } else {
                            Some("byte-identical to the old chunk at the same offset".into())
                        };
                        (
                            Decision::Reuse,
                            Some(SourceRange {
                                start: src_range.start,
                                end: src_range.end,
                                file_tag: old_tag.to_string(),
                            }),
                            note,
                            bytes,
                        )
                    }
                    // The spec's flow is "verify, else FETCH", so the decision
                    // changes — and the rewrite is recorded, because a reader who
                    // sees FETCH has no way to know a REUSE was tried and failed.
                    Err(why) => {
                        let msg = format!("chunk {i} ({}): {why}", c.id);
                        rewrites.push(msg.clone());
                        reuse_failures.push(msg);
                        (
                            Decision::Fetch,
                            None,
                            Some(format!("REUSE attempted and rejected: {why}")),
                            input.chunks.decoded(&c.id)?.into_owned(),
                        )
                    }
                }
            }
        };

        match decision {
            Decision::Reuse => reused += 1,
            Decision::Fetch => fetched += 1,
        }
        if decision == Decision::Reuse
            && source_range
                .as_ref()
                .is_some_and(|s| s.start != target.start)
        {
            moved += 1;
        }
        out.extend_from_slice(&body);

        plan.push(PlanEntry {
            i,
            decision,
            target_range: target,
            source_range,
            chunk_id: c.id.clone(),
            url: match decision {
                Decision::Fetch => input.sources.url(new_tag, &c.id).ok(),
                Decision::Reuse => None,
            },
            compressed_size: c.compressed_size,
            uncompressed_size: c.uncompressed_size,
            uncompressed_md5: c.uncompressed_md5.clone(),
            note,
        });
    }

    if out.len() as i64 != new_entry.size {
        return Err(format!(
            "the patch flow produced {} B, the manifest declares {}",
            out.len(),
            new_entry.size
        ));
    }
    let md5 = hash::md5_hex(&out);
    Ok((
        plan,
        PatchOutcome {
            equals_new: false, // settled by `assemble`, which holds both sides
            md5,
            reused,
            fetched,
            moved,
            reuse_failures,
            rewrites,
        },
        // The bytes go back to `assemble` beside the outcome rather than inside
        // it, so the caller can compare them against the direct assembly and drop
        // them immediately. See `PatchOutcome`.
        out,
    ))
}

/// Assemble the patchmap document from what the run produced.
/// The plan for a capture with no old side: every new-side chunk is FETCH.
///
/// Same shape as [`patch_flow`]'s, because it is the same question - what must a
/// consumer download to produce the new file? The answer for `added` is all of
/// it, so the decision is made here rather than re-derived by the reader. The
/// `note` says why, so a reader who expected a `source_range` is not left
/// guessing whether the field was forgotten.
fn fetch_all_plan(
    input: &AssemblyInput<'_, '_>,
    new_entry: &SophonChunkFile,
    new_tag: &str,
) -> Result<Vec<PlanEntry>, String> {
    Ok(new_entry
        .chunks
        .iter()
        .enumerate()
        .map(|(i, c)| PlanEntry {
            i,
            decision: Decision::Fetch,
            target_range: Range::at(c.offset, c.uncompressed_size),
            source_range: None,
            chunk_id: c.id.clone(),
            url: input.sources.url(new_tag, &c.id).ok(),
            compressed_size: c.compressed_size,
            uncompressed_size: c.uncompressed_size,
            uncompressed_md5: c.uncompressed_md5.clone(),
            note: Some("no old side: this file is new, so every chunk is downloaded whole".into()),
        })
        .collect())
}

/// The four plan counts and the FETCH cost, from one walk.
///
/// **The cost is checked.** `compressed_size` is a manifest number, and a plan
/// that costs a negative number of bytes is a wrong plan, not a small one; an
/// accumulated sum that does not fit `i64` is a typed error naming the chunk and
/// the values. A plain `+=` on either would let a single `compressed_size: -1`
/// reach `Totals` and pass A20: an overflow is a typed error naming the chunk and
/// the values, not a small plan.
pub fn totals_for(
    plan: &[PlanEntry],
    old: Option<&FileResult>,
    new: Option<&FileResult>,
) -> Result<Totals, String> {
    let mut reuse = 0_usize;
    let mut fetch = 0_usize;
    let mut moved = 0_usize;
    let mut download_bytes = 0_i64;
    for e in plan {
        match e.decision {
            Decision::Reuse => reuse += 1,
            Decision::Fetch => {
                fetch += 1;
                if e.compressed_size < 0 {
                    return Err(format!(
                        "chunk {}: compressed_size {} is negative; a plan cost cannot be \
                         negative",
                        e.chunk_id, e.compressed_size
                    ));
                }
                download_bytes =
                    download_bytes
                        .checked_add(e.compressed_size)
                        .ok_or_else(|| {
                            format!(
                                "chunk {}: compressed_size {} overflows the plan's download_bytes \
                         (running total {download_bytes} B)",
                                e.chunk_id, e.compressed_size
                            )
                        })?;
            }
        }
        if e.is_moved() {
            moved += 1;
        }
    }
    Ok(Totals {
        chunks: plan.len(),
        reuse,
        fetch,
        moved,
        download_bytes,
        materialize_old_bytes: old.map(|o| o.bytes.len() as i64).unwrap_or(0),
        materialize_new_bytes: new.map(|n| n.bytes.len() as i64).unwrap_or(0),
    })
}

fn build_map(
    input: &AssemblyInput<'_, '_>,
    old: Option<&FileResult>,
    new: Option<&FileResult>,
    plan: Option<Vec<PlanEntry>>,
) -> Result<PatchMap, String> {
    let side = |r: &FileResult| Side {
        tag: r.tag.clone(),
        size: r.bytes.len() as i64,
        md5: r.declared_md5.clone(),
        nchunks: r.nchunks,
    };
    let totals = plan.as_ref().map(|p| totals_for(p, old, new)).transpose()?;
    // `plan` is `Some` for every capture that has a new side, so `totals` is too.
    // Checked rather than defaulted: a `download_bytes` of `0` on a capture that
    // does have a plan would be a false claim, and one with no plan would be
    // unrecordable. Either way a silent `0` is the wrong answer.
    if new.is_some() && totals.is_none() {
        return Err(format!(
            "{}: a capture with a new side must carry a plan, or its cost is unrecordable",
            input.pair.path
        ));
    }
    // Every capture has expectations - `added` asserts the file appears, `removed`
    // asserts it is gone, and a two-sided capture asserts the new MD5.
    //
    // A capture with **no** side at all is reachable and is not an error: a
    // path selected out of the audio pool has no entry in the `game` manifests,
    // which `run` says outright ("that is a bug there, and here it surfaces as a
    // one-sided pair rather than a wrong download"), so it is folded in with
    // `removed`: there is no result file and nothing is downloaded. That is a weaker capture than the other classes,
    // and saying so is better than claiming a result that does not exist.
    let expectations = match new {
        Some(n) => Expectations {
            result_path: Some(format!(
                "{}/{}",
                crate::emit::EXPECTED_TREE,
                input.pair.path
            )),
            result_md5: Some(n.declared_md5.clone()),
            absent_after: false,
            download_bytes: totals.map_or(0, |t| t.download_bytes),
            must_not_fetch: plan
                .as_ref()
                .map(|p| {
                    p.iter()
                        .filter(|e| e.decision == Decision::Reuse)
                        .map(|e| e.chunk_id.clone())
                        .collect()
                })
                .unwrap_or_default(),
        },
        None => Expectations {
            result_path: None,
            result_md5: None,
            absent_after: true,
            // Known, and worth stating: removing a file, or having none to
            // begin with, downloads nothing. `Vec::new()` on `must_not_fetch`
            // is likewise a statement - there is nothing to fetch, so nothing is
            // forbidden.
            download_bytes: 0,
            must_not_fetch: Vec::new(),
        },
    };
    Ok(PatchMap {
        schema_version: crate::SCHEMA_VERSION,
        capture_id: input.pair.capture_id.clone(),
        path: input.pair.path.clone(),
        matching_field: input.matching_field.to_string(),
        classes: input.classes.to_vec(),
        old: old.map(side),
        new: new.map(side),
        plan,
        totals,
        expectations,
    })
}

/// The patch's shape, as the coverage assertions read it.
///
/// The plan's own arithmetic, and the two sides' sizes. **An absent side is
/// `None`, not `Some(0)`**: the file is not there, which is not the same as a
/// file of zero bytes, and the difference is what makes "a file that grows" and
/// "a file that shrinks" true of a zero-byte file.
pub fn patch_shape(map: &PatchMap) -> PatchShape {
    PatchShape {
        plan: map.totals.map(|t| PlanCounts {
            chunks: t.chunks as u64,
            reuse: t.reuse as u64,
            moved: t.moved as u64,
        }),
        old_size: map.old.as_ref().map(|s| s.size.max(0) as u64),
        new_size: map.new.as_ref().map(|s| s.size.max(0) as u64),
    }
}
