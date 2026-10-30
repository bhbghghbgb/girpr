//! Executing the chunk-store plan: fetch the blobs, or record that there are
//! none.
//!
//! **Lived inline in `run::capture`,** as a branch between two calls that already
//! existed. The branch is small; what it decides is not.
//!
//! # The index is written either way
//!
//! `--no-chunk-blobs` does not skip the chunk store — it stores nothing and
//! records the plan. The resulting index has every role list filled and no
//! blobs, and that is how a reader tells **"this capture did not fetch"** from
//! **"this capture had nothing to fetch"**. Those are different facts about a
//! bundle and only one of them is a problem, so the document that distinguishes
//! them has to exist in both cases.

use crate::http::Fetcher;

use super::chunks::Want;
use super::{IndexedBlob, Set, Store};

/// What the store ended up holding.
pub struct Filled {
    /// One entry per chunk, with the roles it plays.
    pub entries: Vec<(IndexedBlob, Vec<Set>)>,
    /// Fetches that failed, one entry per chunk, in completion order.
    ///
    /// **Completion order, not `wants` order.** `fill` pushes each error from
    /// `JoinSet::join_next` as its task finishes, so a consumer must not zip this
    /// list against `wants`; pair an error with its chunk through the id the
    /// message starts with, or through the index row. `run` reads only the
    /// length, and the row is written beside the error, so nothing today depends
    /// on the order.
    ///
    /// **Not an error.** A chunk the CDN would not serve is recorded as a blob
    /// with its failure and the run continues; A6 is what decides whether that
    /// is acceptable. Turning it into a `Result::Err` here would collapse "one
    /// chunk failed" and "the store could not be built at all" into one outcome,
    /// and the second is the only one that should stop a capture.
    ///
    /// **One chunk, one entry.** The error and the index row for a failed fetch
    /// are produced together, so a chunk cannot be counted here *and* as a slot
    /// that never reported. `run` prices the summary's failed-fetch count off
    /// this length, which is why the distinction matters.
    pub errors: Vec<String>,
}

/// Fill the store, or record the plan when `--no-chunk-blobs` says not to.
///
/// `projected` is passed only so the `--no-chunk-blobs` narration can state the
/// cost the run declined to pay; on the fetching path the numbers come from what
/// actually arrived.
pub async fn fill_or_plan(
    store: &Store,
    fetcher: &Fetcher,
    wants: &[Want],
    jobs: usize,
    no_chunk_blobs: bool,
    projected: u64,
) -> Filled {
    if no_chunk_blobs {
        tracing::info!(
            chunks = wants.len(),
            projected_bytes = projected,
            "--no-chunk-blobs: the index records the plan and stores nothing"
        );
        return Filled {
            entries: Store::plan_only(wants),
            errors: Vec::new(),
        };
    }
    let (entries, errors) = store.fill(fetcher, wants, jobs).await;
    for e in &errors {
        tracing::error!(error = %e, "chunk fetch failed");
    }
    Filled { entries, errors }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blobs::props::BlobProps;
    use crate::manifest::proto::SophonChunk;

    /// A want for one chunk, spelled out so a new `Want` field is a compile
    /// error here rather than a defaulted value in a test about the flag.
    fn want(id: &str) -> Want {
        Want {
            chunk: SophonChunk {
                id: id.to_string(),
                uncompressed_md5: format!("md5-{id}"),
                uncompressed_size: 1_024,
                compressed_size: 512,
                ..Default::default()
            },
            sources: Vec::new(),
            references: Vec::new(),
            sets: vec![Set::MaterializeNew],
            conflicts: Vec::new(),
        }
    }

    #[test]
    fn the_plan_path_records_every_want_with_no_bytes_and_no_stored_path() {
        // `--no-chunk-blobs` has to produce an index that lists every role and
        // stores nothing. Zero entries would be indistinguishable from "there
        // was nothing to fetch" — the exact confusion the flag exists to avoid.
        let wants = vec![want("aaa"), want("bbb"), want("ccc")];
        let entries = Store::plan_only(&wants);

        assert_eq!(
            entries.len(),
            wants.len(),
            "one entry per distinct chunk, so a reader can see what was skipped"
        );
        for ((blob, _roles), w) in entries.iter().zip(&wants) {
            assert_eq!(blob.props.chunk_id, w.chunk.id);
            assert!(
                blob.props.stored_path.is_none(),
                "{} has no stored path: nothing was fetched",
                w.chunk.id
            );
            assert_eq!(
                blob.props.bytes, 0,
                "{} has no bytes: it was never fetched",
                w.chunk.id
            );
        }
    }

    #[test]
    fn the_plan_path_records_the_sizes_the_manifest_declared() {
        // The one measurement the flag does *not* erase. A reader asking "how
        // big would this have been" has an answer from the manifests, and the
        // index should carry it rather than reporting a blob of unknown size.
        let entries = Store::plan_only(&[want("aaa")]);
        let props: &BlobProps = &entries[0].0.props;
        assert_eq!(props.uncompressed_size, 1_024);
        assert_eq!(props.md5_decompressed, "md5-aaa");
        assert!(
            props.md5_compressed.is_empty(),
            "the compressed digest is only known after a fetch"
        );
    }
}
