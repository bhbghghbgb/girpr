use super::*;
use crate::blobs::Sources;
use crate::blobs::chunks::{NewFile, OldFile};
use crate::hash;
use crate::manifest::proto::{SophonChunk, SophonChunkFile};

/// `stored_path` comes out of `chunks/index.json`, which is a document in a
/// bundle someone may have edited. It is joined onto the bundle root, so an
/// escaping value must not turn into a read outside it.
#[test]
fn an_index_stored_path_that_escapes_the_bundle_is_refused() {
    use crate::blobs::props::BlobProps;
    use crate::blobs::{ChunkIndex, IndexedBlob};

    let index = ChunkIndex {
        schema_version: crate::SCHEMA_VERSION,
        blobs: vec![IndexedBlob {
            props: BlobProps {
                chunk_id: "x".into(),
                stored_path: Some("../../escape.zst".into()),
                bytes: 0,
                container: "zstd".into(),
                md5_compressed: String::new(),
                md5_decompressed: String::new(),
                sha256_compressed: String::new(),
                uncompressed_size: 0,
                unknown_field6: 0,
                verified: None,
                decode_error: None,
                fetch_error: None,
                partial: false,
                measurement_error: None,
            },
            referenced_by: Vec::new(),
            sets: Vec::new(),
        }],
        sets: std::collections::BTreeMap::new(),
    };
    let e = ChunkSource::from_index(
        std::path::Path::new("artifact"),
        &index,
        crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
    )
    .decoded("x")
    .unwrap_err();
    assert!(e.contains("escapes the bundle root"), "{e}");
    assert!(e.contains("stored_path"), "and names the field: {e}");
}

/// The three ways a rebuild can fail, in one wording.
///
/// There were two sets of three — a capture's and `materialize`'s — and
/// **nothing asserted any of them**, so either could be reworded without a
/// test noticing, and they had already drifted apart. The label is the only
/// thing that differs between the two callers, and that is what these pin.
#[test]
fn a_failed_rebuild_says_which_file_which_range_and_which_size() {
    use std::borrow::Cow;
    let clean = Layout::of([(0, 10)], 10);

    // 1. The layout itself breaks A11 — a gap between the chunks.
    let holed = Layout::of([(0, 10), (20, 10)], 30);
    let e = tile("7.0.0 a/b.dll", &holed, 30, &["x", "y"], |_| {
        Ok(Cow::Borrowed(&b""[..]))
    })
    .unwrap_err();
    assert!(
        e.starts_with("7.0.0 a/b.dll: does not satisfy A11 — "),
        "{e}"
    );
    assert!(
        e.contains("a gap of 10 B between chunk 0"),
        "and says where: {e}"
    );

    // 2. A range outside the file. `total` is its own parameter, so a clean
    //    layout can still describe a range the file cannot hold.
    let e = tile("7.0.0 a/b.dll (materialize_old)", &clean, 4, &["x"], |_| {
        Ok(Cow::Borrowed(&b""[..]))
    })
    .unwrap_err();
    assert_eq!(
        e, "7.0.0 a/b.dll (materialize_old): chunk x range [0, 10) is outside the 4 B file",
        "the label is the only thing that says which caller this is"
    );

    // 3. A chunk that decoded to the wrong length.
    let e = tile("7.0.0 a/b.dll", &clean, 10, &["x"], |_| {
        Ok(Cow::Borrowed(&b"short"[..]))
    })
    .unwrap_err();
    assert_eq!(
        e,
        "7.0.0 a/b.dll: chunk x decoded to 5 B but its range is [0, 10)"
    );
}

/// The range check is a refusal, not a panic.
///
/// `vec![0u8; size.max(0) as usize]` guarded the sign and nothing else, so a
/// manifest declaring `size = 2^40` attempted a 1 TB allocation — an abort the
/// tool cannot report and cannot recover from, because it is not a panic. The
/// limit turns that into the tool's normal "this is a finding" answer.
#[test]
fn a_file_past_the_assembly_limit_is_refused_rather_than_allocated() {
    let clean = Layout::of([(0, 10)], 10);
    for (total, what) in [
        (1_i64 << 40, "2^40 B"),
        (i64::MAX, "i64::MAX B"),
        (-1, "a negative size"),
    ] {
        let e = tile("7.0.0 a/b.dll", &clean, total, &["x"], |_| {
            Ok(std::borrow::Cow::Borrowed(&b""[..]))
        })
        .unwrap_err();
        assert!(
            e.contains("past the 1073741824 B this tool will assemble"),
            "{total} ({what}): {e}"
        );
        assert!(e.starts_with("7.0.0 a/b.dll: "), "and names the file: {e}");
    }
    // One byte over the limit is still over; exactly at it is allowed through to
    // the ordinary allocation, which is what makes the boundary a stated rule
    // rather than a round number.
    let ok = tile(
        "L",
        &Layout::of([(0, 10)], 10),
        MAX_ASSEMBLY_BYTES as i64,
        &["x"],
        |_| Ok(std::borrow::Cow::Borrowed(&b""[..])),
    );
    assert!(
        ok.is_err(),
        "a clean layout of that size still fails A11 first"
    );

    // And the real thing still goes through: the reference bundle's largest
    // file is 22 662 068 B, which is 47x under the limit. Checked at compile
    // time below rather than here, because a runtime `assert!` on two constants
    // is a value the optimiser already knows.
}

/// **The limit must stay clear of a real fixture.** 22 662 068 B is the largest
/// file in the reference bundle (`reuse_moved`), and 40x is slack
/// well below the 1 GiB the limit is set at. If a future fixture ever needed
/// more, this fails to compile rather than silently producing a capture that
/// cannot assemble its own game files.
const _: () = assert!(MAX_ASSEMBLY_BYTES > 22_662_068 * 40);

/// The range is checked *before* the chunk is decoded.
///
/// So a rebuild that is going to be refused does not first inflate every
/// blob to find out.
#[test]
fn a_range_that_does_not_fit_is_refused_without_decoding_it() {
    use std::borrow::Cow;
    use std::cell::Cell;
    let clean = Layout::of([(0, 10)], 10);
    let decoded = Cell::new(0_u32);
    let e = tile("L", &clean, 4, &["x"], |_| {
        decoded.set(decoded.get() + 1);
        Ok(Cow::Borrowed(&b""[..]))
    })
    .unwrap_err();
    assert!(e.contains("outside the 4 B file"), "{e}");
    assert_eq!(decoded.get(), 0, "nothing should have been decoded");
}

/// Distinct payload bytes, so a test can never pass because two chunks were
/// accidentally the same content.
fn payload(seed: u8, n: usize) -> Vec<u8> {
    (0..n)
        .map(|i| seed.wrapping_mul(31).wrapping_add(i as u8))
        .collect()
}

/// A chunk holding `payload(seed, len)` at `offset`, with a compressed size
/// derived from the content rather than invented.
fn chunk(seed: u8, offset: i64, len: usize) -> SophonChunk {
    let body = payload(seed, len);
    let raw = zstd::encode_all(body.as_slice(), 1).expect("encode");
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

/// A file from chunks, with its whole-file MD5 computed from the bytes those
/// chunks concatenate to. Deriving it is what makes A7/A8 meaningful: a
/// hand-written constant would let a broken assembly agree with itself.
fn file(path: &str, chunks: Vec<SophonChunk>) -> SophonChunkFile {
    let mut bytes = Vec::new();
    for c in &chunks {
        let at = c.offset as usize;
        if bytes.len() < at + c.uncompressed_size as usize {
            bytes.resize(at + c.uncompressed_size as usize, 0);
        }
        bytes[at..at + c.uncompressed_size as usize].copy_from_slice(&payload(
            u8::from_str_radix(&c.id[..2], 16).unwrap_or(0),
            c.uncompressed_size as usize,
        ));
    }
    SophonChunkFile {
        file: path.into(),
        chunks,
        is_folder: false,
        size: bytes.len() as i64,
        md5: hash::md5_hex(&bytes),
    }
}

/// Every chunk's decoded bytes, for a [`ChunkSource::from_memory`].
fn memory_of(entries: &[&SophonChunkFile]) -> BTreeMap<String, Vec<u8>> {
    let mut m = BTreeMap::new();
    for e in entries {
        for c in &e.chunks {
            let seed = u8::from_str_radix(&c.id[..2], 16).unwrap_or(0);
            m.insert(c.id.clone(), payload(seed, c.uncompressed_size as usize));
        }
    }
    m
}

fn sources() -> Sources {
    Sources::from_pairs([
        ("7.0.0".to_string(), "https://cdn/old".to_string()),
        ("7.1.0".to_string(), "https://cdn/new".to_string()),
    ])
}

fn assemble_pair(pair: &FilePair) -> Result<Assembled, String> {
    let entries: Vec<&SophonChunkFile> = [
        pair.old.as_ref().map(|o| &o.entry),
        pair.new.as_ref().map(|n| &n.entry),
    ]
    .into_iter()
    .flatten()
    .collect();
    let src = ChunkSource::from_memory(memory_of(&entries));
    let classes = [Class::ReuseInPlace];
    assemble(&AssemblyInput {
        pair,
        matching_field: "game",
        classes: &classes,
        sources: &sources(),
        chunks: &src,
    })
}

/// The measured shape: the file on disk is **longer** than the target and
/// no reuse source reaches past the target's end.
fn shrinking_pair() -> FilePair {
    let old = file("blk", vec![chunk(1, 0, 64), chunk(2, 64, 64)]);
    let new = file("blk", vec![chunk(1, 0, 64), chunk(9, 64, 32)]);
    FilePair {
        capture_id: "reuse_in_place".into(),
        path: "blk".into(),
        old: Some(OldFile {
            tag: "7.0.0".into(),
            build_id: "3XnPviMfoILi".into(),
            entry: old,
        }),
        new: Some(NewFile {
            tag: "7.1.0".into(),
            build_id: "sMXGW2ll3Fuu".into(),
            entry: new,
        }),
    }
}

/// The measured shape: the file on disk is longer than the target and a
/// reuse source **does** reach past the target's end.
fn shrinking_with_a_past_the_end_source_pair() -> FilePair {
    let old = file("blk", vec![chunk(1, 0, 64), chunk(2, 64, 64)]);
    let new = file("blk", vec![chunk(2, 0, 64), chunk(9, 64, 32)]);
    FilePair {
        capture_id: "reuse_moved".into(),
        path: "blk".into(),
        old: Some(OldFile {
            tag: "7.0.0".into(),
            build_id: "3XnPviMfoILi".into(),
            entry: old,
        }),
        new: Some(NewFile {
            tag: "7.1.0".into(),
            build_id: "sMXGW2ll3Fuu".into(),
            entry: new,
        }),
    }
}

/// The measured shape: three chunks, one shared at the same offset, two new.
fn same_offset_pair() -> FilePair {
    let old = file(
        "dll",
        vec![chunk(1, 0, 64), chunk(2, 64, 128), chunk(3, 192, 32)],
    );
    let new = file(
        "dll",
        vec![chunk(9, 0, 64), chunk(2, 64, 128), chunk(8, 192, 40)],
    );
    FilePair {
        capture_id: "reuse_in_place".into(),
        path: "dll".into(),
        old: Some(OldFile {
            tag: "7.0.0".into(),
            build_id: "3XnPviMfoILi".into(),
            entry: old,
        }),
        new: Some(NewFile {
            tag: "7.1.0".into(),
            build_id: "sMXGW2ll3Fuu".into(),
            entry: new,
        }),
    }
}

/// The measured shape: the same file on both sides — the skip branch.
fn identical_pair() -> FilePair {
    let old = file("dll", vec![chunk(1, 0, 64), chunk(2, 64, 128)]);
    let new = file("dll", vec![chunk(1, 0, 64), chunk(2, 64, 128)]);
    FilePair {
        capture_id: "identical".into(),
        path: "dll".into(),
        old: Some(OldFile {
            tag: "7.0.0".into(),
            build_id: "3XnPviMfoILi".into(),
            entry: old,
        }),
        new: Some(NewFile {
            tag: "7.1.0".into(),
            build_id: "sMXGW2ll3Fuu".into(),
            entry: new,
        }),
    }
}

/// The measured shape: one chunk moves **earlier** (its source is after its
/// destination) and one is new.
fn moved_pair() -> FilePair {
    let old = file(
        "blk",
        vec![chunk(1, 0, 64), chunk(2, 64, 128), chunk(3, 192, 32)],
    );
    let new = file(
        "blk",
        vec![chunk(1, 0, 64), chunk(3, 64, 32), chunk(4, 96, 128)],
    );
    FilePair {
        capture_id: "reuse_moved".into(),
        path: "blk".into(),
        old: Some(OldFile {
            tag: "7.0.0".into(),
            build_id: "3XnPviMfoILi".into(),
            entry: old,
        }),
        new: Some(NewFile {
            tag: "7.1.0".into(),
            build_id: "sMXGW2ll3Fuu".into(),
            entry: new,
        }),
    }
}

// ------------------------------------------------- the three assemblies

/// A7 and A8: both direct assemblies hash to what the manifest declared.
#[test]
fn both_direct_assemblies_reproduce_their_declared_md5() {
    let a = assemble_pair(&same_offset_pair()).expect("assembles");
    let o = a.old.as_ref().expect("an old side");
    let n = a.new.as_ref().expect("a new side");
    assert!(o.md5_matches(), "A7: {} vs {}", o.md5, o.declared_md5);
    assert!(n.md5_matches(), "A8: {} vs {}", n.md5, n.declared_md5);
    assert_eq!(o.nchunks, 3);
    assert_eq!(o.layout.total, 224, "64 + 128 + 32");
    assert!(o.layout.is_clean());
}

#[test]
fn a9_is_red_when_the_patch_flow_and_the_direct_assembly_disagree() {
    // **The negative case, which the suite did not have.** Every A9 test here
    // asserts the *agreeing* case, so nothing checked that a disagreement is
    // visible at all.
    //
    // **What this pins, precisely:** that the disagreement reaches the artifact
    // as a red A9 row and a non-green run. It does **not** pin that the
    // comparison is by bytes — see the note below, which is why that could not
    // be done here.
    let good = assemble_pair(&same_offset_pair()).expect("assembles");
    assert!(
        good.all_green(),
        "the honest pair is green: {:?}",
        good.selfcheck_rows()
    );

    let mut bad = assemble_pair(&same_offset_pair()).expect("assembles");
    // One byte different on the direct side, so its digest moves and stops
    // matching the patch side's: exactly what a patch flow that assembled
    // from stale offsets would produce. The patch side keeps the digest it was
    // assembled with — giving it the *new* side's digest would be the bug.
    let n = bad.new.as_mut().expect("a new side");
    n.bytes[0] ^= 0xff;
    n.md5 = hash::md5_hex(&n.bytes);
    let p = bad.patch.as_mut().expect("a patch");
    assert_eq!(
        p.md5,
        good.new.as_ref().unwrap().md5,
        "before the disagreement both sides agreed"
    );
    p.equals_new = false;
    assert_ne!(p.md5, n.md5, "and now they must not");

    let a9: Vec<_> = bad
        .selfcheck_rows()
        .into_iter()
        .filter(|r| r.id == "A9")
        .collect();
    assert_eq!(a9.len(), 1, "there is exactly one A9 row: {a9:?}");
    assert!(
        !a9[0].ok,
        "a patch flow that disagrees with the direct assembly must not pass: {:?}",
        a9[0]
    );
    assert!(!bad.all_green(), "and the run must not be green");
}

/// A9 — the one that matters. The patch flow is a completely different code
/// path from the direct assembly, so byte-equality is evidence rather than a
/// tautology.
#[test]
fn the_patch_flow_byte_equals_the_direct_new_assembly() {
    let a = assemble_pair(&same_offset_pair()).expect("assembles");
    let p = a.patch.as_ref().expect("a patch");
    let n = a.new.as_ref().expect("a new side");
    assert!(
        p.equals_new,
        "the patch flow and the direct assembly disagree"
    );
    // `equals_new` **is** the byte comparison, settled inside `assemble` while
    // both sides were alive and then dropped. A digest comparison would not
    // be — A9 is specified as byte equality — so `md5` agreeing is
    // corroboration, never the claim.
    assert_eq!(p.md5, n.md5, "and the recorded digests agree too");
    assert!(a.all_green(), "{:?}", a.selfcheck_rows());
}

#[test]
fn the_shared_chunk_is_reused_and_verified_by_md5() {
    let a = assemble_pair(&same_offset_pair()).expect("assembles");
    let p = a.patch.as_ref().unwrap();
    assert_eq!(p.reused, 1);
    assert_eq!(p.fetched, 2);
    assert_eq!(p.moved, 0, "the reused chunk is at the same offset");
    assert!(p.a10_holds());
    assert!(p.rewrites.is_empty(), "{:?}", p.rewrites);
    let plan = a.map.plan.as_ref().unwrap();
    let reused = plan.iter().find(|e| e.decision == Decision::Reuse).unwrap();
    let s = reused.source_range.as_ref().expect("a source range");
    assert_eq!(s.file_tag, "7.0.0");
    assert_eq!(s.start, 64);
    assert_eq!(s.end, 192);
    assert_eq!(s.start, reused.target_range.start, "same offset");
    assert!(reused.url.is_none(), "a REUSE has nothing to download");
    assert!(reused.note.as_deref().unwrap().contains("same offset"));
}

#[test]
fn every_fetch_entry_names_the_url_it_would_download() {
    let a = assemble_pair(&same_offset_pair()).expect("assembles");
    let plan = a.map.plan.as_ref().unwrap();
    for e in plan.iter().filter(|e| e.decision == Decision::Fetch) {
        let u = e
            .url
            .as_deref()
            .unwrap_or_else(|| panic!("entry {} has no url", e.i));
        assert!(u.starts_with("https://cdn/new/"), "{u}");
        assert!(u.ends_with(&e.chunk_id), "{u} does not name the chunk");
        assert!(e.source_range.is_none(), "a FETCH has no local source");
    }
}

/// The moved case: the same bytes at a different offset in the new file. The
/// patch flow has to read them from where they *are*, not where they land.
#[test]
fn a_moved_chunk_is_reused_from_its_old_offset_and_counted_as_moved() {
    let old = file(
        "blk",
        vec![chunk(1, 0, 64), chunk(2, 64, 128), chunk(3, 192, 32)],
    );
    // chunk 1 stays at 0, chunk 3 moves 192 -> 64, chunk 4 is brand new.
    let new = file(
        "blk",
        vec![chunk(1, 0, 64), chunk(3, 64, 32), chunk(4, 96, 128)],
    );
    let a = assemble_pair(&FilePair {
        capture_id: "reuse_moved".into(),
        path: "blk".into(),
        old: Some(OldFile {
            tag: "7.0.0".into(),
            build_id: "3XnPviMfoILi".into(),
            entry: old,
        }),
        new: Some(NewFile {
            tag: "7.1.0".into(),
            build_id: "sMXGW2ll3Fuu".into(),
            entry: new,
        }),
    })
    .expect("assembles");
    let p = a.patch.as_ref().unwrap();
    assert!(
        p.equals_new,
        "the patch flow and the direct assembly disagree"
    );
    assert_eq!(p.reused, 2, "chunk 1 stays and chunk 3 moves");
    assert_eq!(p.fetched, 1, "chunk 4 is new content");
    assert_eq!(p.moved, 1, "only chunk 3 changed offset");
    let plan = a.map.plan.as_ref().unwrap();
    let moved = plan.iter().find(|e| e.is_moved()).expect("a moved entry");
    assert_eq!(moved.target_range.start, 64);
    assert_eq!(
        moved.source_range.as_ref().unwrap().start,
        192,
        "read from where it is"
    );
    assert!(moved.note.as_deref().unwrap().contains("moved"));
    assert_eq!(a.map.totals.as_ref().unwrap().moved, 1);
}

/// **"Moved" is one fact.** The diff's `moved` and the patchmap's own
/// `is_moved()` have to be the same number for the same path, or a class is
/// a statement about a different thing than the patchmap's plan.
///
/// **"Moved" is one fact.** The diff counts a chunk as moved when the new
/// offset is not among *any* of the old file's offsets for that identity;
/// the patch flow takes the **first** old chunk with that identity as the
/// source. When an identity occurs twice in the old file the two answers
/// are different numbers — and `girpr` reads the *first* match
/// (`repair::reuse::sources_for`), so the first match is the rule that has
/// to win.
#[test]
fn the_diffs_moved_count_and_the_patchmaps_own_are_the_same_number() {
    // The pair where they disagree: the old file holds the same content
    // **twice**, at 0 and at 64. The new file wants it at 64 — an offset the
    // old file *does* have — so counting any old offset as a match calls it
    // "not moved", while reading the first source calls it moved.
    let duplicated = FilePair {
        capture_id: "reuse_in_place".into(),
        path: "blk".into(),
        old: Some(OldFile {
            tag: "7.0.0".into(),
            build_id: "3XnPviMfoILi".into(),
            entry: file("blk", vec![chunk(1, 0, 64), chunk(1, 64, 64)]),
        }),
        new: Some(NewFile {
            tag: "7.1.0".into(),
            build_id: "sMXGW2ll3Fuu".into(),
            entry: file("blk", vec![chunk(2, 0, 64), chunk(1, 64, 64)]),
        }),
    };
    // The moved case, for the other side of the comparison.
    let moved = FilePair {
        capture_id: "reuse_moved".into(),
        path: "blk".into(),
        old: Some(OldFile {
            tag: "7.0.0".into(),
            build_id: "3XnPviMfoILi".into(),
            entry: file(
                "blk",
                vec![chunk(1, 0, 64), chunk(2, 64, 128), chunk(3, 192, 32)],
            ),
        }),
        new: Some(NewFile {
            tag: "7.1.0".into(),
            build_id: "sMXGW2ll3Fuu".into(),
            entry: file(
                "blk",
                vec![chunk(1, 0, 64), chunk(3, 64, 32), chunk(4, 96, 128)],
            ),
        }),
    };

    for pair in [same_offset_pair(), duplicated.clone(), moved] {
        let a = assemble_pair(&pair).expect("assembles");
        let plan = a.map.plan.as_ref().expect("a plan");
        let plan_moved = plan.iter().filter(|e| e.is_moved()).count() as u64;
        let diff = crate::manifest::diff::score_pair(
            &pair.old.as_ref().expect("an old side").entry,
            &pair.new.as_ref().expect("a new side").entry,
        );
        assert_eq!(
            diff.moved, plan_moved,
            "{}: the diff and the patchmap disagree about what moved",
            pair.capture_id
        );
        assert_eq!(
            diff.moved,
            diff.moved_later + diff.moved_earlier,
            "{}: every move lands either earlier or later",
            pair.capture_id
        );
    }

    // And the duplicated pair is the one they disagree on: the diff says
    // the chunk moved, because the source the repair would read is at 0
    // while the chunk lands at 64.
    let a = assemble_pair(&duplicated).expect("assembles");
    let plan = a.map.plan.as_ref().expect("a plan");
    let moved_entry = plan.iter().find(|e| e.is_moved()).expect("a moved entry");
    assert_eq!(moved_entry.source_range.as_ref().unwrap().start, 0);
    assert_eq!(moved_entry.target_range.start, 64);
    let diff = crate::manifest::diff::score_pair(
        &duplicated.old.as_ref().unwrap().entry,
        &duplicated.new.as_ref().unwrap().entry,
    );
    assert_eq!(diff.moved, 1, "the first match is the source");
    assert_eq!(diff.moved_later, 1, "and the bytes move later");
    assert_eq!(diff.moved_earlier, 0);
}

/// Every chunk reused and the file laid out differently is the reorder case:
/// nothing is fetched at all, and A9 still has to hold.
#[test]
fn a_pure_reorder_fetches_nothing_and_still_equals_the_new_assembly() {
    let old = file(
        "blk",
        vec![chunk(1, 0, 64), chunk(2, 64, 128), chunk(3, 192, 32)],
    );
    let new = file(
        "blk",
        vec![chunk(3, 0, 32), chunk(1, 32, 64), chunk(2, 96, 128)],
    );
    let a = assemble_pair(&FilePair {
        capture_id: "reuse_moved".into(),
        path: "blk".into(),
        old: Some(OldFile {
            tag: "7.0.0".into(),
            build_id: "3XnPviMfoILi".into(),
            entry: old,
        }),
        new: Some(NewFile {
            tag: "7.1.0".into(),
            build_id: "sMXGW2ll3Fuu".into(),
            entry: new,
        }),
    })
    .expect("assembles");
    let p = a.patch.as_ref().unwrap();
    assert_eq!(p.fetched, 0, "nothing new: every chunk is in the old file");
    assert_eq!(p.reused, 3);
    assert_eq!(
        p.moved, 3,
        "every chunk changed offset: that is what a reorder is"
    );
    assert!(
        p.equals_new,
        "the patch flow and the direct assembly disagree"
    );
    assert_eq!(a.map.totals.as_ref().unwrap().download_bytes, 0);
    assert_eq!(a.map.expectations.must_not_fetch.len(), 3);
    assert!(a.all_green(), "{:?}", a.selfcheck_rows());
}

// ------------------------------------------------------ one-sided pairs

#[test]
fn an_added_file_has_no_old_side_and_nothing_to_reuse() {
    let new = file("new.dll", vec![chunk(4, 0, 64)]);
    let a = assemble_pair(&FilePair {
        capture_id: "added".into(),
        path: "new.dll".into(),
        old: None,
        new: Some(NewFile {
            tag: "7.1.0".into(),
            build_id: "sMXGW2ll3Fuu".into(),
            entry: new,
        }),
    })
    .expect("assembles");
    assert!(a.old.is_none());
    assert!(a.patch.is_none(), "there is nothing to patch from");
    assert!(a.new.as_ref().unwrap().md5_matches());
    assert!(a.map.old.is_none());
    // There is no patch, but there *is* a plan: every new-side chunk must be
    // downloaded whole. Emitting `plan: null` here is what made
    // `expectations.download_bytes` a defaulted `0` - a number a consumer
    // would compare against and lose to, for a file that is not free.
    let plan = a.map.plan.as_ref().expect("a plan: every chunk is fetched");
    assert_eq!(plan.len(), a.new.as_ref().unwrap().nchunks);
    for e in plan {
        assert_eq!(e.decision, Decision::Fetch);
        assert!(e.source_range.is_none(), "no old side to slice from");
        assert!(
            e.note.is_some(),
            "so a reader is told why, not left guessing"
        );
    }
    let t = a.map.totals.as_ref().expect("the cost of the plan");
    assert_eq!(t.reuse, 0);
    assert_eq!(t.fetch, plan.len());
    assert_eq!(t.moved, 0);
    assert_eq!(
        t.download_bytes,
        plan.iter().map(|e| e.compressed_size).sum::<i64>(),
        "the whole file is downloaded, so the patch costs what it weighs"
    );
    assert!(t.download_bytes > 0, "not free");
    assert_eq!(a.map.expectations.download_bytes, t.download_bytes);
    assert!(!a.map.expectations.absent_after);
    assert!(a.map.expectations.must_not_fetch.is_empty());
    assert!(a.a12_holds(), "vacuously");
}

#[test]
fn a_removed_file_has_a_new_side_nothing_to_patch_to() {
    let old = file("gone.blk", vec![chunk(5, 0, 32)]);
    let a = assemble_pair(&FilePair {
        capture_id: "removed".into(),
        path: "gone.blk".into(),
        old: Some(OldFile {
            tag: "7.0.0".into(),
            build_id: "3XnPviMfoILi".into(),
            entry: old,
        }),
        new: None,
    })
    .expect("assembles");
    assert!(a.new.is_none());
    assert!(a.patch.is_none());
    assert!(a.old.as_ref().unwrap().md5_matches());
    assert!(a.map.plan.is_none(), "no new side, so no target");
    assert!(a.map.totals.is_none(), "and so nothing to cost");
    // Expectations are **not** absent. A consumer needs something to assert,
    // and for a removal the assertion is that the file is gone and that
    // nothing was downloaded to achieve it.
    let e = &a.map.expectations;
    assert!(e.absent_after, "the outcome is deletion");
    assert_eq!(e.result_path, None, "there is nothing to rebuild");
    assert_eq!(e.result_md5, None);
    assert_eq!(e.download_bytes, 0, "a true zero: deleting costs nothing");
    assert!(e.must_not_fetch.is_empty());
}

// ------------------------------------------------------------ the totals

#[test]
fn the_totals_are_the_arithmetic_a_reader_would_do_by_hand() {
    let pair = same_offset_pair();
    let a = assemble_pair(&pair).expect("assembles");
    let t = a.map.totals.as_ref().unwrap();
    let plan = a.map.plan.as_ref().unwrap();
    assert_eq!(t.chunks, 3);
    assert_eq!(t.reuse, 1);
    assert_eq!(t.fetch, 2);
    assert_eq!(t.moved, 0);
    assert_eq!(t.materialize_old_bytes, 224);
    assert_eq!(t.materialize_new_bytes, 232, "64 + 128 + 40");
    // download_bytes is the sum over FETCH entries only, which is the whole
    // point: it is what the patch costs, not what the new file weighs.
    let expect: i64 = plan
        .iter()
        .filter(|e| e.decision == Decision::Fetch)
        .map(|e| e.compressed_size)
        .sum();
    assert_eq!(t.download_bytes, expect);
    assert!(t.download_bytes < t.materialize_new_bytes);
    let e = &a.map.expectations;
    assert_eq!(e.download_bytes, t.download_bytes);
    assert_eq!(e.result_path.as_deref(), Some("expected/dll"));
    assert_eq!(
        e.result_md5.as_deref(),
        Some(a.new.as_ref().unwrap().declared_md5.as_str())
    );
    assert!(!e.absent_after, "a two-sided capture leaves a file behind");
    assert_eq!(e.must_not_fetch.len(), 1, "the one reused chunk");
}

// -------------------------------------------------------------- failures

/// A manifest whose chunks do not tile the file is refused with A11 named,
/// rather than assembled into something that might hash correctly by luck.
#[test]
fn a_layout_that_violates_a11_stops_the_assembly_and_says_which_rule() {
    let mut old = file("bad.blk", vec![chunk(1, 0, 64), chunk(2, 64, 128)]);
    // Punch a hole: the second chunk now starts at 100 instead of 64.
    old.chunks[1].offset = 100;
    old.size = 228;
    let e = assemble_pair(&FilePair {
        capture_id: "x".into(),
        path: "bad.blk".into(),
        old: Some(OldFile {
            tag: "7.0.0".into(),
            build_id: "3XnPviMfoILi".into(),
            entry: old,
        }),
        new: None,
    })
    .unwrap_err();
    assert!(e.contains("A11"), "{e}");
    assert!(e.contains("gap of 36 B"), "{e}");
}

/// A10: if the old file's slice does not hash to what the new chunk declares,
/// the flow must FETCH instead — and record that it tried.
///
/// This is hard to reach with a well-formed pair, because a REUSE candidate is
/// found *by* content identity. It is reached by giving the new chunk the old
/// chunk's identity while the assembled old bytes are not what that identity
/// says — which is exactly what a corrupt bundle looks like.
#[test]
fn a_reuse_whose_slice_does_not_verify_is_downgraded_to_fetch_and_recorded() {
    let pair = same_offset_pair();
    // Corrupt the old file's assembly so its bytes no longer match the
    // identity the new manifest reuses, without touching the identity itself.
    let shared = pair.new.as_ref().unwrap().entry.chunks[1].clone();
    let entries: Vec<&SophonChunkFile> = [
        &pair.old.as_ref().unwrap().entry,
        &pair.new.as_ref().unwrap().entry,
    ]
    .into();
    let mut mem = memory_of(&entries);
    mem.insert(shared.id.clone(), payload(200, 128)); // wrong content, same id
    let classes = [Class::ReuseInPlace];
    let src = ChunkSource::from_memory(mem);
    let mut a = assemble(&AssemblyInput {
        pair: &pair,
        matching_field: "game",
        classes: &classes,
        sources: &sources(),
        chunks: &src,
    })
    .expect("assembles");

    // The old side is now wrong, so A7 fails and the patch had to fetch.
    assert!(
        !a.old.as_ref().unwrap().md5_matches(),
        "A7 fails by construction"
    );
    let p = a.patch.as_ref().unwrap();
    assert!(!p.a10_holds(), "the REUSE was attempted and rejected");
    assert_eq!(p.reuse_failures.len(), 1, "{:?}", p.reuse_failures);
    assert!(p.rewrites.len() == 1, "and the downgrade is recorded");
    assert!(
        p.reuse_failures[0].contains("declares"),
        "names what was expected: {:?}",
        p.reuse_failures[0]
    );
    // The plan entry says FETCH and carries the reason, so a reader can tell
    // "never a candidate" from "tried and failed".
    let e = a
        .map
        .plan
        .as_ref()
        .unwrap()
        .iter()
        .find(|e| e.chunk_id == shared.id)
        .unwrap();
    assert_eq!(e.decision, Decision::Fetch);
    assert!(
        e.note
            .as_deref()
            .unwrap()
            .contains("REUSE attempted and rejected")
    );
    assert!(e.url.is_some(), "so it can be downloaded after all");
    // And A9 still holds: the flow fetched the right bytes and produced the
    // new file regardless of the failed REUSE.
    assert!(
        p.equals_new,
        "the patch flow and the direct assembly disagree"
    );
    assert_eq!(p.md5, a.new.as_ref().unwrap().md5);
    let _ = &mut a;
}

/// A missing chunk is a message naming the id, not a panic and not a
/// silently zero-filled file.
#[test]
fn a_missing_chunk_is_named_and_stops_the_assembly() {
    let pair = same_offset_pair();
    let src = ChunkSource::from_memory(BTreeMap::new());
    let classes = [Class::ReuseInPlace];
    let e = assemble(&AssemblyInput {
        pair: &pair,
        matching_field: "game",
        classes: &classes,
        sources: &sources(),
        chunks: &src,
    })
    .unwrap_err();
    let id = &pair.old.as_ref().unwrap().entry.chunks[0].id;
    assert!(e.contains(id.as_str()), "{e}");
}

// -------------------------------------------------------------- the rows

#[test]
fn a_green_capture_produces_one_row_per_assertion_and_no_red_ones() {
    let a = assemble_pair(&same_offset_pair()).expect("assembles");
    let rows = a.selfcheck_rows();
    let kinds: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        kinds.iter().filter(|k| **k == "A7").count(),
        1,
        "one row per assertion instance, not per assertion kind: {kinds:?}"
    );
    assert!(
        kinds.contains(&"A8")
            && kinds.contains(&"A9")
            && kinds.contains(&"A10")
            && kinds.contains(&"A11")
            && kinds.contains(&"A12")
    );
    assert!(a.all_green());
    assert!(
        rows.iter().all(|r| r.ok && !r.actual.is_empty()),
        "every row carries a detail a reader can check: {rows:?}"
    );
}

/// The **skip branch**: the file is already right, so the patch fetches
/// nothing and writes nothing.
#[test]
fn the_skip_branch_fetches_nothing() {
    let a = assemble_pair(&identical_pair()).expect("assembles");
    let plan = a.map.plan.as_ref().expect("a plan");
    assert!(
        plan.iter().all(|e| e.decision == Decision::Reuse),
        "every chunk is already in the file"
    );
    let totals = a.map.totals.expect("totals");
    assert_eq!(totals.fetch, 0);
    assert_eq!(totals.reuse, plan.len());
    assert_eq!(a.map.expectations.download_bytes, 0);
}

/// The patch's shape and the plan it was derived from must agree, or the
/// artifact says two things about the same file.
#[test]
fn the_patch_shape_agrees_with_the_plan_it_came_from() {
    let a = assemble_pair(&same_offset_pair()).expect("assembles");
    let s = crate::assemble::patch_shape(&a.map);
    let plan = a.map.plan.as_ref().expect("a plan");
    let reuse = plan
        .iter()
        .filter(|e| e.decision == Decision::Reuse)
        .count() as u64;
    let fetch = plan
        .iter()
        .filter(|e| e.decision == Decision::Fetch)
        .count() as u64;
    let p = s.plan.expect("a plan");
    assert_eq!(p.chunks, plan.len() as u64);
    assert_eq!(p.reuse, reuse);
    assert_eq!(p.chunks - p.reuse, fetch);
    assert_eq!(p.moved, 0, "nothing moves");
    assert_eq!(
        s.old_size,
        Some(a.map.old.as_ref().expect("an old side").size as u64)
    );
    assert_eq!(
        s.new_size,
        Some(a.map.new.as_ref().expect("a new side").size as u64)
    );
}

/// **An absent side is no file, not a file of zero bytes.**
///
/// The difference is what makes "a file that grows" and "a file that
/// shrinks" true of a zero-byte file: a file that appears has appeared, and a
/// file that disappears has gone. Counting an absent side as zero bytes
/// would make a zero-byte `added` file neither grow nor shrink, and the
/// bundle's coverage of the size boundary would depend on whether the
/// cheapest candidate happened to be empty.
#[test]
fn an_absent_side_is_no_file_and_not_a_file_of_zero_bytes() {
    // A file that is created: no old side, and the new side is empty.
    let created = crate::assemble::PatchShape {
        plan: Some(crate::assemble::PlanCounts {
            chunks: 0,
            reuse: 0,
            moved: 0,
        }),
        old_size: None,
        new_size: Some(0),
    };
    assert!(created.grows(), "a file that appears has grown");
    assert!(!created.shrinks());

    // A file that is deleted: the old side is empty, and there is no new
    // side.
    let deleted = crate::assemble::PatchShape {
        plan: None,
        old_size: Some(0),
        new_size: None,
    };
    assert!(deleted.shrinks(), "a file that disappears has shrunk");
    assert!(!deleted.grows());

    // A file that is present on both sides and empty on both: it neither
    // grows nor shrinks.
    let empty = crate::assemble::PatchShape {
        plan: Some(crate::assemble::PlanCounts {
            chunks: 0,
            reuse: 0,
            moved: 0,
        }),
        old_size: Some(0),
        new_size: Some(0),
    };
    assert!(!empty.grows() && !empty.shrinks());

    // A file that is present on both sides and grows in bytes.
    let bigger = crate::assemble::PatchShape {
        plan: Some(crate::assemble::PlanCounts {
            chunks: 1,
            reuse: 0,
            moved: 0,
        }),
        old_size: Some(0),
        new_size: Some(100),
    };
    assert!(bigger.grows());
    assert!(!bigger.shrinks());
}

/// The negative expectation is the plan's REUSE set, stated as the ids a
/// consumer must **not** see requested.
#[test]
fn the_negative_expectation_is_the_plans_reuse_set() {
    let a = assemble_pair(&moved_pair()).expect("assembles");
    let plan = a.map.plan.as_ref().expect("a plan");
    let reuse: Vec<&str> = plan
        .iter()
        .filter(|e| e.decision == Decision::Reuse)
        .map(|e| e.chunk_id.as_str())
        .collect();
    assert_eq!(a.map.expectations.must_not_fetch, reuse);
}

/// A patchmap says what the **patch** is, and nothing about how a repair
/// tool gets there: no tier, no write order, no trim, and no `SUMMARY`
/// line transcribed into a foreign schema.
#[test]
fn the_patchmap_carries_no_repair_tool_vocabulary() {
    let a = assemble_pair(&moved_pair()).expect("assembles");
    let v = serde_json::to_value(&a.map).expect("serializes");
    let obj = v.as_object().expect("an object");
    assert!(
        !obj.contains_key("repair"),
        "the repair block is gone: {obj:?}"
    );
    let ex = obj["expectations"].as_object().expect("expectations");
    assert!(
        !ex.contains_key("summary"),
        "the SUMMARY expectations are gone: {ex:?}"
    );
}

/// The patch's shape says whether the file grows or shrinks, how much of
/// it is reused and how much of that has moved — the three facts the
/// coverage assertions read.
#[test]
fn the_patch_shape_says_whether_the_file_grows_or_shrinks() {
    let growing = assemble_pair(&same_offset_pair()).expect("assembles");
    let s = crate::assemble::patch_shape(&growing.map);
    assert!(s.grows(), "the file grows");
    assert_eq!(s.plan.expect("a plan").moved, 0, "and nothing moves");

    let shrinking = assemble_pair(&shrinking_pair()).expect("assembles");
    let s = crate::assemble::patch_shape(&shrinking.map);
    assert!(s.shrinks(), "the file shrinks");

    let moved = assemble_pair(&moved_pair()).expect("assembles");
    let s = crate::assemble::patch_shape(&moved.map);
    let p = s.plan.expect("a plan");
    assert!(p.moved > 0, "the bytes move");
    assert!(p.reuse > 0, "and some of them are reused");

    let moved_shrinking =
        assemble_pair(&shrinking_with_a_past_the_end_source_pair()).expect("assembles");
    let s = crate::assemble::patch_shape(&moved_shrinking.map);
    assert!(s.plan.expect("a plan").moved > 0, "the bytes move");
    assert!(s.shrinks(), "and the file shrinks");
}

#[test]
fn decisions_serialise_the_way_the_schema_doc_writes_them() {
    assert_eq!(
        serde_json::to_string(&Decision::Reuse).unwrap(),
        "\"REUSE\""
    );
    assert_eq!(
        serde_json::to_string(&Decision::Fetch).unwrap(),
        "\"FETCH\""
    );
    assert_eq!(Decision::Fetch.to_string(), "FETCH");
}

#[test]
fn the_map_carries_the_field_names_the_schema_doc_lists() {
    let a = assemble_pair(&same_offset_pair()).expect("assembles");
    let v = serde_json::to_value(&a.map).expect("serialise");
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "capture_id",
            "classes",
            "expectations",
            "matching_field",
            "new",
            "old",
            "path",
            "plan",
            "schema_version",
            "totals",
        ],
        "docs/01-artifact-schema.md §13"
    );
    let entry = &v["plan"][0];
    let mut ek: Vec<&str> = entry
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    ek.sort_unstable();
    assert_eq!(
        ek,
        vec![
            "chunk_id",
            "compressed_size",
            "decision",
            "i",
            "note",
            "source_range",
            "target_range",
            "uncompressed_md5",
            "uncompressed_size",
            "url",
        ]
    );
    assert_eq!(v["plan"][0]["target_range"]["start"], 0);
    assert_eq!(v["plan"][0]["target_range"]["end"], 64);
}

/// **K1.** A plan cost is an address-free total, but it still has to be a
/// count: a negative `compressed_size` is refused with the chunk and value
/// rather than recorded as a negative download.
#[test]
fn a_negative_compressed_size_is_a_typed_error_not_a_negative_total() {
    let entry = PlanEntry {
        i: 0,
        decision: Decision::Fetch,
        target_range: Range::at(0, 8),
        source_range: None,
        chunk_id: "chunk-a".into(),
        url: Some("https://cdn/chunk-a".into()),
        compressed_size: -1,
        uncompressed_size: 8,
        uncompressed_md5: "00".repeat(16),
        note: None,
    };
    let e = totals_for(&[entry], None, None).unwrap_err();
    assert!(e.contains("chunk-a"), "names the chunk: {e}");
    assert!(e.contains("-1"), "names the value: {e}");
    assert!(e.contains("negative"), "{e}");
}

/// **K1.** `Σ compressed_size` over a hostile plan saturates into a typed
/// error naming the running total, not a debug abort or a release wrap.
#[test]
fn a_plan_cost_that_overflows_is_a_typed_error() {
    let entry = |i: usize, id: &str| PlanEntry {
        i,
        decision: Decision::Fetch,
        target_range: Range::at(0, 8),
        source_range: None,
        chunk_id: id.into(),
        url: None,
        compressed_size: i64::MAX,
        uncompressed_size: 8,
        uncompressed_md5: "00".repeat(16),
        note: None,
    };
    let e = totals_for(&[entry(0, "first"), entry(1, "second")], None, None).unwrap_err();
    assert!(e.contains("second"), "names the term that overflowed: {e}");
    assert!(
        e.contains("overflows") && e.contains(&i64::MAX.to_string()),
        "names the arithmetic and the value: {e}"
    );
}

/// **K1.** `assemble` revalidates a hand-built manifest, so the same crafted
/// `compressed_size` cannot bypass the capture's decode-boundary check.
#[test]
fn assemble_refuses_a_manifest_with_a_negative_cost() {
    let mut bad = chunk(1, 0, 8);
    bad.compressed_size = -1;
    let pair = FilePair {
        capture_id: "negative_cost".into(),
        path: "dll".into(),
        old: None,
        new: Some(NewFile {
            tag: "7.1.0".into(),
            build_id: "sMXGW2ll3Fuu".into(),
            entry: file("dll", vec![bad]),
        }),
    };
    let e = assemble_pair(&pair).unwrap_err();
    assert!(e.contains("dll"), "names the entry: {e}");
    assert!(
        e.contains("compressed_size -1") && e.contains("negative"),
        "names the field and the value: {e}"
    );
}
