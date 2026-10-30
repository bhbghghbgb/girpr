//! Planning the chunk store, and refusing to start it if the plan is wrong.
//!
//! **Lived inline in `run::capture`,** as ~50 lines between the selection and
//! the first blob fetch. Two of them are decisions — "is this retention plan
//! legal" (A12) and "is this store affordable" (`--max-download-bytes`) — and
//! both are now testable without running a capture.
//!
//! # Why the whole plan is one function
//!
//! The expensive mistake is planning *after* fetching. `projected` is what the
//! log announces as the cost of the store, and A12 is checked on the plan
//! rather than on what arrived — because a refusal that lands after the bytes
//! are paid for has already spent them. So the plan, its two checks, and the
//! `tracing::` lines that narrate it are one unit: there is no path that gets a
//! `projected` figure without having passed both gates.

use crate::failure::{Failure, assertion};
use crate::http::ApiError;
use crate::manifest::CapturedManifest;

use super::chunks::{FilePair, Want, retention};
use super::{file_pairs, projected_download};

/// Refuse a store larger than `--max-download-bytes`.
///
/// **A second, independent ceiling.** The budget is charged against `total_cost`
/// in M5; this is over the bytes actually on the wire. Both hold, and they are
/// not the same number, so neither can stand in for the other. A
/// `--no-chunk-blobs` run fetches nothing, so [`plan`] does not call this at all.
///
/// The boundary is `>`, not `>=`: a store that lands exactly on the ceiling is
/// affordable, and a run that refused it would be refusing the case the operator
/// sized their flag for.
///
/// Separate from [`plan`] so the rule and its boundary are testable without a
/// selection, a manifest or a chunk in sight.
pub fn check_byte_ceiling(projected: u64, cap: Option<u64>) -> Result<(), Failure> {
    let Some(cap) = cap else {
        return Ok(());
    };
    if projected > cap {
        return Err(assertion(
            "fetch the chunk blobs",
            &ApiError::shape(
                "max-download-bytes",
                format!(
                    "the admitted captures need {projected} B of chunk blobs and the \
                     ceiling is {cap} B; lower --per-class or raise the ceiling"
                ),
            ),
        ));
    }
    Ok(())
}

/// A planned chunk store.
pub struct ChunkPlan {
    /// One pair per admitted capture, in admission order.
    pub pairs: Vec<FilePair>,
    /// Every distinct chunk the store will hold.
    pub wants: Vec<Want>,
    /// `Σ compressed_size` over `wants` — the cost announced in the log and
    /// checked against `--max-download-bytes`.
    pub projected: u64,
}

/// Plan the chunk store and refuse it if it is illegal or unaffordable.
///
/// Refuses on, in this order:
///
/// 1. **A12**, per admitted capture: `patch_fetch` must be a subset of
///    `materialize_new`, and every reusable chunk must actually be reusable.
///    Checked per capture so the error names the file, not the store.
/// 2. **`--max-download-bytes`**, over the whole store. The budget is charged
///    against `total_cost` in M5; this is a *second*, independent ceiling over
///    the bytes actually on the wire. Both hold.
///
/// **The second check does not apply to `--no-chunk-blobs`.** That run fetches no
/// chunk bytes at all, so the only bytes its plan could bound are hypothetical;
/// refusing it would charge for a store that is never written. `projected` is
/// still computed and returned, because the plan document records it.
pub fn plan(
    fields: &[String],
    manifests: &[Vec<CapturedManifest>],
    selection: &crate::select::Selection,
    old_tag: &str,
    new_tag: &str,
    max_download_bytes: Option<u64>,
    no_chunk_blobs: bool,
) -> Result<ChunkPlan, Failure> {
    let pairs = file_pairs(selection, fields, manifests, old_tag, new_tag);

    for (a, p) in selection.admitted.iter().zip(&pairs) {
        let r = retention(p);
        if !r.a12_holds() {
            return Err(assertion(
                &format!("plan the chunk sets for {}", a.path),
                &ApiError::shape("A12", r.a12_failure().unwrap_or("unknown")),
            ));
        }
        tracing::info!(
            capture_id = %a.capture_id,
            path = %a.path,
            old_chunks = r.ids(super::chunks::Set::MaterializeOld).len(),
            new_chunks = r.ids(super::chunks::Set::MaterializeNew).len(),
            patch_fetch = r.ids(super::chunks::Set::PatchFetch).len(),
            reuse = r.reuse.len(),
            "retention planned"
        );
    }

    let wants = super::chunks::wants(&pairs);
    let projected = projected_download(&wants);
    tracing::info!(
        chunks = wants.len(),
        projected_bytes = projected,
        max_download_bytes = max_download_bytes.unwrap_or(0),
        distinct_files = selection.admitted.len(),
        "chunk store planned"
    );

    // **A metadata-only run is not bound by a wire-byte ceiling.** It is a
    // ceiling on bytes that will cross the wire, and this run sends none. The
    // A12 check above still runs, because a plan that cannot be materialized is
    // wrong whether or not this run writes it.
    //
    // (Admission already charges every capture against the same flag, and the
    // projected store is a distinct-chunk subset of those charges, so this
    // ceiling is a guard against a future charging change more than a limit a
    // current run can hit. It stays because the two numbers are not the same
    // number, and the second is the one that costs bandwidth.)
    if no_chunk_blobs {
        tracing::info!(
            projected_bytes = projected,
            "--no-chunk-blobs: the wire-byte ceiling is not applied to a store \
             nothing fetches"
        );
    } else if let Some(cap) = max_download_bytes {
        check_byte_ceiling(projected, Some(cap))?;
    }

    Ok(ChunkPlan {
        pairs,
        wants,
        projected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::diff::State;
    use crate::manifest::proto::{SophonChunk, SophonChunkFile};
    use crate::select::{Admission, Budget, Class, Selection};

    fn chunk(md5: &str, offset: i64, size: i64) -> SophonChunk {
        SophonChunk {
            id: format!("{md5}_{offset}"),
            uncompressed_md5: md5.to_string(),
            offset,
            compressed_size: size,
            uncompressed_size: size,
            ..Default::default()
        }
    }

    fn file(path: &str, md5: &str, chunks: Vec<SophonChunk>) -> SophonChunkFile {
        let size = chunks.iter().map(|c| c.uncompressed_size).sum();
        SophonChunkFile {
            file: path.into(),
            chunks,
            is_folder: false,
            size,
            md5: md5.into(),
        }
    }

    /// One admitted `identical` capture: the same single chunk on both sides, so
    /// `projected` is exactly that chunk's `compressed_size`.
    fn identical_capture(projected: i64) -> (Selection, Vec<Vec<CapturedManifest>>, Vec<String>) {
        let id = "1111111111111111_22222222222222222222222222222222";
        let f = file("big.bin", "same", vec![chunk(id, 0, projected)]);
        let manifests = vec![vec![
            CapturedManifest::empty("7.0.0", "game", vec![f.clone()]),
            CapturedManifest::empty("7.1.0", "game", vec![f]),
        ]];
        let selection = Selection {
            admitted: vec![Admission {
                capture_id: "identical".into(),
                primary_class: Class::Identical,
                path: "big.bin".into(),
                matching_field: "game".into(),
                state: State::Identical,
                also_satisfies: Vec::new(),
                budget_cost: projected as u64,
                patch_bytes: 0,
            }],
            rejected: Vec::new(),
            forced_skipped: Vec::new(),
            forced_unscored: Vec::new(),
            matrix: Vec::new(),
            bytes_admitted: projected as u64,
            budget: Budget::unlimited(),
        };
        (selection, manifests, vec!["game".to_string()])
    }

    #[test]
    fn a_metadata_only_plan_is_not_refused_by_the_wire_ceiling() {
        // `projected` is what a fetching run would pay. This run fetches nothing,
        // so a ceiling below the projection has nothing to bound.
        let (selection, manifests, fields) = identical_capture(1_000);
        let err = plan(
            &fields,
            &manifests,
            &selection,
            "7.0.0",
            "7.1.0",
            Some(1),
            false,
        )
        .err()
        .expect("the guard is real for a fetching run");
        assert!(
            err.detail.contains("1000 B"),
            "and it names the projection: {}",
            err.detail
        );

        let ok = plan(
            &fields,
            &manifests,
            &selection,
            "7.0.0",
            "7.1.0",
            Some(1),
            true,
        )
        .expect("a run that fetches no chunk bytes has no wire ceiling");
        assert_eq!(
            ok.projected, 1_000,
            "the plan still records what the store would have cost"
        );
    }

    #[test]
    fn a_metadata_only_plan_still_lists_every_want() {
        // Skipping the ceiling must not turn into skipping the plan: the index a
        // `--no-chunk-blobs` run writes still lists every chunk and its roles.
        let (selection, manifests, fields) = identical_capture(1_000);
        let ok = plan(
            &fields,
            &manifests,
            &selection,
            "7.0.0",
            "7.1.0",
            Some(1),
            true,
        )
        .expect("a metadata-only plan is legal");
        assert_eq!(ok.pairs.len(), 1);
        assert_eq!(ok.wants.len(), 1, "the distinct chunk is still planned");
    }

    #[test]
    fn a_store_over_the_ceiling_is_refused_with_both_numbers() {
        // The message has to carry both figures: "you need N, the cap is M" is
        // actionable in a way that "exceeds max-download-bytes" is not, because
        // the operator has to choose between two levers and this names both
        // sides of the comparison.
        let e = check_byte_ceiling(2_000, Some(1_000)).expect_err("over the ceiling");
        assert!(
            e.detail.contains("2000 B") && e.detail.contains("1000 B"),
            "both figures must appear: {}",
            e.detail
        );
        assert!(
            e.detail.contains("--per-class"),
            "and the lever that can fix it: {}",
            e.detail
        );
    }

    #[test]
    fn a_store_exactly_on_the_ceiling_is_affordable() {
        // `>`, not `>=`. A run that refused the exact ceiling would be refusing
        // the case the operator sized their flag for, which is the one case the
        // flag is certainly right about.
        check_byte_ceiling(1_000, Some(1_000)).expect("on the ceiling is under it");
        check_byte_ceiling(999, Some(1_000)).expect("under the ceiling");
    }

    #[test]
    fn no_ceiling_means_never_refused_however_large() {
        check_byte_ceiling(u64::MAX, None).expect("no cap, no refusal");
        // And `Some(0)` is a real cap, not an absent one: a run that asked for
        // zero bytes must not proceed with a store.
        check_byte_ceiling(1, Some(0)).expect_err("Some(0) is a ceiling of zero");
        check_byte_ceiling(0, Some(0)).expect("and zero fits in zero");
    }
}
