//! The salvage order: which chunks can still be salvaged when the moving
//! bytes are **not** staged in RAM.
//!
//! This is the defrag-shaped half of the in-place repair (docs/07 §4.4). A
//! version upgrade shifts a file's content, so a chunk's bytes have to
//! physically move from `r_i` to `o_i` *inside the same file*. There is no
//! staging area: the source and the destination are both in the file, and
//! the copy between them uses the existing 512 KiB buffer and nothing else.
//!
//! The classical answer to "relocate blocks inside a file without a staging
//! area" is to **order the relocations** so that no source is overwritten
//! before it is read. This module computes that order — as a pure function,
//! with no I/O, so it can be swapped for a better algorithm without touching
//! the writer.
//!
//! # The proof, in two lines
//!
//! Write chunk `i`'s destination `W_i = [o_i, o_i+s_i)` from its reuse
//! source `R_i = [r_i, r_i+s_i)` in the same file. The `W`s are disjoint and,
//! after the plan's zero-fill of any hole, they tile `[0, size)`.
//!
//! - **ascending order** (smallest `o` first): before chunk `i` the written
//!   region is `[0, o_i)`, so `R_i` survives **iff `r_i >= o_i`** (and
//!   `r_i + s_i <= O`, the file's current length).
//! - **descending order**: before chunk `i` the written region is
//!   `(o_i+s_i, size)`, so `R_i` survives **iff `r_i <= o_i`** (and
//!   `r_i + s_i <= O`).
//!
//! Neither condition mentions any other chunk, so both are **exactly
//! computable at plan time** — no simulation, no heuristic. A pure function
//! picks whichever direction preserves more reuse and marks the surviving
//! chunks; the rest download. `r_i == o_i` counts for both.
//!
//! A uniform upward shift of a file's content is the descending case; a
//! downward shift is the ascending case. A file whose chunking re-flowed has
//! sources in both directions and keeps the larger half.

use crate::sophon::SophonChunk;

/// A local reuse candidate: `(uncompressed_md5, uncompressed_size, offset)`.
///
/// The same shape as [`crate::repair::manifest::LocalChunkMap`]'s values.
pub type Candidate = (String, i64, i64);

/// The direction to write in, and which chunks survive it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Order {
    /// `true` = ascending (the smallest destination offset first), `false` =
    /// descending.
    pub ascending: bool,
    /// Per chunk: does its reuse source survive until the chunk is written?
    ///
    /// A `false` here does not mean "the source is broken" — it means "the
    /// source would be overwritten before it could be read", so the chunk has
    /// to be downloaded. Whether the source is *intact* is a separate
    /// question, answered by the scan.
    pub live: Vec<bool>,
}

/// The direction that preserves the most reuse, and the chunks it preserves.
///
/// `chunks` are the file's chunks (ascending by offset, as the plan
/// guarantees); `candidates` is the file's local reuse map; `usable_len` is
/// the file's **current** length on disk (a source that reaches past it is
/// not on disk at all).
///
/// The candidate for a chunk is the first entry whose `(md5, size)` match the
/// chunk's — the same rule `try_reuse_slice` uses, so the two never disagree
/// about which bytes a chunk could come from.
pub fn salvage_order(
    chunks: &[SophonChunk],
    candidates: &[Candidate],
    usable_len: u64,
) -> Order {
    let mut ascending_live = vec![false; chunks.len()];
    let mut descending_live = vec![false; chunks.len()];
    for (i, c) in chunks.iter().enumerate() {
        let Some((_, _, r)) = find_candidate(c, candidates) else {
            continue;
        };
        let r = *r as u64;
        let o = c.offset.max(0) as u64;
        let s = c.uncompressed_size.max(0) as u64;
        // A source that reaches past the end of the file is not there.
        if r.saturating_add(s) > usable_len {
            continue;
        }
        if r >= o {
            ascending_live[i] = true;
        }
        if r <= o {
            descending_live[i] = true;
        }
    }
    let up = ascending_live.iter().filter(|b| **b).count();
    let down = descending_live.iter().filter(|b| **b).count();
    // A tie is broken towards ascending: it is the direction that reads the
    // file forwards, and the two tiers produce the same file either way.
    if up >= down {
        Order {
            ascending: true,
            live: ascending_live,
        }
    } else {
        Order {
            ascending: false,
            live: descending_live,
        }
    }
}

/// The candidate that could supply `c`'s bytes, if any.
///
/// The first entry whose `(md5, size)` match the chunk's — the same rule the
/// temp path's `try_reuse_slice` uses.
pub fn find_candidate<'a>(
    c: &SophonChunk,
    candidates: &'a [Candidate],
) -> Option<&'a Candidate> {
    candidates.iter().find(|(md5, sz, _)| {
        md5.eq_ignore_ascii_case(&c.uncompressed_md5) && *sz == c.uncompressed_size
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &str, offset: i64, size: i64) -> SophonChunk {
        SophonChunk {
            id: id.into(),
            uncompressed_md5: format!("md5-{id}"),
            offset,
            compressed_size: size,
            uncompressed_size: size,
            unknown: 0,
            compressed_md5: String::new(),
        }
    }

    fn cand(id: &str, offset: i64) -> Candidate {
        (format!("md5-{id}"), 4, offset)
    }

    #[test]
    fn a_source_after_its_destination_counts_for_the_ascending_order() {
        // r > o: ascending writes the destination first, so the source is
        // still there when it is read.
        let c = chunk("a", 0, 4);
        let o = salvage_order(&[c], &[cand("a", 8)], 16);
        assert!(o.ascending, "ascending keeps the source");
        assert_eq!(o.live, vec![true]);
    }

    #[test]
    fn a_source_before_its_destination_counts_for_the_descending_order() {
        // r < o: only descending leaves the source alone.
        let c = chunk("a", 8, 4);
        let o = salvage_order(&[c], &[cand("a", 0)], 16);
        assert!(!o.ascending, "descending keeps the source");
        assert_eq!(o.live, vec![true]);
    }

    #[test]
    fn a_source_at_its_own_destination_counts_for_both() {
        // r == o: the chunk's own place is its source, and it survives in
        // either direction. Two cases force each direction in turn, and the
        // r == o chunk is live in both of them.
        // Ascending is forced by a chunk whose source sits *above* its
        // destination.
        let up = salvage_order(
            &[chunk("x", 0, 4), chunk("same", 8, 4)],
            &[cand("x", 4), cand("same", 8)],
            16,
        );
        assert!(up.ascending, "two of the three survive ascending");
        assert_eq!(up.live, vec![true, true]);
        // Descending is forced by a chunk whose source sits *below* its
        // destination.
        let down = salvage_order(
            &[chunk("y", 8, 4), chunk("same", 4, 4)],
            &[cand("y", 0), cand("same", 4)],
            16,
        );
        assert!(!down.ascending, "two of the three survive descending");
        assert_eq!(down.live, vec![true, true]);
    }

    #[test]
    fn a_source_past_the_end_of_the_file_counts_for_neither() {
        // The bytes are not on disk, so no direction can save them.
        let c = chunk("a", 0, 4);
        let o = salvage_order(&[c], &[cand("a", 8)], 8);
        assert!(!o.live[0], "the source reaches past the file's end");
    }

    #[test]
    fn a_chunk_with_no_candidate_is_not_live() {
        let c = chunk("a", 0, 4);
        let o = salvage_order(&[c], &[], 16);
        assert_eq!(o.live, vec![false]);
    }

    #[test]
    fn an_empty_candidate_list_yields_an_arbitrary_but_valid_direction() {
        let chunks = vec![chunk("a", 0, 4), chunk("b", 4, 4)];
        let o = salvage_order(&chunks, &[], 16);
        assert_eq!(o.live, vec![false, false]);
        // The direction is a choice, not a claim: both are valid, and the
        // function must simply pick one and say which.
        assert!(o.ascending || !o.ascending);
    }

    /// The worked example from docs/07 §4.4: the content shifted one chunk to
    /// the right. Descending keeps three of the four chunks; ascending keeps
    /// none of them.
    #[test]
    fn a_uniform_shift_is_the_descending_case() {
        // on disk:  AAAA BBBB CCCC DDDD
        // target:   0000 AAAA BBBB CCCC
        // sources:  none  @0   @4   @8
        let chunks = vec![
            chunk("zero", 0, 4), // no source at all
            chunk("a", 4, 4),
            chunk("b", 8, 4),
            chunk("c", 12, 4),
        ];
        let candidates = vec![cand("a", 0), cand("b", 4), cand("c", 8)];
        let o = salvage_order(&chunks, &candidates, 16);
        assert!(!o.ascending, "the sources are all below their destinations");
        assert_eq!(
            o.live,
            vec![false, true, true, true],
            "three of the four are salvageable"
        );
    }

    /// The same file, seen from the other side: a downward shift keeps the
    /// ascending order.
    #[test]
    fn a_downward_shift_is_the_ascending_case() {
        let chunks = vec![chunk("b", 0, 4), chunk("c", 4, 4), chunk("d", 8, 4)];
        let candidates = vec![cand("b", 4), cand("c", 8), cand("d", 12)];
        let o = salvage_order(&chunks, &candidates, 16);
        assert!(o.ascending);
        assert_eq!(o.live, vec![true, true, true]);
    }

    /// A re-flowed file has sources in both directions: the order keeps the
    /// larger half, which is the whole point of computing it.
    #[test]
    fn a_reflowed_file_keeps_the_larger_half() {
        // target: BBBB CCCC AAAA DDDD
        // source: AAAA BBBB CCCC DDDD (at 0, 4, 8, 12)
        let chunks = vec![
            chunk("b", 0, 4),
            chunk("c", 4, 4),
            chunk("a", 8, 4),
            chunk("d", 12, 4),
        ];
        let candidates = vec![cand("b", 4), cand("c", 8), cand("a", 0), cand("d", 12)];
        let o = salvage_order(&chunks, &candidates, 16);
        assert!(o.ascending, "three of the four survive ascending");
        assert_eq!(o.live, vec![true, true, false, true]);
    }

    /// The candidate is chosen by `(md5, size)`, exactly as the temp path
    /// chooses it — the two must never disagree about which bytes a chunk
    /// could come from.
    #[test]
    fn the_candidate_is_matched_by_md5_and_size() {
        let c = chunk("a", 0, 4);
        let candidates = vec![
            ("md5-other".to_string(), 4, 0),
            ("md5-a".to_string(), 8, 4), // right md5, wrong size
        ];
        assert!(find_candidate(&c, &candidates).is_none());
        let candidates = vec![("MD5-A".to_string(), 4, 0)];
        assert!(find_candidate(&c, &candidates).is_some(), "the md5 is case-insensitive");
    }
}
