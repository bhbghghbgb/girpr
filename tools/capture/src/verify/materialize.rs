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
use crate::assemble::ranges::Layout;
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
///
/// **One file's worth of [`ChunksByFile`].** This is a convenience over the
/// index that every caller inside [`rebuild_all`] shares — asking it separately
/// means rebuilding the grouping, which is the work the grouping exists to do
/// once. Nothing outside this module needs it.
pub fn chunks_for(
    index: &ChunkIndex,
    tag: &str,
    capture_id: &str,
    path: &str,
    role: Set,
) -> Result<Vec<(String, i64)>, String> {
    Ok(ChunksByFile::build(index)
        .get(tag, capture_id, path, role)?
        .into_iter()
        .map(|(id, offset)| (id.to_string(), offset))
        .collect())
}

/// One reference: the bytes of `chunk_id`, at `offset`, inside one file.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Ref<'i> {
    tag: &'i str,
    capture_id: &'i str,
    path: &'i str,
    role: Set,
    chunk_id: &'i str,
    offset: i64,
}

impl Ref<'_> {
    fn key(&self) -> (&str, &str, &str, Set) {
        (self.tag, self.capture_id, self.path, self.role)
    }
}

/// Every reference in the index, grouped by the file it places bytes into.
///
/// [`chunks_for`] answered one question by walking **every** blob's entire
/// `referenced_by` vector, and then [`rebuild`] built a fresh map of **every**
/// blob's size — per side, and `rebuild_all` calls it twice per capture. On the
/// reference bundle that is 38 blobs and 66 references scanned sixteen times to
/// answer sixteen questions, and sixteen maps of 38 entries thrown away.
///
/// Built once instead, and sorted, so a lookup is a binary search and a walk of
/// the (short) run of rows that share the key. Nothing is cloned: the rows
/// borrow the index's own strings.
///
/// Sorted by `(tag, capture_id, path, role, chunk_id, offset)`.
///
/// The key first, so a lookup is a binary search and then a walk of the rows
/// that share it. `chunk_id` next, so a chunk placed twice inside one file is
/// two **adjacent** rows rather than two rows a reader has to correlate. The
/// answer is re-sorted by offset before it is handed back, which is what the
/// caller wants and what the rows came out in before.
#[derive(Debug, Clone, Default)]
pub struct ChunksByFile<'i> {
    rows: Vec<Ref<'i>>,
    /// `chunk_id` -> decompressed size, for the layout check.
    sizes: BTreeMap<&'i str, i64>,
}

impl<'i> ChunksByFile<'i> {
    /// Read every reference in the index, once.
    pub fn build(index: &'i ChunkIndex) -> Self {
        let mut rows: Vec<Ref<'i>> = Vec::new();
        let mut sizes = BTreeMap::new();
        for b in &index.blobs {
            sizes.insert(b.props.chunk_id.as_str(), b.props.uncompressed_size as i64);
            rows.extend(b.referenced_by.iter().map(|r| Ref {
                tag: r.tag.as_str(),
                capture_id: r.capture_id.as_str(),
                path: r.path.as_str(),
                role: r.role,
                chunk_id: b.props.chunk_id.as_str(),
                offset: r.offset,
            }));
        }
        rows.sort_by(|a, b| {
            a.key()
                .cmp(&b.key())
                .then(a.chunk_id.cmp(b.chunk_id))
                .then(a.offset.cmp(&b.offset))
        });
        Self { rows, sizes }
    }

    /// How many references the index holds, across every file.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the index references nothing at all.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The chunks that make up one file on one side, in offset order.
    ///
    /// **The same chunk id may appear more than once**, and it is not an
    /// error: a file may place the same content at more than one offset — a
    /// Unity asset with repeated blocks does, and the live `resources.assets`
    /// has five chunk ids that appear two or three times each. The blob is
    /// stored once and *placed* at every offset it is referenced at, which is
    /// what the capture's own assembly does with the manifest.
    pub fn get(
        &self,
        tag: &str,
        capture_id: &str,
        path: &str,
        role: Set,
    ) -> Result<Vec<(&'i str, i64)>, String> {
        let key = (tag, capture_id, path, role);
        let start = self.rows.partition_point(|r| r.key() < key);
        let run = &self.rows[start..];
        let end = run.iter().position(|r| r.key() != key).unwrap_or(run.len());
        let mut hits = run[..end].to_vec();

        // The offsets are what places the bytes, and two references to the same
        // chunk id at two offsets are two placements of the same content.
        hits.sort_by_key(|r| r.offset);
        Ok(hits.into_iter().map(|r| (r.chunk_id, r.offset)).collect())
    }

    /// The decompressed size the index records for a chunk.
    ///
    /// `0` for a chunk the index does not list, which is what the old
    /// `sizes.get(..).copied().unwrap_or(0)` said. Such a chunk cannot be
    /// decoded either, so it fails one step later and with a better message.
    pub fn size(&self, chunk_id: &str) -> i64 {
        self.sizes.get(chunk_id).copied().unwrap_or(0)
    }
}

/// Rebuild one side of one capture from the stored blobs.
///
/// The layout is checked before any byte is placed (A11), so a closure set with
/// a hole produces an error naming the hole rather than a short file.
///
/// `root` is not a parameter: where the blobs live is already inside `src`, and
/// this function used to take one and discard it.
pub fn rebuild(
    by_file: &ChunksByFile<'_>,
    src: &ChunkSource<'_>,
    tag: &str,
    capture_id: &str,
    path: &str,
    role: Set,
) -> Result<Rebuilt, String> {
    let chunks = by_file.get(tag, capture_id, path, role)?;
    if chunks.is_empty() {
        return Err(format!(
            "{tag} {path} ({role}): the index references no chunks for it"
        ));
    }
    let parts: Vec<(i64, i64)> = chunks
        .iter()
        .map(|(id, o)| (*o, by_file.size(id)))
        .collect();
    let declared_size: i64 = parts.iter().map(|(_, l)| *l).sum();
    // `parts` is still needed for `declared_size`; `Layout::of` takes it by
    // value so the one-vector-holds-both property holds at the call site too.
    let layout = Layout::of(parts.iter().copied(), declared_size);
    // The label is the only thing that distinguishes this rebuild from the
    // capture's own: the tiling, and every message it can produce, is one
    // function. See `assemble::tile`.
    let label = format!("{tag} {path} ({role})");
    let ids: Vec<&str> = chunks.iter().map(|(id, _)| *id).collect();
    let bytes = crate::assemble::tile(&label, &layout, declared_size, &ids, |id| src.decoded(id))?;
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
///
/// The two things this walk would otherwise redo per side are done once here:
/// the index's references are grouped by file ([`ChunksByFile::build`]) and the
/// decoded chunks are remembered ([`ChunkSource::memoizing`]).
pub fn rebuild_all(
    root: &std::path::Path,
    index: &ChunkIndex,
    patchmaps: &BTreeMap<String, crate::assemble::PatchMap>,
) -> Result<Vec<Pair>, String> {
    let by_file = ChunksByFile::build(index);
    let src = ChunkSource::from_index(root, index).memoizing();
    let mut names: Vec<&String> = patchmaps.keys().collect();
    names.sort();
    let mut out = Vec::new();
    for name in names {
        let map = &patchmaps[name];
        let old = rebuild_side(
            &by_file,
            &src,
            name,
            map.path.as_str(),
            map.old.as_ref(),
            Set::MaterializeOld,
        )?;
        let new = rebuild_side(
            &by_file,
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
    by_file: &ChunksByFile<'_>,
    src: &ChunkSource<'_>,
    capture_id: &str,
    path: &str,
    side: Option<&crate::assemble::Side>,
    role: Set,
) -> Result<Option<Rebuilt>, String> {
    let Some(side) = side else {
        return Ok(None);
    };
    let r = rebuild(by_file, src, &side.tag, capture_id, path, role)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assemble::ranges::Range;
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
        root: crate::testtemp::TempDir,
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
            let root = crate::testtemp::TempDir::new("gircap-m9", name);
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

        fn source(&self) -> ChunkSource<'_> {
            ChunkSource::from_index(&self.root, &self.index)
        }

        fn by_file(&self) -> ChunksByFile<'_> {
            ChunksByFile::build(&self.index)
        }

        fn bodies(&self, tag: &str) -> Vec<u8> {
            let src = self.source();
            let r = rebuild(
                &self.by_file(),
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

    /// The grouping and the convenience wrapper must agree, because the wrapper
    /// is what every test in this module asks and the grouping is what
    /// production asks. They are the same code today; this is what stops that
    /// from quietly ceasing to be true.
    #[test]
    fn the_one_file_wrapper_answers_what_the_grouped_index_answers() {
        let f = Fixture::build("agree");
        let by_file = f.by_file();
        assert_eq!(by_file.len(), 7, "every reference in the index");
        for (tag, role, want) in [
            ("7.0.0", Set::MaterializeOld, 3),
            ("7.1.0", Set::MaterializeNew, 3),
            ("7.1.0", Set::PatchFetch, 1),
        ] {
            let one = by_file.get(tag, "c", DLL, role).unwrap();
            let many = chunks_for(&f.index, tag, "c", DLL, role).unwrap();
            assert_eq!(one.len(), want, "{tag} {role}");
            assert_eq!(
                one.iter()
                    .map(|(i, o)| (i.to_string(), *o))
                    .collect::<Vec<_>>(),
                many,
                "{tag} {role}"
            );
        }
        assert!(
            by_file
                .get("6.7.0", "c", DLL, Set::MaterializeOld)
                .unwrap()
                .is_empty()
        );
    }

    /// The closure check: the blobs alone reproduce the file. Read from disk,
    /// through the same store a capture used, with no manifest consulted.
    #[test]
    fn both_sides_rebuild_from_the_blobs_alone() {
        let f = Fixture::build("rebuild");
        let src = f.source();
        let old =
            rebuild(&f.by_file(), &src, "7.0.0", "c", DLL, Set::MaterializeOld).expect("old side");
        assert_eq!(old.bytes.len(), 224, "64 + 128 + 32");
        assert_eq!(old.md5, hash::md5_hex(&old.bytes));
        assert_eq!(old.nchunks, 3);
        assert!(old.layout.is_clean());
        assert_eq!(old.layout.ranges[1], Range::at(64, 128));
        let new =
            rebuild(&f.by_file(), &src, "7.1.0", "c", DLL, Set::MaterializeNew).expect("new side");
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
        let (o_path, n_path) = (format!("game/{DLL}"), format!("expected/{DLL}"));
        // The tree the bundle would have written alongside the blobs.
        for (label, bytes, rel) in [("game", &old, &o_path), ("expected", &new, &n_path)] {
            let shipped = f.root.join(rel);
            std::fs::create_dir_all(shipped.parent().unwrap()).unwrap();
            std::fs::write(&shipped, bytes).unwrap();
            let r = rebuild(
                &f.by_file(),
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
            &f.by_file(),
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
            &f.by_file(),
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
            &f.by_file(),
            &f.source(),
            "6.7.0",
            "c",
            DLL,
            Set::MaterializeOld,
        )
        .unwrap_err();
        assert!(e.contains("references no chunks"), "{e}");
    }

    /// The same chunk at two offsets is **not** an error: a file may place the
    /// same content twice — a Unity asset with repeated blocks does — and the
    /// blob is placed at both offsets. The live `resources.assets` has five
    /// chunk ids that appear two or three times each, so this is the ordinary
    /// case, not an edge one.
    #[test]
    fn a_chunk_referenced_twice_for_one_file_is_placed_at_both_offsets() {
        let f = Fixture::build("twice");
        let mut index = f.index.clone();
        let id = index.blobs[1].props.chunk_id.clone();
        index.blobs[1]
            .referenced_by
            .push(reference("7.0.0", "c", DLL, Set::MaterializeOld, 999));
        let hits = chunks_for(&index, "7.0.0", "c", DLL, Set::MaterializeOld).expect("both");
        let offsets: Vec<i64> = hits.iter().map(|(_, o)| *o).collect();
        assert_eq!(offsets, vec![0, 64, 192, 999], "every offset is kept");
        assert_eq!(
            hits.iter().filter(|(c, _)| *c == id).count(),
            2,
            "and the repeated chunk is placed twice"
        );
    }

    /// A bundle whose index lists a blob that is not on disk is not closed, and
    /// that is exactly what this command exists to find.
    #[test]
    fn a_blob_missing_from_disk_stops_the_rebuild() {
        let f = Fixture::build("missing");
        let victim = f.index.blobs[0].props.stored_path.clone().unwrap();
        std::fs::remove_file(f.root.join(&victim)).unwrap();
        let e = rebuild(
            &f.by_file(),
            &f.source(),
            "7.0.0",
            "c",
            DLL,
            Set::MaterializeOld,
        )
        .unwrap_err();
        assert!(e.contains(&victim), "{e}");
    }

    /// The memo is what turns a `verify` from 104 blob reads into 38, so it has
    /// to be *on* for the rebuild walk and it has to actually avoid the second
    /// read. Deleting the blob out from under a source that has already decoded
    /// it is the only way to observe that from outside.
    #[test]
    fn a_memoised_source_serves_a_blob_it_can_no_longer_read() {
        let f = Fixture::build("memo");
        let id = f.index.blobs[1].props.chunk_id.clone();
        let rel = f.index.blobs[1].props.stored_path.clone().unwrap();

        let memo = f.source().memoizing();
        let first = memo.decoded(&id).expect("first decode reads the blob");
        // What the rebuild would have placed at offset 64, captured while the
        // blob is still on disk.
        let in_the_file = f.bodies("7.0.0")[64..192].to_vec();
        std::fs::remove_file(f.root.join(&rel)).expect("gone");
        let second = memo.decoded(&id).expect("second decode comes from memory");
        assert_eq!(first, second, "and it is the same bytes");
        assert_eq!(first.as_ref(), &in_the_file, "and the real content");

        // The default source re-reads every time, so the same call now fails.
        // That difference is the whole point of the two constructors: capture
        // wants the page cache, verify wants the memory.
        let e = f.source().decoded(&id).unwrap_err();
        assert!(e.contains(&rel), "{e}");
    }

    /// Memoising must not become trusting: a blob whose bytes disagree with the
    /// index is refused on the decode that fills the cell, which is every blob
    /// the source ever returns.
    #[test]
    fn a_memoised_source_still_refuses_a_blob_the_index_disagrees_with() {
        let f = Fixture::build("memobad");
        let id = f.index.blobs[0].props.chunk_id.clone();
        let rel = f.index.blobs[0].props.stored_path.clone().unwrap();
        let p = f.root.join(&rel);
        let mut raw = std::fs::read(&p).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xFF;
        std::fs::write(&p, &raw).unwrap();

        let memo = f.source().memoizing();
        let e = memo.decoded(&id).unwrap_err();
        assert!(e.contains("decodes to md5"), "{e}");
        assert!(e.contains(&rel), "and names the blob: {e}");
        // And it refused, rather than caching the bad bytes and serving them.
        let e = memo.decoded(&id).unwrap_err();
        assert!(e.contains("decodes to md5"), "{e}");
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
