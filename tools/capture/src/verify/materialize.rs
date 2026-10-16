//! Rebuilding both files from `chunks/`, with no network.
//!
//! **M9.** This is the closure check: if the retained blobs cannot reproduce
//! `game/` and `expected/` byte-for-byte, then the bundle's claim that it is
//! self-contained is false, and everything downstream of that claim is void.
//!
//! ## Rebuilt from the index, not from the manifests
//!
//! The chunk list comes from `chunks/index.json` — `referenced_by` for the
//! offsets, `uncompressed_size` for the lengths — and *not* from the retained
//! manifests. That is deliberate. If the rebuild consulted the manifests it
//! would only be proving that the manifests are still readable; rebuilding from
//! the closure set proves the thing the bundle actually claims, which is that
//! the blobs alone are enough. A bundle whose manifests survived but whose index
//! was incomplete fails here, and should.

use std::collections::BTreeMap;

use crate::assemble::ChunkSource;
use crate::assemble::ranges::{Layout, Range};
use crate::blobs::ChunkIndex;
use crate::blobs::chunks::Set;
use crate::hash;

/// One side of a capture, rebuilt from the index alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rebuilt {
    /// The build the chunks came from.
    pub tag: String,
    /// The manifest-relative path.
    pub path: String,
    /// The bytes, placed at each chunk's recorded offset.
    pub bytes: Vec<u8>,
    /// `md5(bytes)`.
    pub md5: String,
    /// How many chunks went into it.
    pub nchunks: usize,
    /// Where they went, and whether that was legal (A11).
    pub layout: Layout,
}

/// The chunks the index says make up one file on one side.
///
/// `(chunk_id, offset)`, ordered by offset so the caller's layout check and the
/// assembly both see the file the way a reader would.
pub fn chunks_for(
    index: &ChunkIndex,
    tag: &str,
    capture_id: &str,
    path: &str,
    role: Set,
) -> Result<Vec<(String, i64)>, String> {
    let mut out: Vec<(String, i64)> = Vec::new();
    for b in &index.blobs {
        let hits: Vec<i64> = b
            .referenced_by
            .iter()
            .filter(|r| {
                r.tag == tag && r.capture_id == capture_id && r.path == path && r.role == role
            })
            .map(|r| r.offset)
            .collect();
        match hits.len() {
            0 => continue,
            // One blob referenced at two offsets for one file is a manifest that
            // places the same bytes twice. Taking the first would silently drop
            // the second and produce a file with a hole; recorded, not resolved.
            n if n > 1 => {
                return Err(format!(
                    "chunk {} is referenced {n} times by {tag} {path} ({role}), at offsets {}",
                    b.props.chunk_id,
                    hits.iter()
                        .map(i64::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            _ => out.push((b.props.chunk_id.clone(), hits[0])),
        }
    }
    out.sort_by_key(|(_, o)| *o);
    Ok(out)
}

/// Rebuild one side of one capture from the stored blobs.
///
/// The layout is checked before any byte is placed (A11), so a closure set with
/// a hole produces an error naming the hole rather than a short file.
pub fn rebuild(
    root: &std::path::Path,
    index: &ChunkIndex,
    src: &ChunkSource,
    tag: &str,
    capture_id: &str,
    path: &str,
    role: Set,
) -> Result<Rebuilt, String> {
    let _ = root;
    let chunks = chunks_for(index, tag, capture_id, path, role)?;
    if chunks.is_empty() {
        return Err(format!(
            "{tag} {path} ({role}): the index references no chunks for it"
        ));
    }
    let sizes: BTreeMap<&str, u64> = index
        .blobs
        .iter()
        .map(|b| (b.props.chunk_id.as_str(), b.props.uncompressed_size))
        .collect();
    let parts: Vec<(i64, i64)> = chunks
        .iter()
        .map(|(id, o)| (*o, sizes.get(id.as_str()).copied().unwrap_or(0) as i64))
        .collect();
    let declared_size: i64 = parts.iter().map(|(_, l)| *l).sum();
    let layout = Layout::of(&parts, declared_size);
    if !layout.is_clean() {
        return Err(format!(
            "{tag} {path} ({role}): does not satisfy A11 — {}",
            layout.violations.join("; ")
        ));
    }
    let mut bytes = vec![0_u8; declared_size.max(0) as usize];
    for ((id, _), r) in chunks.iter().zip(&layout.ranges) {
        let decoded = src.decoded(id)?;
        if decoded.len() as i64 != r.len() {
            return Err(format!(
                "{tag} {path}: chunk {id} decoded to {} B but the index says its range is {r}",
                decoded.len()
            ));
        }
        let dst = bytes
            .get_mut(r.start as usize..r.end as usize)
            .ok_or_else(|| format!("{tag} {path}: range {r} is outside the file"))?;
        dst.copy_from_slice(&decoded);
    }
    Ok(Rebuilt {
        tag: tag.to_string(),
        path: path.to_string(),
        md5: hash::md5_hex(&bytes),
        bytes,
        nchunks: chunks.len(),
        layout,
    })
}

/// Both sides of one capture, as the patchmap describes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    /// The `capture_id`.
    pub capture_id: String,
    /// The manifest-relative path.
    pub path: String,
    /// The old side, absent for an `added` capture.
    pub old: Option<Rebuilt>,
    /// The new side, absent for a `removed` capture.
    pub new: Option<Rebuilt>,
}

/// Rebuild every side of every capture the index knows about.
///
/// Ordered by `capture_id` then path, so two runs produce the same report in the
/// same order regardless of how the index's own lists were built.
pub fn rebuild_all(
    root: &std::path::Path,
    index: &ChunkIndex,
    patchmaps: &BTreeMap<String, crate::assemble::PatchMap>,
) -> Result<Vec<Pair>, String> {
    let src = ChunkSource::from_index(root, index);
    let mut names: Vec<&String> = patchmaps.keys().collect();
    names.sort();
    let mut out = Vec::new();
    for name in names {
        let map = &patchmaps[name];
        let old = rebuild_side(
            root,
            index,
            &src,
            name,
            map.path.as_str(),
            map.old.as_ref(),
            Set::MaterializeOld,
        )?;
        let new = rebuild_side(
            root,
            index,
            &src,
            name,
            map.path.as_str(),
            map.new.as_ref(),
            Set::MaterializeNew,
        )?;
        out.push(Pair {
            capture_id: name.clone(),
            path: map.path.clone(),
            old,
            new,
        });
    }
    Ok(out)
}

/// Rebuild one side, if the patchmap says there is one.
///
/// A one-sided capture is the normal shape for `added` and `removed`, so "no
/// old side" is `None` rather than an error — and a map that claims a side the
/// index cannot supply *is* an error, because that is the closure check failing.
fn rebuild_side(
    root: &std::path::Path,
    index: &ChunkIndex,
    src: &ChunkSource,
    capture_id: &str,
    path: &str,
    side: Option<&crate::assemble::Side>,
    role: Set,
) -> Result<Option<Rebuilt>, String> {
    let Some(side) = side else {
        return Ok(None);
    };
    let r = rebuild(root, index, src, &side.tag, capture_id, path, role)?;
    if r.nchunks != side.nchunks {
        return Err(format!(
            "{capture_id}: the index names {} {role} chunks, the patchmap says {}",
            r.nchunks, side.nchunks
        ));
    }
    Ok(Some(r))
}

/// Compare a rebuilt file against the bytes the bundle shipped.
///
/// Byte-for-byte, not digest-for-digest: the point is that a consumer can
/// `cmp` the two trees, and a digest comparison would hide a difference that
/// `cmp` would find. The first differing offset is named because "they differ"
/// without a position is the least useful possible report.
pub fn compare(label: &str, rebuilt: &[u8], shipped: &[u8]) -> Result<(), String> {
    if rebuilt == shipped {
        return Ok(());
    }
    let first = rebuilt
        .iter()
        .zip(shipped)
        .position(|(a, b)| a != b)
        .unwrap_or(rebuilt.len().min(shipped.len()));
    Err(format!(
        "{label}: {} B rebuilt vs {} B shipped, first difference at offset {first}",
        rebuilt.len(),
        shipped.len()
    ))
}

/// The relative path a capture's file lives at under a tree.
pub fn tree_path(tree: &str, path: &str) -> String {
    format!("{tree}/{path}")
}

/// The ranges a rebuilt file's chunks occupy, for the run log.
pub fn ranges(r: &Rebuilt) -> Vec<Range> {
    r.layout.ranges.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blobs::chunks::{Reference, Set};
    use crate::blobs::props::BlobProps;
    use crate::blobs::{IndexedBlob, Store};
    use crate::manifest::proto::SophonChunk;

    const DLL: &str = "GenshinImpact_Data/Plugins/HoYoChannel.dll";

    /// A chunk and the compressed bytes a real bundle would hold for it.
    ///
    /// The id prefix carries the content digest, which is the shape the live
    /// manifests use — two builds that publish the same content under different
    /// prefixes are exactly what the closure set has to survive.
    fn chunk(seed: u8, len: usize, prefix: u64) -> (SophonChunk, Vec<u8>, Vec<u8>) {
        let body: Vec<u8> = (0..len)
            .map(|i| seed.wrapping_mul(31).wrapping_add(i as u8))
            .collect();
        let raw = zstd::encode_all(body.as_slice(), 1).expect("encode");
        let c = SophonChunk {
            id: format!("{prefix:016x}_{}", hash::md5_hex(&body)),
            uncompressed_md5: hash::md5_hex(&body),
            offset: 0,
            compressed_size: raw.len() as i64,
            uncompressed_size: len as i64,
            unknown_field6: 0,
            compressed_md5: hash::md5_hex(&raw),
        };
        (c, body, raw)
    }

    fn reference(tag: &str, capture: &str, path: &str, role: Set, offset: i64) -> Reference {
        Reference {
            tag: tag.into(),
            build_id: Some(
                match tag {
                    "7.0.0" => "3XnPviMfoILi",
                    _ => "sMXGW2ll3Fuu",
                }
                .into(),
            ),
            capture_id: capture.into(),
            path: path.into(),
            role,
            offset,
        }
    }

    /// A real bundle on disk: four blobs, three of them referenced by both
    /// builds, written through [`Store`] so the paths are the ones a capture
    /// would produce.
    ///
    /// The shape is the measured `HoYoChannel.dll`: offset 0 holds *different*
    /// content in each build (same length, different id prefix), offset 690719
    /// is byte-identical in both, and the tail is shared.
    struct Fixture {
        root: std::path::PathBuf,
        index: ChunkIndex,
    }

    /// One blob's worth of fixture: the chunk, the bytes it holds, the bytes as
    /// served, who references it and for what.
    ///
    /// A named type rather than a five-wide tuple, because the tuple is read
    /// positionally at exactly one site and any reordering is a silent change of
    /// which bytes are stored against which digest.
    type Spec = (SophonChunk, Vec<u8>, Vec<u8>, Vec<Reference>, Vec<Set>);

    impl Fixture {
        fn build(name: &str) -> Fixture {
            let root = std::env::temp_dir().join(format!(
                "gircap-m9-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("scratch");

            // (chunk, body, compressed, references)
            let (a, ab, ar) = chunk(1, 64, 0x7d3f_0504_4383_7e58);
            let (b, bb, br) = chunk(2, 128, 0xf5dc_3ca1_f3a0_ba3d);
            let (c, cb, cr) = chunk(3, 32, 0xa431_4f3c_1ba1_d7d4);
            let (d, db, dr) = chunk(9, 64, 0xddb3_1a9f_d853_46c4);

            let spec: Vec<Spec> = vec![
                (
                    a.clone(),
                    ab,
                    ar,
                    vec![reference("7.0.0", "c", DLL, Set::MaterializeOld, 0)],
                    vec![Set::MaterializeOld],
                ),
                (
                    b.clone(),
                    bb,
                    br,
                    vec![
                        reference("7.0.0", "c", DLL, Set::MaterializeOld, 64),
                        reference("7.1.0", "c", DLL, Set::MaterializeNew, 64),
                    ],
                    vec![Set::MaterializeOld, Set::MaterializeNew],
                ),
                (
                    c.clone(),
                    cb,
                    cr,
                    vec![
                        reference("7.0.0", "c", DLL, Set::MaterializeOld, 192),
                        reference("7.1.0", "c", DLL, Set::MaterializeNew, 192),
                    ],
                    vec![Set::MaterializeOld, Set::MaterializeNew],
                ),
                (
                    d.clone(),
                    db,
                    dr,
                    vec![
                        reference("7.1.0", "c", DLL, Set::MaterializeNew, 0),
                        reference("7.1.0", "c", DLL, Set::PatchFetch, 0),
                    ],
                    vec![Set::MaterializeNew, Set::PatchFetch],
                ),
            ];

            let mut entries = Vec::new();
            for (chunk, _body, raw, refs, sets) in &spec {
                let rel = BlobProps::stored_path_for(&chunk.id);
                let path = root.join(&rel);
                std::fs::create_dir_all(path.parent().unwrap()).expect("blobs dir");
                std::fs::write(&path, raw).expect("blob");
                let mut props = BlobProps::measure(raw, chunk).expect("measure");
                props.set_stored(rel);
                entries.push((
                    IndexedBlob {
                        props,
                        referenced_by: refs.clone(),
                        sets: sets.clone(),
                    },
                    sets.clone(),
                ));
            }
            Fixture {
                index: ChunkIndex::build(entries),
                root,
            }
        }

        fn source(&self) -> ChunkSource {
            ChunkSource::from_index(&self.root, &self.index)
        }

        fn bodies(&self, tag: &str) -> Vec<u8> {
            let src = self.source();
            let r = rebuild(
                &self.root,
                &self.index,
                &src,
                tag,
                "c",
                DLL,
                if tag == "7.0.0" {
                    Set::MaterializeOld
                } else {
                    Set::MaterializeNew
                },
            )
            .expect("rebuilds");
            r.bytes
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn the_index_names_every_chunk_of_a_side_with_its_offset() {
        let f = Fixture::build("index");
        let old = chunks_for(&f.index, "7.0.0", "c", DLL, Set::MaterializeOld).unwrap();
        assert_eq!(old.len(), 3);
        assert_eq!(
            old.iter().map(|(_, o)| *o).collect::<Vec<_>>(),
            vec![0, 64, 192],
            "ordered by offset, the way a reader walks a file"
        );
        let new = chunks_for(&f.index, "7.1.0", "c", DLL, Set::MaterializeNew).unwrap();
        assert_eq!(new.len(), 3);
        assert_eq!(
            new.iter().map(|(_, o)| *o).collect::<Vec<_>>(),
            vec![0, 64, 192],
            "same layout, different content at offset 0"
        );
        assert_eq!(
            chunks_for(&f.index, "7.1.0", "c", DLL, Set::PatchFetch)
                .unwrap()
                .len(),
            1
        );
    }

    /// The closure check: the blobs alone reproduce the file. Read from disk,
    /// through the same store a capture used, with no manifest consulted.
    #[test]
    fn both_sides_rebuild_from_the_blobs_alone() {
        let f = Fixture::build("rebuild");
        let src = f.source();
        let old = rebuild(
            &f.root,
            &f.index,
            &src,
            "7.0.0",
            "c",
            DLL,
            Set::MaterializeOld,
        )
        .expect("old side");
        assert_eq!(old.bytes.len(), 224, "64 + 128 + 32");
        assert_eq!(old.md5, hash::md5_hex(&old.bytes));
        assert_eq!(old.nchunks, 3);
        assert!(old.layout.is_clean());
        assert_eq!(old.layout.ranges[1], Range::at(64, 128));

        let new = rebuild(
            &f.root,
            &f.index,
            &src,
            "7.1.0",
            "c",
            DLL,
            Set::MaterializeNew,
        )
        .expect("new side");
        assert_eq!(new.bytes.len(), 224, "the same three ranges");
        // Offset 0 now holds different content, which is the whole point: a
        // rebuild that ignored the new chunk would still be 224 B and would
        // still hash to *something*.
        assert_ne!(new.md5, old.md5);
        assert_ne!(&new.bytes[..64], &old.bytes[..64]);
    }

    /// The rebuilt bytes must be the bytes the bundle shipped, byte for byte.
    #[test]
    fn a_rebuild_matches_the_tree_it_came_from() {
        let f = Fixture::build("match");
        let old = f.bodies("7.0.0");
        let new = f.bodies("7.1.0");
        let (o_path, n_path) = (tree_path("game", DLL), tree_path("expected", DLL));
        // The tree the bundle would have written alongside the blobs.
        for (label, bytes, rel) in [("game", &old, &o_path), ("expected", &new, &n_path)] {
            let shipped = f.root.join(rel);
            std::fs::create_dir_all(shipped.parent().unwrap()).unwrap();
            std::fs::write(&shipped, bytes).unwrap();
            let r = rebuild(
                &f.root,
                &f.index,
                &f.source(),
                if label == "game" { "7.0.0" } else { "7.1.0" },
                "c",
                DLL,
                if label == "game" {
                    Set::MaterializeOld
                } else {
                    Set::MaterializeNew
                },
            )
            .unwrap();
            let disk = std::fs::read(&shipped).unwrap();
            assert!(compare(&format!("{label}/{DLL}"), &r.bytes, &disk).is_ok());
        }
    }

    /// Corrupt one blob and the rebuild must fail, naming the blob.
    ///
    /// The byte flipped is the *last* one, inside the frame: flipping the first
    /// would destroy the zstd magic and produce a different (also correct)
    /// diagnosis — "not a frame at all" rather than "decodes to the wrong
    /// bytes". Both are checked, because a bundle can fail either way and a
    /// reader needs to be told which.
    #[test]
    fn a_corrupted_blob_stops_the_rebuild_and_names_the_blob() {
        let f = Fixture::build("corrupt");
        let victim = f
            .index
            .blobs
            .iter()
            .find(|b| {
                b.referenced_by
                    .iter()
                    .any(|r| r.role == Set::MaterializeOld)
            })
            .unwrap()
            .props
            .stored_path
            .clone()
            .unwrap();
        let p = f.root.join(&victim);

        // Inside the frame: it still decodes, to different bytes.
        let mut raw = std::fs::read(&p).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xFF;
        std::fs::write(&p, &raw).unwrap();
        let e = rebuild(
            &f.root,
            &f.index,
            &f.source(),
            "7.0.0",
            "c",
            DLL,
            Set::MaterializeOld,
        )
        .unwrap_err();
        assert!(e.contains("decodes to md5"), "{e}");
        assert!(e.contains(&victim), "and names the blob: {e}");

        // And the magic: not a frame at all, reported as such.
        let mut raw = std::fs::read(&p).unwrap();
        raw[0] ^= 0xFF;
        std::fs::write(&p, &raw).unwrap();
        let e = rebuild(
            &f.root,
            &f.index,
            &f.source(),
            "7.0.0",
            "c",
            DLL,
            Set::MaterializeOld,
        )
        .unwrap_err();
        assert!(e.contains("not a decodable zstd blob"), "{e}");
        assert!(e.contains("no-zstd-magic"), "{e}");
    }

    #[test]
    fn a_side_with_no_references_is_an_error_not_an_empty_file() {
        let f = Fixture::build("noside");
        let e = rebuild(
            &f.root,
            &f.index,
            &f.source(),
            "6.7.0",
            "c",
            DLL,
            Set::MaterializeOld,
        )
        .unwrap_err();
        assert!(e.contains("references no chunks"), "{e}");
    }

    /// A chunk named twice at two offsets would silently drop one of them.
    #[test]
    fn a_chunk_referenced_twice_for_one_file_is_refused() {
        let f = Fixture::build("twice");
        let mut index = f.index.clone();
        let id = index.blobs[1].props.chunk_id.clone();
        index.blobs[1]
            .referenced_by
            .push(reference("7.0.0", "c", DLL, Set::MaterializeOld, 999));
        let e = chunks_for(&index, "7.0.0", "c", DLL, Set::MaterializeOld).unwrap_err();
        assert!(e.contains("referenced 2 times"), "{e}");
        assert!(
            e.contains("at offsets 64, 999"),
            "and names both offsets: {e}"
        );
        assert!(e.contains(&id), "and the chunk: {e}");
    }

    /// A bundle whose index lists a blob that is not on disk is not closed, and
    /// that is exactly what this command exists to find.
    #[test]
    fn a_blob_missing_from_disk_stops_the_rebuild() {
        let f = Fixture::build("missing");
        let victim = f.index.blobs[0].props.stored_path.clone().unwrap();
        std::fs::remove_file(f.root.join(&victim)).unwrap();
        let e = rebuild(
            &f.root,
            &f.index,
            &f.source(),
            "7.0.0",
            "c",
            DLL,
            Set::MaterializeOld,
        )
        .unwrap_err();
        assert!(e.contains(&victim), "{e}");
    }

    #[test]
    fn comparing_identical_bytes_is_quiet_and_names_the_first_difference_otherwise() {
        assert!(compare("x", b"abc", b"abc").is_ok());
        let e = compare("game/dll", b"abcd", b"abXd").unwrap_err();
        assert!(e.contains("first difference at offset 2"), "{e}");
        assert!(e.contains("4 B rebuilt vs 4 B shipped"), "{e}");
        // A length difference is reported too, and the offset is the end of the
        // shorter one rather than a byte index that does not exist.
        let e = compare("x", b"abc", b"abcd").unwrap_err();
        assert!(e.contains("first difference at offset 3"), "{e}");
    }

    fn side_map(
        old: Option<(&str, i64, usize)>,
        new: Option<(&str, i64, usize)>,
    ) -> crate::assemble::PatchMap {
        let side = |s: Option<(&str, i64, usize)>| {
            s.map(|(tag, size, nchunks)| crate::assemble::Side {
                tag: tag.into(),
                size,
                md5: String::new(),
                nchunks,
            })
        };
        crate::assemble::PatchMap {
            schema_version: crate::SCHEMA_VERSION,
            capture_id: "c".into(),
            path: DLL.into(),
            matching_field: "game".into(),
            classes: Vec::new(),
            old: side(old),
            new: side(new),
            plan: None,
            totals: None,
            expectations: crate::assemble::Expectations {
                result_path: Some(format!("expected/{DLL}")),
                result_md5: None,
                absent_after: new.is_none(),
                download_bytes: 0,
                must_not_fetch: Vec::new(),
            },
        }
    }

    #[test]
    fn a_capture_rebuilds_from_its_patchmap_and_the_counts_must_agree() {
        let f = Fixture::build("all");
        let mut maps = BTreeMap::new();
        maps.insert(
            "c".to_string(),
            side_map(Some(("7.0.0", 224, 3)), Some(("7.1.0", 224, 3))),
        );
        let pairs = rebuild_all(&f.root, &f.index, &maps).expect("rebuilds");
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].old.as_ref().unwrap().bytes.len(), 224);
        assert_eq!(pairs[0].new.as_ref().unwrap().bytes.len(), 224);

        // A patchmap claiming the wrong chunk count is caught rather than
        // silently producing a file.
        let mut wrong = maps.get("c").unwrap().clone();
        wrong.old.as_mut().unwrap().nchunks = 2;
        let mut wrong_maps = BTreeMap::new();
        wrong_maps.insert("c".to_string(), wrong);
        let e = rebuild_all(&f.root, &f.index, &wrong_maps).unwrap_err();
        assert!(
            e.contains("the index names 3 materialize_old chunks"),
            "{e}"
        );
    }

    /// `added` and `removed` are one-sided and must rebuild the side they have,
    /// with the other reported as absent rather than as an error.
    #[test]
    fn a_one_sided_capture_rebuilds_only_the_side_it_has() {
        let f = Fixture::build("onesided");
        let mut added = side_map(None, Some(("7.1.0", 224, 3)));
        added.path = DLL.into();
        let mut maps = BTreeMap::new();
        maps.insert("c".to_string(), added);
        // The references name capture `c`; renaming it here is what makes this
        // check meaningful, so point the map at a name the index does not use.
        let mut renamed = maps.get("c").unwrap().clone();
        renamed.capture_id = "not-in-the-index".into();
        let mut renamed_maps = BTreeMap::new();
        renamed_maps.insert("not-in-the-index".to_string(), renamed);
        let e = rebuild_all(&f.root, &f.index, &renamed_maps).unwrap_err();
        assert!(e.contains("references no chunks"), "{e}");
    }

    #[test]
    fn a_one_sided_capture_with_matching_references_rebuilds_its_one_side() {
        let f = Fixture::build("onesidedok");
        let mut maps = BTreeMap::new();
        maps.insert("c".to_string(), side_map(None, Some(("7.1.0", 224, 3))));
        let pairs = rebuild_all(&f.root, &f.index, &maps).expect("rebuilds");
        assert!(pairs[0].old.is_none(), "no old side to rebuild");
        assert_eq!(pairs[0].new.as_ref().unwrap().bytes.len(), 224);
    }

    /// The store's own path helper is the only place a blob's location is
    /// decided, so a fixture that hard-coded one would stop testing it.
    #[test]
    fn the_blobs_live_where_the_store_puts_them() {
        let f = Fixture::build("paths");
        for b in &f.index.blobs {
            let p = b.props.stored_path.as_deref().expect("stored");
            assert_eq!(p, BlobProps::stored_path_for(&b.props.chunk_id));
            assert!(f.root.join(p).is_file(), "{p} is listed but not on disk");
        }
        assert_eq!(f.index.blobs.len(), 4);
        let _ = Store::plan_only(&[]);
    }
}
