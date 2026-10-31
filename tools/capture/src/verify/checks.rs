//! Per-blob and per-patchmap re-verification checks: A6, A7, A8, A11, A18 and A20.

use std::path::Path;

use crate::assemble::{Decision, PatchMap};
use crate::blobs::ChunkIndex;
use crate::blobs::props::BlobProps;
use crate::emit::index::Manifest;
use crate::emit::selfcheck::Assertion;
use crate::hash;

use super::bundle::Bundle;
use super::materialize;
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
    let mut rows = Vec::new();
    for b in &index.blobs {
        let id = b.props.chunk_id.as_str();
        let Some(rel) = b.props.stored_path.as_deref() else {
            let why = match b.props.why_unusable() {
                Some(reason) => {
                    format!("no path: the index lists it but nothing was stored ({reason})")
                }
                None => "no path: the index lists it but nothing was stored".into(),
            };
            rows.push(Assertion::fail(
                "A6",
                crate::emit::selfcheck::wording("A6"),
                id,
                "a stored blob at a path",
                &why,
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
        // **Bytes at a path are not a usable blob.** Version 3 keeps a partial
        // fetch and a blob whose measurement did not return, both with
        // `stored_path` set. A6 is about a chunk that can be assembled from, so
        // it asks the same predicate assembly does rather than only whether the
        // file exists.
        if let Some(why) = b.props.why_unusable() {
            rows.push(Assertion::fail(
                "A6",
                crate::emit::selfcheck::wording("A6"),
                rel,
                "a complete, measured blob with a valid fetch",
                &why,
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
                &describe_bytes(&raw, ceiling),
            )),
        }
    }
    rows
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

/// A13, re-read from disk.
///
/// **Delegates to [`crate::emit::selfcheck::check_config_ini`]**, which the
/// capture also calls — one spelling of the read → parse → compare, so
/// `verify`'s rows cannot disagree with the capture's recorded ones.
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
