//! The three retention sets, and how a chunk is assigned to them.
//!
//! **M6.** `patch_fetch` is `materialize_new` minus every chunk whose identity
//! appears in **this same old file** — same-file only, which is the
//! simplification under measurement, and `reuse_pool.json` is what bounds it.
//!
//! ## Identity is `(uncompressed_md5, uncompressed_size)`
//!
//! Not `chunk.id`. The id is `<16 opaque hex>_<uncompressed_md5>` and the
//! prefix is not a function of anything observable, so two builds that publish
//! the same bytes under different prefixes hand out different ids for the same
//! content. The measured 7.0.0 → 7.1.0 pair has exactly this: `HoYoChannel.dll`
//! offset 0 is `bbce8a31…` in the old build and `de1cfe69…` in the new one —
//! different content at the same offset — while offset 690719 is the *same* id
//! in both, because its content really is the same.
//!
//! A reuse test keyed on the id would therefore be right only by accident, and
//! the accident would look like a feature. Keyed on content, the same test is
//! right by construction and `reuse_pool.json`'s cross-file-only count is the
//! measurement of what it gives up.
//!
//! ## Why the sets are per-file and not per-bundle
//!
//! A chunk id appears in `materialize_old` for one file and `materialize_new`
//! for another. Collapsing that into one bundle-wide set would make the
//! `patch_fetch ⊆ materialize_new` assertion (A12) unfalsifiable — a chunk
//! fetched for file A would appear to satisfy file B's requirement — so the sets
//! are built per [`FilePair`] and merged only at the index, where
//! `referenced_by` keeps the two apart.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::manifest::proto::{SophonChunk, SophonChunkFile};

/// The three roles a blob is retained for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Set {
    /// Needed to rebuild the old file — every chunk of it.
    MaterializeOld,
    /// Needed to rebuild the new file — every chunk of it.
    MaterializeNew,
    /// Needed by the *patch* flow, which is `materialize_new` minus what it can
    /// reuse from the old file's own bytes.
    PatchFetch,
}

impl Set {
    /// All three, in the order `chunks/index.json` lists them.
    pub const ALL: [Set; 3] = [Set::MaterializeOld, Set::MaterializeNew, Set::PatchFetch];

    /// The wire spelling, which is also the directory-independent key a reader
    /// uses to pull one list out of `index.json`.
    pub const fn id(self) -> &'static str {
        match self {
            Set::MaterializeOld => "materialize_old",
            Set::MaterializeNew => "materialize_new",
            Set::PatchFetch => "patch_fetch",
        }
    }
}

impl std::fmt::Display for Set {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

/// What a chunk's identity is for reuse purposes.
///
/// **Borrows its digest.** This owned a `String` that was cloned once per
/// *comparison*: `retention` asked "is this new chunk's identity in this old
/// file" and allocated 32 bytes to ask it, once per new chunk, for a set
/// membership test that needed no allocation at all. `(&str, i64)` is already
/// `Hash + Eq`, so this is a view over the manifest and nothing copies.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Identity<'a> {
    /// The content digest.
    pub uncompressed_md5: &'a str,
    /// The decoded length.
    ///
    /// Carried alongside the digest because it is what makes the pair a
    /// *complete* claim: the same MD5 with a different length is not the same
    /// bytes, and a store keyed on the digest alone would assemble a file of the
    /// wrong length from the right pieces.
    pub uncompressed_size: i64,
}

impl<'a> Identity<'a> {
    /// The identity of a manifest chunk.
    pub fn of(c: &'a SophonChunk) -> Identity<'a> {
        Identity {
            uncompressed_md5: c.uncompressed_md5.as_str(),
            uncompressed_size: c.uncompressed_size,
        }
    }
}

/// One use of a blob: which file, which build, which role, at which offset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    /// The build this use belongs to.
    pub tag: String,
    /// The build id the chunk is addressed under — the `<build_id>` path segment
    /// in `…/chunks/<series>/<build_id>/<16hex>_<32hex>`.
    ///
    /// Part of the chunk's **address**, not a label: the same chunk id under the
    /// other build's prefix is a 404, confirmed against the live
    /// `autopatchhk.yuanshen.com` (`Server: AliyunOSS`, `application/xml`
    /// `NoSuchKey`). The tag alone does not determine it — two tags can map to
    /// different build ids over time — so a consumer that has to build a URL or
    /// decide whether a request was namespaced correctly needs this and cannot
    /// derive it from `tag`.
    ///
    /// `None` on a bundle captured before this field existed. Such a document
    /// still reads; the fallback is `MANIFEST.json` → `versions.{old,new}.build_id`
    /// matched on `tag`, which is where the value came from in the first place.
    #[serde(default)]
    pub build_id: Option<String>,
    /// The capture that wanted the file.
    pub capture_id: String,
    /// The manifest-relative path.
    pub path: String,
    /// What the bytes are needed for.
    pub role: Set,
    /// Where the chunk sits in the file it belongs to.
    pub offset: i64,
}

/// One chunk the store must have, with every reason it is wanted.
#[derive(Debug, Clone, PartialEq)]
pub struct Want {
    /// The declaration the store measures against, taken from the first source.
    pub chunk: SophonChunk,
    /// Where the bytes can be fetched from, best first.
    ///
    /// More than one when both builds declare this chunk id. That is not a
    /// conflict — it is the shared-chunk case, and it is the reason the store
    /// fetches **once** for a pair of files the acceptance counts as five
    /// distinct ids. Ordered newest-tag-first because the old build's CDN may
    /// stop serving while the new one's certainly does not, and a capture that
    /// worked yesterday should not be the reason a fixture cannot be rebuilt.
    pub sources: Vec<Source>,
    /// Every file that wants these bytes, across both builds.
    pub references: Vec<Reference>,
    /// The distinct roles it is retained for.
    pub sets: Vec<Set>,
    /// Declarations from other sources that disagree with [`Want::chunk`].
    ///
    /// Kept rather than resolved. Two manifests publishing the same id for
    /// different content is a contradiction, and picking a winner silently would
    /// mean the selfcheck could never see it — the blob would be measured green
    /// against whichever declaration happened to be first.
    pub conflicts: Vec<Conflict>,
}

/// A build that declares a chunk, and therefore a CDN that serves it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Source {
    /// The tag, which is the only thing the store needs to resolve a URL.
    pub tag: String,
}

/// A source whose declaration disagrees with the one the store will measure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    /// The build that disagreed.
    pub tag: String,
    /// Its `uncompressed_md5`.
    pub uncompressed_md5: String,
    /// Its `uncompressed_size`.
    pub uncompressed_size: i64,
}

impl Want {
    /// Whether any role wants it.
    pub fn is_wanted(&self) -> bool {
        !self.sets.is_empty()
    }

    /// The best source: the one whose bytes the store should ask for.
    pub fn best_source(&self) -> Option<&str> {
        self.sources.first().map(|s| s.tag.as_str())
    }

    /// A one-line description of a disagreement, for the run log.
    pub fn describe_conflicts(&self) -> String {
        self.conflicts
            .iter()
            .map(|c| {
                format!(
                    "{} declares {} as {}/{}",
                    c.tag, self.chunk.id, c.uncompressed_md5, c.uncompressed_size
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// One file's old and new entries, and what has to be retained to work with them.
#[derive(Debug, Clone, PartialEq)]
pub struct FilePair {
    /// The capture that wants it.
    pub capture_id: String,
    /// The manifest-relative path.
    pub path: String,
    /// The old entry, absent for an `added` path.
    pub old: Option<OldFile>,
    /// The new entry, absent for a `removed` path.
    pub new: Option<NewFile>,
}

/// One build's file entry, with the tag it belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct OldFile {
    /// The tag.
    pub tag: String,
    /// The build id the entry was published under. Carried with the tag rather
    /// than looked up later, because it is the tag's address at capture time and
    /// a later `getBuild` for the same tag may disagree.
    pub build_id: String,
    /// The entry.
    pub entry: SophonChunkFile,
}

/// The new side. A separate type rather than a tag field on one type, because
/// `removed` has no new side and a single struct would make every use of it a
/// `None`-check that a reader could get wrong.
#[derive(Debug, Clone, PartialEq)]
pub struct NewFile {
    /// The tag.
    pub tag: String,
    /// The build id the entry was published under.
    pub build_id: String,
    /// The entry.
    pub entry: SophonChunkFile,
}

/// What retaining a file costs and what it would fetch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Retention {
    /// Chunk ids, in file-offset order, for each role.
    pub sets: BTreeMap<Set, Vec<String>>,
    /// New-side chunks whose identity is in *this* old file, in file order.
    pub reuse: Vec<String>,
    /// New-side chunk ids that would be fetched by the patch flow.
    pub fetch: Vec<String>,
}

impl Retention {
    /// The ids retained for one role.
    pub fn ids(&self, set: Set) -> &[String] {
        self.sets.get(&set).map(Vec::as_slice).unwrap_or(&[])
    }

    /// A12: `patch_fetch` disjoint from the reuse set, and a subset of
    /// `materialize_new`.
    ///
    /// Checked here rather than only in the selfcheck because it is a property
    /// of the *plan*, and a plan that violated it would produce an artifact
    /// whose parts are right and whose conclusion is wrong.
    ///
    /// **Delegates to [`Retention::a12_failure`]**, which was the same two
    /// conditions written out again in a different order. Two spellings of one
    /// rule is how a plan that violates one half of A12 gets reported as
    /// satisfying the other.
    pub fn a12_holds(&self) -> bool {
        self.a12_failure().is_none()
    }

    /// Which half of A12 failed, if either.
    ///
    /// One set over `materialize_new`, built once: both conditions are
    /// membership tests against it, and the naive form is O(new²) per file.
    pub fn a12_failure(&self) -> Option<&'static str> {
        let new: std::collections::HashSet<&str> = self
            .ids(Set::MaterializeNew)
            .iter()
            .map(String::as_str)
            .collect();
        let reuse: std::collections::HashSet<&str> =
            self.reuse.iter().map(String::as_str).collect();
        for id in self.ids(Set::PatchFetch) {
            if !new.contains(id.as_str()) {
                return Some("patch_fetch is not a subset of materialize_new");
            }
            if reuse.contains(id.as_str()) {
                return Some("patch_fetch overlaps the reuse set");
            }
        }
        None
    }
}

/// What retaining one file costs: which chunks, in which roles.
///
/// `patch_fetch` is `materialize_new` minus the chunks whose
/// `(uncompressed_md5, uncompressed_size)` this same old file already holds.
/// The subtraction is deliberately *not* by `chunk.id`: see the module docs.
pub fn retention(pair: &FilePair) -> Retention {
    let mut r = Retention::default();
    // A set, not a `Vec` with `contains`. The question is "is this new chunk's
    // identity in this old file", asked once per new chunk against every old
    // chunk — quadratic in a file's chunk count, which is where the measured
    // `HoYoChannel.dll` reaches three and a real `.blk` reaches hundreds.
    let old_identities: std::collections::HashSet<Identity<'_>> = pair
        .old
        .as_ref()
        .map(|o| o.entry.chunks.iter().map(Identity::of).collect())
        .unwrap_or_default();

    if let Some(o) = &pair.old {
        for c in &o.entry.chunks {
            r.sets
                .entry(Set::MaterializeOld)
                .or_default()
                .push(c.id.clone());
        }
    }

    if let Some(n) = &pair.new {
        for c in &n.entry.chunks {
            r.sets
                .entry(Set::MaterializeNew)
                .or_default()
                .push(c.id.clone());
            if old_identities.contains(&Identity::of(c)) {
                r.reuse.push(c.id.clone());
            } else {
                r.fetch.push(c.id.clone());
                r.sets
                    .entry(Set::PatchFetch)
                    .or_default()
                    .push(c.id.clone());
            }
        }
    }
    r
}

/// Every chunk the store needs, merged across files and both builds.
///
/// Keyed by `chunk_id` alone, because that is the content address and a shared
/// chunk is one blob however many files and both builds reference it — the
/// measured `HoYoChannel.dll` case is three old ids, two new-only ids and one
/// shared, so **five** fetches, not six. Where a chunk is declared by both
/// builds it gains two [`Source`]s and is fetched from the newest tag first.
///
/// Returned in a deterministic order so that a capture which stored the same
/// blobs in a different order depending on scheduling would not produce two
/// artifacts for one world.
pub fn wants(pairs: &[FilePair]) -> Vec<Want> {
    let mut by_id: BTreeMap<String, Want> = BTreeMap::new();
    for pair in pairs {
        // One chunk, one role. Taking the whole entry here and looping outside
        // meant the first non-reusable new chunk pushed `patch_fetch` onto
        // *every* new chunk, reusable ones included — which silently broke A12
        // and made the store fetch the chunk it had just proved it need not.
        let mut push = |tag: &str, build_id: &str, c: &SophonChunk, role: Set| {
            let w = by_id.entry(c.id.clone()).or_insert_with(|| Want {
                chunk: c.clone(),
                sources: vec![Source {
                    tag: tag.to_string(),
                }],
                references: Vec::new(),
                sets: Vec::new(),
                conflicts: Vec::new(),
            });
            if w.sources.iter().all(|s| s.tag != tag) {
                // A second build also declares this id. If it agrees, this is
                // the shared-chunk case and the store gains a fallback source;
                // if it does not, that is recorded rather than silently
                // resolved.
                if Identity::of(&w.chunk) == Identity::of(c) {
                    w.sources.push(Source {
                        tag: tag.to_string(),
                    });
                } else if !w.conflicts.iter().any(|k| k.tag == tag) {
                    w.conflicts.push(Conflict {
                        tag: tag.to_string(),
                        uncompressed_md5: c.uncompressed_md5.clone(),
                        uncompressed_size: c.uncompressed_size,
                    });
                }
            }
            // **The offset is part of the key.** A file may place the same
            // content at more than one offset — a Unity asset with repeated
            // blocks does, and the live `resources.assets` has five chunk ids
            // that appear two or three times each. Keying the reference on
            // `(role, path)` alone kept only the first of them, so the index
            // described a file with holes in it and the bundle could not be
            // rebuilt from its own blobs.
            let already = w
                .references
                .iter()
                .any(|r| r.role == role && r.path == pair.path && r.offset == c.offset);
            if !already {
                w.references.push(Reference {
                    tag: tag.to_string(),
                    build_id: Some(build_id.to_string()),
                    capture_id: pair.capture_id.clone(),
                    path: pair.path.clone(),
                    role,
                    offset: c.offset,
                });
            }
            if !w.sets.contains(&role) {
                w.sets.push(role);
            }
        };

        if let Some(o) = &pair.old {
            for c in &o.entry.chunks {
                push(&o.tag, &o.build_id, c, Set::MaterializeOld);
            }
        }
        if let Some(n) = &pair.new {
            let r = retention(pair);
            // A set, not `r.reuse.contains(&c.id)` per chunk: that was a linear
            // scan with a 49-byte string compare, once per new chunk — quadratic
            // per file. `reuse` stays a `Vec` because its order is recorded.
            let reusable: std::collections::HashSet<&str> =
                r.reuse.iter().map(String::as_str).collect();
            for c in &n.entry.chunks {
                push(&n.tag, &n.build_id, c, Set::MaterializeNew);
                if !reusable.contains(c.id.as_str()) {
                    push(&n.tag, &n.build_id, c, Set::PatchFetch);
                }
            }
        }
    }
    // Newest tag first, so a blob whose only old-build source has gone away is
    // still fetchable. Stable: two tags in the same order always sort the same.
    let mut out: Vec<Want> = by_id.into_values().collect();
    for w in &mut out {
        w.sources.sort_by(|a, b| b.tag.cmp(&a.tag));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash;

    /// A chunk whose content is `payload`, at `offset`.
    fn chunk(payload: &[u8], offset: i64, prefix: u64) -> SophonChunk {
        let raw = zstd::encode_all(payload, 1).expect("encode");
        SophonChunk {
            id: format!("{prefix:016x}_{}", hash::md5_hex(payload)),
            uncompressed_md5: hash::md5_hex(payload),
            offset,
            compressed_size: raw.len() as i64,
            uncompressed_size: payload.len() as i64,
            unknown_field6: 7,
            compressed_md5: hash::md5_hex(&raw),
        }
    }

    fn entry(path: &str, chunks: Vec<SophonChunk>) -> SophonChunkFile {
        let size = chunks.iter().map(|c| c.uncompressed_size).sum();
        SophonChunkFile {
            file: path.into(),
            chunks,
            is_folder: false,
            size,
            md5: "ff".repeat(16),
        }
    }

    /// The measured shape: three chunks old, three new, one shared, two new
    /// ones — five distinct chunk ids in all.
    fn hoyo() -> FilePair {
        FilePair {
            capture_id: "kept_in_place".into(),
            path: "GenshinImpact_Data/Plugins/HoYoChannel.dll".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: entry(
                    "dll",
                    vec![
                        chunk(b"old-zero", 0, 0x7d3f_0504_4383_7e58),
                        chunk(b"shared", 10, 0xf5dc_3ca1_f3a0_ba3d),
                        chunk(b"old-two", 20, 0xa431_4f3c_1ba1_d7d4),
                    ],
                ),
            }),
            new: Some(NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: entry(
                    "dll",
                    vec![
                        chunk(b"new-zero-much-longer", 0, 0xddb3_1a9f_d853_46c4),
                        chunk(b"shared", 10, 0xf5dc_3ca1_f3a0_ba3d),
                        chunk(b"new-two", 20, 0x340c_a86e_e8ac_9be0),
                    ],
                ),
            }),
        }
    }

    /// One `Want` per chunk id — which the store's tmp-file naming assumes.
    ///
    /// The `HoYoChannel.dll` fixture is the right one to assert it on: three old
    /// ids, two new-only and one **shared between the two builds**, so a dedup
    /// that stopped working would produce a duplicate id here rather than only in
    /// a case nobody would think to test.
    #[test]
    fn every_want_carries_its_own_chunk_id() {
        let wants = wants(&[hoyo()]);
        let ids: Vec<&str> = wants.iter().map(|w| w.chunk.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            ids.len(),
            "wants() must return one Want per chunk id; got {} wants over {} ids: {ids:?}",
            ids.len(),
            sorted.len(),
        );
        assert_eq!(
            ids.len(),
            5,
            "the fixture must produce the five distinct ids it is documented to, \
             or this test is not about the case it claims"
        );
    }
    // ---------------------------------------------------------------- sets

    #[test]
    fn the_three_sets_hold_exactly_what_they_say_they_hold() {
        let p = hoyo();
        let r = retention(&p);
        assert_eq!(r.ids(Set::MaterializeOld).len(), 3);
        assert_eq!(r.ids(Set::MaterializeNew).len(), 3);
        assert_eq!(
            r.ids(Set::PatchFetch).len(),
            2,
            "the two new chunks whose content is not in this old file"
        );
        assert_eq!(r.fetch.len(), 2);
        assert_eq!(
            r.reuse.len(),
            1,
            "the one chunk that really is the same bytes"
        );
    }

    #[test]
    fn patch_fetch_is_a_subset_of_materialize_new_and_disjoint_from_reuse() {
        let r = retention(&hoyo());
        assert!(r.a12_holds(), "{:?}", r.a12_failure());
        for id in r.ids(Set::PatchFetch) {
            assert!(
                r.ids(Set::MaterializeNew).contains(id),
                "{id} is not in materialize_new"
            );
            assert!(
                !r.reuse.contains(id),
                "{id} would be both reused and fetched"
            );
        }
    }

    /// Reuse is by *content*, not by chunk id — which is why the old and new
    /// builds here hand out different ids for different bytes at the same
    /// offset, and the same id only where the bytes really agree.
    #[test]
    fn reuse_is_keyed_on_content_not_on_the_chunk_id() {
        let p = hoyo();
        let r = retention(&p);
        let old0 = &p.old.as_ref().unwrap().entry.chunks[0];
        let new0 = &p.new.as_ref().unwrap().entry.chunks[0];
        assert_ne!(old0.id, new0.id, "different bytes, different ids");
        assert_ne!(
            old0.uncompressed_size, new0.uncompressed_size,
            "and different lengths: `old-zero` is 8 B and `new-zero-much-longer` is not"
        );
        assert_ne!(old0.uncompressed_md5, new0.uncompressed_md5);
        assert!(
            !r.reuse.contains(&new0.id),
            "a chunk that merely occupies the same offset is not a reuse"
        );
        let shared_old = &p.old.as_ref().unwrap().entry.chunks[1];
        let shared_new = &p.new.as_ref().unwrap().entry.chunks[1];
        assert_eq!(shared_old.id, shared_new.id);
        assert!(r.reuse.contains(&shared_new.id));
    }

    /// A same-content chunk published under a *different id prefix* is still the
    /// same bytes and must be reused. This is the case a key-on-id
    /// implementation gets wrong and a key-on-content one gets right, so it is
    /// pinned as its own test rather than folded into the one above.
    #[test]
    fn the_same_bytes_under_two_ids_are_still_a_reuse() {
        let a = chunk(b"identical", 0, 0x1111_1111_1111_1111);
        let b = chunk(b"identical", 9, 0x2222_2222_2222_2222);
        assert_ne!(a.id, b.id, "the prefix differs, so the ids differ");
        assert_eq!(a.uncompressed_md5, b.uncompressed_md5);
        let p = FilePair {
            capture_id: "c".into(),
            path: "f".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: entry("f", vec![a]),
            }),
            new: Some(NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: entry("f", vec![b.clone()]),
            }),
        };
        let r = retention(&p);
        assert!(
            r.reuse.contains(&b.id),
            "reused: {reused:?}",
            reused = r.reuse
        );
        assert!(r.fetch.is_empty(), "nothing to fetch");
        assert!(r.a12_holds());
    }

    /// `added` has no old file, so every chunk is fetched; `removed` has no new
    /// file, so `patch_fetch` is empty rather than falling back to the old one.
    #[test]
    fn a_one_sided_file_retains_only_what_exists() {
        let added = FilePair {
            capture_id: "added".into(),
            path: "new.dll".into(),
            old: None,
            new: Some(NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: entry("new.dll", vec![chunk(b"fresh", 0, 1), chunk(b"also", 5, 2)]),
            }),
        };
        let r = retention(&added);
        assert!(r.ids(Set::MaterializeOld).is_empty());
        assert_eq!(r.ids(Set::MaterializeNew).len(), 2);
        assert_eq!(r.ids(Set::PatchFetch).len(), 2, "nothing to reuse from");
        assert!(r.a12_holds());

        let removed = FilePair {
            capture_id: "removed".into(),
            path: "gone.blk".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: entry("gone.blk", vec![chunk(b"bye", 0, 1)]),
            }),
            new: None,
        };
        let r = retention(&removed);
        assert_eq!(r.ids(Set::MaterializeOld).len(), 1);
        assert!(r.ids(Set::MaterializeNew).is_empty());
        assert!(r.ids(Set::PatchFetch).is_empty());
        assert!(r.reuse.is_empty() && r.fetch.is_empty());
        assert!(r.a12_holds(), "vacuously: there is no patch to fetch for");
    }

    #[test]
    fn a_zero_reuse_file_fetches_every_new_chunk() {
        let p = FilePair {
            capture_id: "zero_reuse_multi".into(),
            path: "unity default resources".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: entry("f", vec![chunk(b"o1", 0, 1), chunk(b"o2", 1, 2)]),
            }),
            new: Some(NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: entry("f", vec![chunk(b"n1", 0, 3), chunk(b"n2", 1, 4)]),
            }),
        };
        let r = retention(&p);
        assert!(r.reuse.is_empty());
        assert_eq!(r.fetch.len(), 2);
        assert_eq!(r.ids(Set::PatchFetch).len(), 2);
        assert!(r.a12_holds());
    }

    // --------------------------------------------------------------- wants

    /// The measured acceptance: 3 old + 2 new-only = **5 distinct** chunk ids,
    /// with one shared between the two sides and therefore fetched once.
    #[test]
    fn the_hoyo_pair_needs_five_distinct_chunks() {
        let w = wants(&[hoyo()]);
        assert_eq!(w.len(), 5, "the shared chunk is fetched once, not twice");
        let shared = w
            .iter()
            .find(|x| x.chunk.uncompressed_md5 == hash::md5_hex(b"shared"))
            .expect("the shared chunk");
        assert_eq!(shared.conflicts, Vec::new(), "both builds agree on it");
        assert_eq!(
            shared
                .sources
                .iter()
                .map(|s| s.tag.as_str())
                .collect::<Vec<_>>(),
            vec!["7.1.0", "7.0.0"],
            "newest first: the old CDN is the one that may stop serving"
        );
        assert_eq!(shared.best_source(), Some("7.1.0"));
        assert_eq!(
            shared.sets,
            vec![Set::MaterializeOld, Set::MaterializeNew],
            "retained for both sides, fetched for neither"
        );
        assert_eq!(shared.references.len(), 2, "one per build");
        assert_eq!(
            shared.references.iter().map(|r| r.role).collect::<Vec<_>>(),
            vec![Set::MaterializeOld, Set::MaterializeNew]
        );
        assert_eq!(
            shared
                .references
                .iter()
                .map(|r| r.tag.as_str())
                .collect::<Vec<_>>(),
            vec!["7.0.0", "7.1.0"]
        );
        // Each reference carries the build its chunk is addressed under, and the
        // two differ: the shared chunk has one id and two addresses, which is why
        // `tag` alone cannot stand in for it.
        assert_eq!(
            shared
                .references
                .iter()
                .map(|r| r.build_id.as_deref().unwrap_or("?"))
                .collect::<Vec<_>>(),
            vec!["3XnPviMfoILi", "sMXGW2ll3Fuu"],
            "the shared chunk is addressed under two different build ids"
        );
    }

    /// A document written before `build_id` existed still reads, as `None`.
    ///
    /// This is the back-compat the field is additive for: bundles captured
    /// earlier are already on disk, and `verify` reads whatever is there. A
    /// consumer that needs the build id falls back to `MANIFEST.json` →
    /// `versions.{old,new}.build_id` matched on `tag`.
    #[test]
    fn a_reference_written_before_build_id_existed_still_reads() {
        let old_document = r#"{
            "tag": "7.0.0",
            "capture_id": "kept_in_place",
            "path": "dll",
            "role": "materialize_old",
            "offset": 0
        }"#;
        let r: Reference = serde_json::from_str(old_document).expect("an old document still reads");
        assert_eq!(r.tag, "7.0.0");
        assert_eq!(r.build_id, None, "absent, not an empty string");

        // And a current one round-trips through the same type.
        let now = Reference {
            tag: "7.0.0".into(),
            build_id: Some("3XnPviMfoILi".into()),
            capture_id: "kept_in_place".into(),
            path: "dll".into(),
            role: Set::MaterializeOld,
            offset: 0,
        };
        let v: serde_json::Value = serde_json::to_value(&now).unwrap();
        assert_eq!(v["build_id"], "3XnPviMfoILi");
        assert_eq!(serde_json::from_value::<Reference>(v).unwrap(), now);
    }

    /// The whole point of the shared-chunk case: a chunk in both builds is one
    /// blob on disk, listed under both roles, and counted once against the CDN.
    #[test]
    fn a_chunk_in_both_builds_is_one_blob_with_two_sources() {
        let w = wants(&[hoyo()]);
        let from_old_only: Vec<&Want> = w.iter().filter(|x| x.sources.len() == 1).collect();
        assert_eq!(
            from_old_only.len(),
            4,
            "every chunk but the shared one has one source"
        );
        let two_source = w.iter().filter(|x| x.sources.len() == 2).count();
        assert_eq!(two_source, 1);
        // And the patch_fetch set names two of the five, never the shared one.
        let shared_id = w
            .iter()
            .find(|x| x.chunk.uncompressed_md5 == hash::md5_hex(b"shared"))
            .expect("the shared chunk")
            .chunk
            .id
            .clone();
        let fetch: Vec<&str> = w
            .iter()
            .filter(|x| x.sets.contains(&Set::PatchFetch))
            .map(|x| x.chunk.id.as_str())
            .collect();
        assert_eq!(fetch.len(), 2);
        assert!(
            !fetch.contains(&shared_id.as_str()),
            "the reusable chunk is never fetched: {fetch:?}"
        );
    }

    /// Two builds publishing the same id for *different* content is a
    /// contradiction, and the store must record it rather than pick a winner —
    /// a blob measured green against whichever declaration came first would hide
    /// it from the selfcheck entirely.
    #[test]
    fn two_builds_disagreeing_about_one_id_is_recorded_as_a_conflict() {
        let mut old_c = chunk(b"old-bytes", 0, 0xfeed);
        old_c.id = "sameid_0000000000000000000000000000000000".into();
        let mut new_c = chunk(b"different-bytes-entirely", 0, 0xfeed);
        new_c.id = old_c.id.clone();
        assert_ne!(old_c.uncompressed_md5, new_c.uncompressed_md5);

        let p = FilePair {
            capture_id: "c".into(),
            path: "f".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: entry("f", vec![old_c]),
            }),
            new: Some(NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: entry("f", vec![new_c.clone()]),
            }),
        };
        let w = wants(&[p]);
        assert_eq!(w.len(), 1, "one id, so one blob — and one of them is wrong");
        // The *old* side is visited first, so its declaration is the one the
        // store measures against and 7.1.0 is the one recorded as disagreeing.
        assert_eq!(w[0].sources.len(), 1, "and no fallback source to trust");
        assert_eq!(w[0].sources[0].tag, "7.0.0");
        assert_eq!(w[0].best_source(), Some("7.0.0"));
        assert_eq!(w[0].conflicts.len(), 1);
        assert_eq!(w[0].conflicts[0].tag, "7.1.0");
        assert_eq!(w[0].conflicts[0].uncompressed_md5, new_c.uncompressed_md5);
        let d = w[0].describe_conflicts();
        assert!(d.contains("7.1.0") && d.contains(&new_c.id), "{d}");
    }

    #[test]
    fn a_chunk_wanted_by_two_files_is_stored_once_with_two_references() {
        let shared = chunk(b"shared-by-two", 0, 0xabcd);
        let mk = |path: &str| FilePair {
            capture_id: path.into(),
            path: path.into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: entry(path, vec![shared.clone()]),
            }),
            new: None,
        };
        let w = wants(&[mk("a.bin"), mk("b.bin")]);
        assert_eq!(w.len(), 1, "one blob, one fetch");
        assert_eq!(w[0].references.len(), 2);
        let paths: Vec<&str> = w[0].references.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(paths, vec!["a.bin", "b.bin"], "and both are named");
        assert_eq!(w[0].references[0].offset, 0);
        assert!(
            w[0].references
                .iter()
                .all(|r| r.role == Set::MaterializeOld)
        );
    }

    /// The **same chunk** at two offsets in one file is one blob with **two
    /// references**, not one.
    ///
    /// A file may place the same content at more than one offset — a Unity
    /// asset with repeated blocks does, and the live `resources.assets` has
    /// five chunk ids that appear two or three times each. Keying the reference
    /// on `(role, path)` alone kept only the first offset, so the index
    /// described a file with holes in it and the bundle could not be rebuilt
    /// from its own blobs.
    #[test]
    fn a_chunk_at_two_offsets_in_one_file_gets_two_references() {
        let shared = chunk(b"repeated", 0, 0xabcd);
        let mut second = shared.clone();
        second.offset = 512;
        let pair = FilePair {
            capture_id: "c".into(),
            path: "repeated.bin".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: entry("repeated.bin", vec![shared.clone(), second.clone()]),
            }),
            new: None,
        };
        let w = wants(&[pair]);
        assert_eq!(w.len(), 1, "one blob, one fetch");
        assert_eq!(
            w[0].references.len(),
            2,
            "but two placements: the content is at two offsets"
        );
        let offsets: Vec<i64> = w[0].references.iter().map(|r| r.offset).collect();
        assert_eq!(offsets, vec![0, 512]);
    }

    #[test]
    fn wants_are_in_a_deterministic_order() {
        let ids =
            |p: &[FilePair]| -> Vec<String> { wants(p).into_iter().map(|w| w.chunk.id).collect() };
        let alone = ids(&[hoyo()]);
        assert_eq!(alone, ids(&[hoyo()]), "the same inputs, the same order");
        // Adding a pair adds exactly its own chunk, and does not reshuffle.
        let removed_id = removal_pair().old.unwrap().entry.chunks[0].id.clone();
        let both = ids(&[hoyo(), removal_pair()]);
        assert_eq!(both.len(), 6);
        let mut sorted = both.clone();
        sorted.sort_unstable();
        assert_eq!(
            both, sorted,
            "sorted by chunk id, so insertion order cannot leak in"
        );
        assert!(both.contains(&removed_id));
        // And the order does not depend on which pair is visited first.
        assert_eq!(both, ids(&[removal_pair(), hoyo()]));
    }

    fn removal_pair() -> FilePair {
        FilePair {
            capture_id: "removed".into(),
            path: "gone.blk".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: entry("gone.blk", vec![chunk(b"only-in-old", 0, 0xdead)]),
            }),
            new: None,
        }
    }

    #[test]
    fn every_want_carries_at_least_one_set_and_the_chunk_it_is_a_want_for() {
        for w in wants(&[hoyo()]) {
            assert!(w.is_wanted(), "{}", w.chunk.id);
            assert!(!w.references.is_empty(), "{}", w.chunk.id);
            assert!(w.best_source().is_some(), "{}", w.chunk.id);
            for r in &w.references {
                assert!(Set::ALL.contains(&r.role));
            }
            assert!(w.chunk.compressed_size > 0, "a declared chunk");
            assert!(w.conflicts.is_empty(), "the measured pair has no conflicts");
        }
    }

    #[test]
    fn set_spellings_are_the_ones_the_index_documents() {
        assert_eq!(Set::MaterializeOld.id(), "materialize_old");
        assert_eq!(Set::MaterializeNew.id(), "materialize_new");
        assert_eq!(Set::PatchFetch.id(), "patch_fetch");
        let json: Vec<String> = Set::ALL
            .iter()
            .map(|s| serde_json::to_string(s).unwrap())
            .collect();
        assert_eq!(
            json,
            vec![
                "\"materialize_old\"",
                "\"materialize_new\"",
                "\"patch_fetch\""
            ]
        );
    }
}
