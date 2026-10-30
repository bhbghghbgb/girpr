//! The salvage order: which reuse source survives, and in which direction.
//!
//! A chunk's reuse source `R_i = [r_i, r_i+s_i)` and its destination
//! `W_i = [o_i, o_i+s_i)` are both inside the *same* file. The writer may only
//! copy from a source that has not already been overwritten, so the question
//! is which sources a given write order destroys.
//!
//! **Ascending** (lowest `o` first): before chunk `i` is written, the region
//! already written is `[0, o_i)`. `R_i` survives ⟺ `r_i ≥ o_i`.
//!
//! **Descending** (highest `o` first): before chunk `i` is written, the
//! region already written is `(o_i+s_i, size)`. `R_i` survives ⟺ `r_i ≤ o_i`.
//!
//! Neither condition mentions another chunk, so both are **exactly computable
//! at plan time** — no simulation and no heuristic. `r_i == o_i` counts for
//! both (the chunk is already correct where it belongs). A source that
//! reaches past the end of the file counts for neither: those bytes are not
//! there to be copied.
//!
//! The two directions are not symmetric. A uniform upward shift of the
//! content is the descending case, a downward shift is the ascending one, and
//! a file whose chunking re-flowed has sources in both directions and keeps
//! the larger half.
//!
//! # What the rule assumes, and what it survives
//!
//! The rule assumes the destinations are disjoint (the layout is validated at
//! plan time), so "the region
//! already written" is the whole prefix (ascending) or the whole suffix
//! (descending) of the file. When the layout has a hole, the region actually
//! written is a *subset* of that - so a source the rule rejects may really
//! have survived. The rule is therefore **conservative**: never wrong,
//! occasionally pessimistic.
//!
//! The rule does not need to know which chunks are already correct and will
//! not be written. A chunk that is not written is a region that is not
//! touched, and an untouched region destroys fewer sources, not more.
//!
//! # Why the source is read before it is overwritten
//!
//! A write can only destroy a source that has **not** been read yet. In
//! ascending order a chunk reads its source and writes its destination at the
//! same moment, and every later write is at a *higher* offset — so a later
//! write may destroy that source, but only after it was read. In descending
//! order every later write is at a *lower* offset, and the same argument
//! holds. That is why the rule only has to look at the region written
//! *before* the chunk, and why `r_i ≥ o_i` (ascending) is safe even though
//! a later destination may sit on top of the source.
//!
//! # The deferred optimum
//!
//! A full interval-graph optimum - a topological order over "chunk `j`'s
//! write clobbers chunk `i`'s source", breaking cycles by downloading - is
//! possible and is not implemented. The order is produced by
//! this one pure function precisely so that it can be swapped without
//! touching the writer.

use crate::sophon::{FileReuse, SophonChunk};

/// The reuse source of each chunk, in the same order as the chunks: the
/// offset in the same file the chunk's bytes could be copied from, or `None`
/// when there is no candidate.
pub type Sources = Vec<Option<u64>>;

/// The order the writer must use, and which sources survive it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Order {
    /// `true` = write the chunks from the lowest offset up; `false` = from
    /// the highest down.
    pub ascending: bool,
    /// Per chunk, in the order given: does this chunk's reuse source survive
    /// the order? A `false` means the chunk has to be downloaded.
    pub live: Vec<bool>,
}

impl Order {
    /// How many chunks this order keeps.
    pub fn live_count(&self) -> usize {
        self.live.iter().filter(|b| **b).count()
    }
}

/// The order that preserves the most reuse, and which sources survive it.
///
/// `chunks` and `sources` are parallel: `sources[i]` is where chunk `i`'s
/// bytes live. `usable_len` is the length of the file being read from — a
/// source that reaches past it is not there to be copied.
///
/// A tie goes to **ascending**: both directions are valid then, and the
/// ascending one writes forward, which is the friendlier pattern for a
/// spinning disk.
pub fn salvage_order(chunks: &[SophonChunk], sources: &[Option<u64>], usable_len: u64) -> Order {
    order_from(chunks.iter(), sources, usable_len)
}

/// [`salvage_order`] over borrowed chunks.
///
/// Callers that already hold a selection of chunks (the scan's bad chunks, or
/// the in-place path's indices into `file.chunks`) can order them without
/// cloning them first. `sources[i]` is where `chunks[i]`'s bytes live.
pub fn salvage_order_refs(
    chunks: &[&SophonChunk],
    sources: &[Option<u64>],
    usable_len: u64,
) -> Order {
    order_from(chunks.iter().copied(), sources, usable_len)
}

/// The shared rule: count both directions cheaply, then materialize only the
/// winner. The two passes share `survives_one`, so the count and the vector
/// cannot disagree about which sources are live.
fn order_from<'a>(
    chunks: impl Iterator<Item = &'a SophonChunk> + Clone,
    sources: &[Option<u64>],
    usable_len: u64,
) -> Order {
    let kept_ascending = survives_count(chunks.clone(), sources, usable_len, true);
    let kept_descending = survives_count(chunks.clone(), sources, usable_len, false);
    let ascending = kept_descending <= kept_ascending;
    Order {
        ascending,
        live: survives(chunks, sources, usable_len, ascending),
    }
}

/// Whether one chunk's source survives one direction.
///
/// The whole rule, in one place: a source survives when it is inside the
/// file and it is not under the region the order has already written.
fn survives_one(c: &SophonChunk, source: Option<u64>, usable_len: u64, ascending: bool) -> bool {
    let Some(r) = source else {
        return false;
    };
    let o = c.offset.max(0) as u64;
    let s = c.uncompressed_size.max(0) as u64;
    // The source must be inside the file being read from.
    if r.checked_add(s).is_none_or(|end| end > usable_len) {
        return false;
    }
    // Ascending: the region already written is `[0, o)`, so the source has
    // to be at or after the destination. Descending: the region already
    // written is `(o+s, size)`, so the source has to be at or before the
    // destination.
    if ascending { r >= o } else { r <= o }
}

/// How many sources survive one direction, without materializing a vector.
fn survives_count<'a>(
    chunks: impl Iterator<Item = &'a SophonChunk>,
    sources: &[Option<u64>],
    usable_len: u64,
    ascending: bool,
) -> usize {
    chunks
        .enumerate()
        .filter(|(i, c)| survives_one(c, sources.get(*i).copied().flatten(), usable_len, ascending))
        .count()
}

/// Which sources survive one direction.
fn survives<'a>(
    chunks: impl Iterator<Item = &'a SophonChunk>,
    sources: &[Option<u64>],
    usable_len: u64,
    ascending: bool,
) -> Vec<bool> {
    chunks
        .enumerate()
        .map(|(i, c)| survives_one(c, sources.get(i).copied().flatten(), usable_len, ascending))
        .collect()
}

/// The reuse source of each chunk, taken from the local reuse map.
///
/// The map is `path -> FileReuse`, and the **first** entry matching a
/// chunk's `(md5, size)` is its source — the same choice the temp path's
/// reuse lookup makes, so the two cannot disagree about where a chunk's
/// bytes are.
///
/// A chunk with no matching entry has no source: it has to be downloaded.
pub fn sources_for(chunks: &[SophonChunk], reuse: Option<&FileReuse>) -> Sources {
    chunks
        .iter()
        .map(|c| {
            reuse
                .and_then(|r| r.get(c))
                .map(|entry| entry.offset.max(0) as u64)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sophon::ReuseEntry;
    use crate::util::Md5;

    /// A chunk with the given destination offset and length. The salvage rule
    /// does not read the digest, so a fixed one is enough here.
    fn chunk(id: &str, offset: i64, size: i64) -> SophonChunk {
        chunk_at(id, "d41d8cd98f00b204e9800998ecf8427e", offset, size)
    }

    fn chunk_at(id: &str, md5: &str, offset: i64, size: i64) -> SophonChunk {
        SophonChunk {
            id: id.into(),
            uncompressed_md5: md5.into(),
            offset,
            compressed_size: size,
            uncompressed_size: size,
            unknown: 0,
            compressed_md5: String::new(),
        }
    }

    /// One chunk, one source, one direction — the smallest case the rule can
    /// be asked about.
    fn one(offset: i64, size: i64, source: Option<u64>, usable: u64) -> Order {
        let c = chunk("c", offset, size);
        salvage_order(&[c], &[source], usable)
    }

    #[test]
    fn a_source_after_the_destination_counts_for_ascending() {
        // r > o: ascending keeps it, descending does not.
        let order = one(0, 4, Some(8), 64);
        assert!(order.ascending);
        assert_eq!(order.live, vec![true]);
    }

    #[test]
    fn a_source_before_the_destination_counts_for_descending() {
        // r < o: descending keeps it, ascending does not.
        let order = one(8, 4, Some(0), 64);
        assert!(!order.ascending);
        assert_eq!(order.live, vec![true]);
    }

    #[test]
    fn a_source_at_the_destination_counts_for_both() {
        // r == o: the chunk is already correct where it belongs, and it is
        // safe in either direction.
        let chunks = [chunk("c", 4, 4)];
        let sources = [Some(4u64)];
        assert_eq!(survives(chunks.iter(), &sources, 64, true), vec![true]);
        assert_eq!(survives(chunks.iter(), &sources, 64, false), vec![true]);
    }

    #[test]
    fn a_source_past_the_end_of_the_file_counts_for_neither() {
        // The file is 64 bytes; a source at 60 would fit a 4-byte chunk
        // exactly, but at 61 it reaches past the end.
        let order = one(0, 4, Some(61), 64);
        assert_eq!(order.live, vec![false]);
    }

    #[test]
    fn a_chunk_with_no_source_is_never_live() {
        let order = one(0, 4, None, 64);
        assert_eq!(order.live, vec![false]);
    }

    #[test]
    fn an_empty_candidate_list_yields_an_arbitrary_but_valid_direction() {
        let chunks = vec![chunk("a", 0, 4), chunk("b", 4, 4)];
        let order = salvage_order(&chunks, &[None, None], 64);
        assert_eq!(order.live, vec![false, false]);
        assert_eq!(order.live_count(), 0);
    }

    #[test]
    fn the_direction_is_the_one_that_preserves_more_reuse() {
        // Three chunks, all shifted one chunk up: the content moved later,
        // so descending is the direction that keeps it.
        let chunks = vec![chunk("a", 4, 4), chunk("b", 8, 4), chunk("c", 12, 4)];
        let sources = vec![Some(0u64), Some(4), Some(8)];
        let order = salvage_order(&chunks, &sources, 64);
        assert!(!order.ascending, "the content moved up, so descending");
        assert_eq!(order.live, vec![true, true, true]);
    }

    #[test]
    fn a_tie_goes_to_ascending() {
        // One source each way: neither direction is better, and ascending is
        // the one that writes forward.
        let chunks = vec![chunk("a", 0, 4), chunk("b", 8, 4)];
        let sources = vec![Some(8u64), Some(4)];
        let order = salvage_order(&chunks, &sources, 64);
        assert!(order.ascending);
        assert_eq!(order.live, vec![true, false]);
    }

    #[test]
    fn a_source_that_is_past_the_file_counts_for_neither_direction() {
        // The file is 16 bytes; the source at 12 would fit for a 4-byte
        // chunk, but at 16 it does not.
        let chunks = vec![chunk("a", 0, 4)];
        let order = salvage_order(&chunks, &[Some(16)], 16);
        assert_eq!(order.live, vec![false]);
    }

    /// The plan's own worked example, in the plan's own shape: the content
    /// shifted one chunk to the right.
    #[test]
    fn the_worked_example_salvages_three_of_four_in_the_descending_direction() {
        // on disk:  [0,4)=AAAA [4,8)=BBBB [8,12)=CCCC [12,16)=DDDD
        // target:   [0,4)=0000 [4,8)=AAAA [8,12)=BBBB [16-)=CCCC
        // sources:      none      r=0      r=4         r=8
        let chunks = vec![
            chunk("a", 0, 4),
            chunk("b", 4, 4),
            chunk("c", 8, 4),
            chunk("d", 12, 4),
        ];
        let sources = vec![None, Some(0u64), Some(4), Some(8)];
        let order = salvage_order(&chunks, &sources, 16);
        assert!(!order.ascending);
        assert_eq!(order.live, vec![false, true, true, true]);
        assert_eq!(order.live_count(), 3);
    }

    #[test]
    fn the_borrowed_form_agrees_with_the_owned_one() {
        let chunks = vec![
            chunk("a", 0, 4),
            chunk("b", 4, 4),
            chunk("c", 8, 4),
            chunk("d", 12, 4),
        ];
        let sources = vec![None, Some(0u64), Some(4), Some(8)];
        let refs: Vec<&SophonChunk> = chunks.iter().collect();
        assert_eq!(
            salvage_order_refs(&refs, &sources, 16),
            salvage_order(&chunks, &sources, 16)
        );
    }

    #[test]
    fn sources_for_takes_the_first_entry_matching_the_chunk() {
        let md5 = "d41d8cd98f00b204e9800998ecf8427e";
        let chunks = vec![chunk_at("a", md5, 0, 4)];
        let reuse = FileReuse::new(vec![
            ReuseEntry {
                md5: Md5::parse_hex(md5).unwrap(),
                size: 4,
                offset: 12,
            },
            // A duplicate key, spelled differently: the index must still keep
            // the first entry, exactly as the linear scan did.
            ReuseEntry {
                md5: Md5::parse_hex("D41D8CD98F00B204E9800998ECF8427E").unwrap(),
                size: 4,
                offset: 99,
            },
        ]);
        let sources = sources_for(&chunks, Some(&reuse));
        assert_eq!(sources, vec![Some(12)]);
    }

    #[test]
    fn sources_for_matches_the_md5_case_insensitively_and_the_size_exactly() {
        let chunks = vec![chunk_at("a", "d41d8cd98f00b204e9800998ecf8427e", 0, 4)];
        let reuse = FileReuse::new(vec![ReuseEntry {
            md5: Md5::parse_hex("D41D8CD98F00B204E9800998ECF8427E").unwrap(),
            size: 4,
            offset: 12,
        }]);
        assert_eq!(sources_for(&chunks, Some(&reuse)), vec![Some(12)]);
        // The same md5 at a different size is a different chunk.
        let reuse = FileReuse::new(vec![ReuseEntry {
            md5: Md5::parse_hex("D41D8CD98F00B204E9800998ECF8427E").unwrap(),
            size: 5,
            offset: 12,
        }]);
        assert_eq!(sources_for(&chunks, Some(&reuse)), vec![None]);
    }

    #[test]
    fn sources_for_gives_a_chunk_with_no_entry_no_source() {
        let chunks = vec![chunk("a", 0, 4)];
        assert_eq!(sources_for(&chunks, None), vec![None]);
    }
}
