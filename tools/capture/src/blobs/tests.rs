use super::*;
use crate::hash;
use crate::manifest::proto::{SophonChunk, SophonChunkFile};

fn scratch(name: &str) -> girpr_tempdir::TempDir {
    girpr_tempdir::TempDir::new("gircap-blobs", name)
}

/// `fs::rename` **does** replace an existing destination on Windows: it calls
/// `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`. This is checked rather
/// than assumed, because "rename fails when the target exists" is a common
/// belief and acting on it here would mean deleting the atomic step the code
/// needs. The destination is deliberately *longer* than the source, so a
/// copy-not-replace implementation would leave a tail behind and be caught.
#[test]
fn renaming_a_blob_over_an_existing_one_replaces_it() {
    let dir = scratch("rename-over");
    let (final_path, tmp) = (dir.join("blob.zst"), dir.join(".blob.zst.tmp"));
    std::fs::write(&final_path, b"the older and longer blob").expect("seed destination");
    std::fs::write(&tmp, b"NEW").expect("write tmp");

    std::fs::rename(&tmp, &final_path).expect("rename onto an existing file");

    assert_eq!(
        std::fs::read(&final_path).expect("read"),
        b"NEW",
        "the destination must be the new blob and nothing else"
    );
    assert!(
        !tmp.exists(),
        "the tmp file must be gone after the rename publishes it"
    );
}

#[test]
fn two_wants_for_one_chunk_id_would_not_share_a_temp_file() {
    // The store half of the tmp-naming invariant: derived from the chunk id
    // alone, two tasks wanting the same id would write one file interleaved
    // and the first `rename` would publish a **half-written blob** — silent
    // corruption, with every digest in `chunks/index.json` quietly describing
    // the wrong bytes and nothing failing.
    //
    // Fed the *same* id twice on purpose, because that is the only input that
    // distinguishes a slot-suffixed name from an id-derived one.
    let dir = scratch("tmp-per-slot");
    let store = Store::new(
        &dir,
        Sources::from_pairs([]),
        crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
    );
    let (a, at) = store.paths_for("chunks/blobs/dead_beef.zst", 0);
    let (b, bt) = store.paths_for("chunks/blobs/dead_beef.zst", 1);

    assert_eq!(
        a, b,
        "the published path is the blob's, and does not depend on the slot"
    );
    assert_ne!(
        at, bt,
        "two tasks writing the same blob must not share one tmp file"
    );
    assert!(
        at.parent() == a.parent(),
        "and the tmp file stays beside the blob it becomes, not somewhere else"
    );
    for t in [&at, &bt] {
        let name = t.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            name.starts_with('.') && name.ends_with(".tmp"),
            "{name} must stay hidden and temporary"
        );
    }
}

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

/// Serve one HTTP response over loopback and close the connection.
///
/// `declared` is the `Content-Length` promised; `body` is what is actually
/// written. A body shorter than the declaration is a truncated CDN read.
fn serve_blob_once(declared: usize, body: Vec<u8>) -> (u16, std::thread::JoinHandle<()>) {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).expect("nonblocking accept");
    let handle = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(pair) => break pair,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => panic!("accept: {e}"),
            }
        };
        // The accept loop needs a nonblocking listener so a regression that
        // never connects cannot hang the suite. An accepted socket on Windows
        // can inherit that, and a mock that reads before the client has sent
        // its request would then answer and close mid-send. Back to blocking
        // so the request is read first and the reply cannot race it.
        stream.set_nonblocking(false).expect("blocking stream");
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).ok();
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                break;
            }
        }
        let head =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n");
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
        stream.flush().unwrap();
    });
    (port, handle)
}

/// The two builds' files declaring the same chunk, which gives the want its
/// newest-first sources: `7.1.0` then `7.0.0`.
fn pair_declared_by_both(c: &SophonChunk, capture_id: &str) -> FilePair {
    FilePair {
        capture_id: capture_id.into(),
        path: "dll".into(),
        old: Some(chunks::OldFile {
            tag: "7.0.0".into(),
            build_id: "3XnPviMfoILi".into(),
            entry: entry("dll", vec![c.clone()]),
        }),
        new: Some(chunks::NewFile {
            tag: "7.1.0".into(),
            build_id: "sMXGW2ll3Fuu".into(),
            entry: entry("dll", vec![c.clone()]),
        }),
    }
}

/// Every file under `chunks/blobs/`, sorted, so a leftover temp or losing
/// candidate fails the test by name.
fn blob_dir_names(root: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join("chunks/blobs"))
        .expect("the blob dir was created by the first attempt")
        .map(|entry| {
            entry
                .expect("read a dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// `blobs` is documented as ordered by id, which is what makes the lookup a
/// binary search. A bundle is a file someone can edit and `blobs` is a `pub`
/// field, so the lookup has to stay correct for an index that is *not*
/// sorted — otherwise a hand-edited bundle turns "found" into "not found",
/// and the failure reads like a missing blob rather than a corrupt index.
#[test]
fn a_blob_is_found_whether_or_not_the_index_is_sorted() {
    let ids = ["aaa", "mmm", "zzz"];
    let blob = |id: &str| IndexedBlob {
        props: BlobProps {
            chunk_id: id.to_string(),
            stored_path: None,
            bytes: 0,
            container: String::new(),
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
    };
    let build = |order: &[&str]| {
        let entries: Vec<(IndexedBlob, Vec<Set>)> = order
            .iter()
            .map(|id| (blob(id), vec![Set::MaterializeOld]))
            .collect();
        ChunkIndex::build(entries)
    };
    let sorted = build(&ids);
    let reversed = build(&ids.iter().rev().copied().collect::<Vec<_>>());
    assert_eq!(
        sorted
            .blobs
            .iter()
            .map(|b| &b.props.chunk_id)
            .collect::<Vec<_>>(),
        ids,
        "build orders by id, which is what the lookup assumes"
    );
    for id in ids {
        assert_eq!(
            sorted.get(id).map(|b| b.props.chunk_id.as_str()),
            Some(id),
            "found in a sorted index"
        );
        assert_eq!(
            reversed.get(id).map(|b| b.props.chunk_id.as_str()),
            Some(id),
            "and still found in one that is not sorted"
        );
    }
    assert!(reversed.get("no-such-blob").is_none());
}

fn pair() -> FilePair {
    FilePair {
        capture_id: "reuse_in_place".into(),
        path: "dll".into(),
        old: Some(chunks::OldFile {
            tag: "7.0.0".into(),
            build_id: "3XnPviMfoILi".into(),
            entry: entry(
                "dll",
                vec![
                    chunk(b"old-zero", 0, 0x7d3f),
                    chunk(b"shared", 10, 0xf5dc),
                    chunk(b"old-two", 20, 0xa431),
                ],
            ),
        }),
        new: Some(chunks::NewFile {
            tag: "7.1.0".into(),
            build_id: "sMXGW2ll3Fuu".into(),
            entry: entry(
                "dll",
                vec![
                    chunk(b"new-zero", 0, 0xddb3),
                    chunk(b"shared", 10, 0xf5dc),
                    chunk(b"new-two", 20, 0x340c),
                ],
            ),
        }),
    }
}

fn sources() -> Sources {
    Sources::from_pairs([
        ("7.0.0".to_string(), "https://cdn/old".to_string()),
        ("7.1.0".to_string(), "https://cdn/new".to_string()),
    ])
}

/// **A capture's sides come from its own field.**
///
/// The audio manifests widen the *selection pool*, so a capture can be an
/// audio file. Its sides are then in the audio manifests, and looking them
/// up in the `game` manifests instead would find nothing: the pair would
/// come out with **no sides at all**, and the capture would assert nothing
/// — a patchmap that says "this file must be gone afterwards" for a file
/// that is simply there.
#[test]
fn a_capture_from_another_field_gets_its_sides_from_that_field() {
    use crate::select::{Admission, Selection};

    let audio = |tag: &str, build: &str, path: &str| CapturedManifest {
        tag: tag.into(),
        build_id: build.into(),
        entries: vec![entry(path, vec![chunk(b"vo", 0, 0x1234)])],
        ..CapturedManifest::empty(tag, "en-us", vec![])
    };
    let manifests = vec![
        // The `game` field: a different path entirely.
        vec![
            CapturedManifest {
                tag: "7.0.0".into(),
                build_id: "gameold".into(),
                entries: vec![entry("game/other.dat", vec![chunk(b"x", 0, 1)])],
                ..CapturedManifest::empty("7.0.0", "game", vec![])
            },
            CapturedManifest {
                tag: "7.1.0".into(),
                build_id: "gamenew".into(),
                entries: vec![entry("game/other.dat", vec![chunk(b"x", 0, 1)])],
                ..CapturedManifest::empty("7.1.0", "game", vec![])
            },
        ],
        // The `en-us` field: the file the capture is about.
        vec![
            audio("7.0.0", "audiold", "vo.dat"),
            audio("7.1.0", "audinew", "vo.dat"),
        ],
    ];
    let selection = Selection {
        admitted: vec![Admission {
            capture_id: "identical".into(),
            primary_class: crate::select::Class::Identical,
            path: "vo.dat".into(),
            matching_field: "en-us".into(),
            state: crate::manifest::diff::State::Identical,
            also_satisfies: Vec::new(),
            budget_cost: 10,
            patch_bytes: 0,
        }],
        rejected: Vec::new(),
        forced_skipped: Vec::new(),
        forced_unscored: Vec::new(),
        matrix: Vec::new(),
        bytes_admitted: 10,
        budget: crate::select::Budget::unlimited(),
    };
    let pairs = file_pairs(
        &selection,
        &["game".into(), "en-us".into()],
        &manifests,
        "7.0.0",
        "7.1.0",
    );
    assert_eq!(pairs.len(), 1);
    let p = &pairs[0];
    assert!(p.old.is_some(), "the old side is in the audio manifest");
    assert!(p.new.is_some(), "and so is the new one");
    assert_eq!(p.old.as_ref().expect("an old side").build_id, "audiold");
    assert_eq!(p.new.as_ref().expect("a new side").build_id, "audinew");
}

#[test]
fn the_index_role_lists_are_derived_from_the_blobs_that_carry_them() {
    let w = chunks::wants(&[pair()]);
    let entries: Vec<(IndexedBlob, Vec<Set>)> = w
        .iter()
        .map(|x| {
            (
                IndexedBlob {
                    props: BlobProps::unmeasured(&x.chunk),
                    referenced_by: x.references.clone(),
                    sets: x.sets.clone(),
                },
                x.sets.clone(),
            )
        })
        .collect();
    let idx = ChunkIndex::build(entries);
    assert_eq!(idx.schema_version, crate::SCHEMA_VERSION);
    assert_eq!(idx.sets.len(), 3);
    assert_eq!(idx.sets["materialize_old"].len(), 3);
    assert_eq!(idx.sets["materialize_new"].len(), 3);
    assert_eq!(idx.sets["patch_fetch"].len(), 2);
    // Every id in a role list is a blob in the index, and vice versa: a list
    // naming an absent blob would send M7 looking for bytes that are not there.
    for (role, ids) in &idx.sets {
        for id in ids {
            let b = idx
                .get(id)
                .unwrap_or_else(|| panic!("{role} names a missing blob {id}"));
            assert!(
                b.sets.iter().any(|s| s.id() == role),
                "{id} is in the {role} list but its entry does not say so"
            );
        }
    }
    for b in &idx.blobs {
        for s in &b.sets {
            assert!(
                idx.sets[s.id()].contains(&b.props.chunk_id),
                "{} claims {} but the list omits it",
                b.props.chunk_id,
                s.id()
            );
        }
    }
}

#[test]
fn the_index_lists_blobs_sorted_by_id_so_two_runs_agree() {
    let w = chunks::wants(&[pair()]);
    let entries = w
        .iter()
        .map(|x| {
            (
                IndexedBlob {
                    props: BlobProps::unmeasured(&x.chunk),
                    referenced_by: x.references.clone(),
                    sets: x.sets.clone(),
                },
                x.sets.clone(),
            )
        })
        .collect();
    let ids: Vec<String> = ChunkIndex::build(entries)
        .blobs
        .iter()
        .map(|b| b.props.chunk_id.clone())
        .collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
}

#[test]
fn a_url_needs_a_registered_tag_and_says_which_one_it_wanted() {
    let s = sources();
    assert_eq!(
        s.url("7.1.0", "abc").unwrap(),
        "https://cdn/new/abc",
        "the base already carries the build id"
    );
    let e = s.url("6.7.0", "abc").unwrap_err().to_string();
    assert!(e.contains("6.7.0"), "{e}");
    assert_eq!(s.tags(), vec!["7.0.0", "7.1.0"]);
}

#[test]
fn plan_only_still_lists_every_chunk_and_its_roles() {
    let w = chunks::wants(&[pair()]);
    let entries = Store::plan_only(&w);
    assert_eq!(entries.len(), 5);
    assert!(entries.iter().all(|(b, _)| b.props.stored_path.is_none()));
    assert!(entries.iter().all(|(b, _)| b.props.verified.is_none()));
    let idx = ChunkIndex::build(entries);
    assert_eq!(idx.sets["patch_fetch"].len(), 2, "the plan survives");
}

#[test]
fn the_projected_download_is_once_per_distinct_chunk() {
    let w = chunks::wants(&[pair()]);
    let sum: u64 = w.iter().map(|x| x.chunk.compressed_size as u64).sum();
    assert_eq!(projected_download(&w), sum);
    assert_eq!(w.len(), 5, "three old ids plus two new-only ones");
}

/// **K1.** The projection is a *total*, not an address: a manifest that
/// declares `i64::MAX` on several chunks saturates instead of aborting the
/// run or wrapping to a small, plausible cost.
#[test]
fn a_hostile_projected_download_saturates() {
    let mut w = chunks::wants(&[pair()]);
    for want in &mut w {
        want.chunk.compressed_size = i64::MAX;
    }
    assert_eq!(
        projected_download(&w),
        u64::MAX,
        "five maxima cannot be represented, so the answer is 'more than this'"
    );
}

/// **K1.** The index is a document someone may have edited. A reference at
/// `i64::MAX` is refused with the blob, path and arithmetic named, before
/// `Bundle::open` lets any command add it to a length.
#[test]
fn a_crafted_index_reference_offset_is_refused_with_its_values() {
    let w = chunks::wants(&[pair()]);
    let mut entries = Store::plan_only(&w);
    let blob = &mut entries[0].0;
    let id = blob.props.chunk_id.clone();
    blob.referenced_by[0].offset = i64::MAX;
    let idx = ChunkIndex::build(entries);
    let e = idx.validate_arithmetic().expect_err("overflowing offset");
    assert!(e.contains(&id), "names the blob: {e}");
    assert!(
        e.contains(&i64::MAX.to_string()) && e.contains("overflows i64"),
        "names the value and the arithmetic: {e}"
    );
}

/// **K1.** An `uncompressed_size` above `i64::MAX` cannot be a length; the
/// index refuses it rather than letting a `u64 as i64` cast turn it negative.
#[test]
fn an_index_size_that_does_not_fit_i64_is_refused() {
    let w = chunks::wants(&[pair()]);
    let mut entries = Store::plan_only(&w);
    let id = entries[0].0.props.chunk_id.clone();
    entries[0].0.props.uncompressed_size = u64::MAX;
    let idx = ChunkIndex::build(entries);
    let e = idx.validate_arithmetic().expect_err("unrepresentable size");
    assert!(e.contains(&id), "names the blob: {e}");
    assert!(
        e.contains(&u64::MAX.to_string()) && e.contains("does not fit i64"),
        "names the value and the bound: {e}"
    );
}

#[test]
fn a_description_names_the_numbers_a_reader_wants_first() {
    let w = chunks::wants(&[pair()]);
    let idx = ChunkIndex::build(Store::plan_only(&w));
    let d = describe(&w, &idx);
    assert!(d.contains("5 distinct chunks"), "{d}");
    assert!(d.contains("0 failing A6"), "{d}");
    // Nothing was stored, and the line must not claim otherwise.
    assert!(d.contains("0 of 5 stored"), "{d}");
    assert!(!d.contains(", 5 stored,"), "{d}");
}

/// And with every blob written the same line says all five.
#[test]
fn a_description_counts_what_was_actually_stored() {
    let w = chunks::wants(&[pair()]);
    let entries: Vec<(IndexedBlob, Vec<Set>)> = w
        .iter()
        .map(|x| {
            let mut props = BlobProps::unmeasured(&x.chunk);
            props.stored_path = Some(BlobProps::stored_path_for(&x.chunk.id));
            (
                IndexedBlob {
                    props,
                    referenced_by: x.references.clone(),
                    sets: x.sets.clone(),
                },
                x.sets.clone(),
            )
        })
        .collect();
    let d = describe(&w, &ChunkIndex::build(entries));
    assert!(d.contains("5 of 5 stored"), "{d}");
}

/// A blob that was never fetched is not silently absent from the index, and
/// its row carries **why** in the structured field version 3 added.
#[test]
fn an_unfilled_want_still_has_a_row_and_carries_the_fetch_failure() {
    let w = chunks::wants(&[pair()]);
    let root = girpr_tempdir::TempDir::new("gircap-m6", "unfilled_want");
    let store = Store::new(
        &root,
        sources(),
        crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
    );
    let failure = FetchError {
        source: Some("7.1.0".into()),
        url: Some("https://cdn/new/old-zero".into()),
        status: Some(404),
        transport: None,
        attempts: 1,
        detail: "old-zero via 7.1.0: HTTP 404 Not Found".into(),
    };
    let (entry, roles) = store.unfilled(&w[0], &failure);
    assert_eq!(roles, w[0].sets);
    assert!(entry.props.stored_path.is_none());
    assert!(
        entry.props.decode_error.is_none(),
        "no bytes arrived, so there is nothing to decode"
    );
    assert_eq!(
        entry.props.fetch_error.as_ref(),
        Some(&failure),
        "the row must carry the structured fetch failure"
    );
    assert!(!entry.props.is_usable());
}

/// **One failed fetch is one failure.** The slot's index and the error are
/// returned together, so the row can carry the reason and the run's failed
/// count cannot see the same chunk twice.
///
/// Before this, `Store::one` threw the slot away on failure: `fill` pushed
/// the error, the `None` slot looked like a task that never reported, and a
/// second bare chunk id was pushed for it. One failure, two entries, and a
/// row that could only say "not stored".
#[tokio::test]
async fn a_chunk_the_cdn_will_not_serve_is_one_failure_with_its_reason_in_the_row() {
    let w = chunks::wants(&[pair()]);
    let root = girpr_tempdir::TempDir::new("gircap-m6", "one_failed_fetch");
    // Port 0 is never bound: every attempt is an immediate refusal, with no
    // network, no timeout and no retry (one attempt is the whole policy).
    let dead = "http://127.0.0.1:0".to_string();
    let store = Store::new(
        &root,
        Sources::from_pairs([
            ("7.0.0".to_string(), dead.clone()),
            ("7.1.0".to_string(), dead),
        ]),
        crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
    );
    let fetcher = crate::http::Fetcher::new(
        crate::http::Transport::local(),
        crate::http::RequestLog::new(),
    )
    .expect("a fetcher with no configured headers to reject");

    let (entries, errors) = store.fill(&fetcher, &w, 1).await;

    assert_eq!(
        errors.len(),
        w.len(),
        "one entry per failed chunk, not one plus its bare id: {errors:?}"
    );
    assert_eq!(entries.len(), w.len(), "every want still has a row");
    for (entry, want) in entries.iter().zip(&w) {
        assert_eq!(entry.0.props.chunk_id, want.chunk.id);
        assert!(entry.0.props.stored_path.is_none());
        assert!(
            entry.0.props.decode_error.is_none(),
            "a fetch that produced no bytes has nothing to decode"
        );
        let why = entry
            .0
            .props
            .fetch_error
            .as_ref()
            .expect("the row says why it is absent");
        assert_ne!(
            why.detail, "not stored",
            "the generic message tells nobody why"
        );
        assert!(
            why.detail.starts_with(&want.chunk.id) && why.detail.contains(" via 7."),
            "the reason names the chunk and the source it last tried: {}",
            why.detail
        );
        assert!(
            why.transport.is_some(),
            "a refused connection is a transport failure: {why:?}"
        );
    }

    // F11: every one of those attempts created `chunks/blobs/.<id>.<slot>.tmp`
    // before the connection was refused. A leftover would be invisible to a
    // reader but permanent: the slot is part of the name, so a later run only
    // truncates it if the same id lands in the same slot again.
    let blobs = root.join("chunks/blobs");
    let leftovers: Vec<String> = std::fs::read_dir(&blobs)
        .expect("the blob dir was created before the first attempt")
        .map(|entry| {
            entry
                .expect("read a dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "a failed fetch must leave nothing behind: {leftovers:?}"
    );
}

/// **K2.** A fetched body that is not a zstd frame is evidence, not a failed
/// fetch: the row and the disk agree, and the run reports no error.
///
/// The body is served over loopback through the real [`Store::fill`] path so
/// this covers the rename, the measure-from-file step and the index entry,
/// not just [`BlobProps::measure_file`]. Before the fix the file stayed on
/// disk while the row said `bytes: 0`, `stored_path: null` and the run counted
/// a failed fetch — three contradictory accounts of one body.
#[tokio::test]
async fn an_undecodable_body_is_stored_and_measured_rather_than_a_failed_fetch() {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    let xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error>\n  \
                    <Code>NoSuchKey</Code>\n  <Message>The specified key does not \
                    exist.</Message>\n</Error>\n"
        .to_vec();

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).expect("nonblocking accept");
    let served = xml.clone();
    let server = std::thread::spawn(move || {
        // Bounded rather than a blocking `accept`: a future regression that
        // makes the fetch fail before connecting must fail the assertions,
        // not hang the suite on `join`.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(pair) => break pair,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => panic!("accept: {e}"),
            }
        };
        // The accept loop needs a nonblocking listener so a regression that
        // never connects cannot hang the suite. An accepted socket on Windows
        // can inherit that, and a mock that reads before the client has sent
        // its request would then answer and close mid-send. Back to blocking
        // so the request is read first and the reply cannot race it.
        stream.set_nonblocking(false).expect("blocking stream");
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).ok();
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                break;
            }
        }
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            served.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(&served).unwrap();
        stream.flush().unwrap();
    });

    let c = chunk(b"a chunk the CDN will not serve", 0, 0x5a5a);
    let p = FilePair {
        capture_id: "undecodable".into(),
        path: "dll".into(),
        old: None,
        new: Some(chunks::NewFile {
            tag: "7.1.0".into(),
            build_id: "sMXGW2ll3Fuu".into(),
            entry: entry("dll", vec![c.clone()]),
        }),
    };
    let w = chunks::wants(&[p]);
    assert_eq!(w.len(), 1, "the fixture asks for exactly one chunk");

    let root = girpr_tempdir::TempDir::new("gircap-k2", "undecodable_kept");
    let store = Store::new(
        &root,
        Sources::from_pairs([("7.1.0".to_string(), format!("http://127.0.0.1:{port}/cdn"))]),
        crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
    );
    let fetcher = crate::http::Fetcher::new(
        crate::http::Transport::local(),
        crate::http::RequestLog::new(),
    )
    .expect("a fetcher with no configured headers to reject");

    let (entries, errors) = store.fill(&fetcher, &w, 1).await;
    server.join().expect("the mock served the body");

    assert!(
        errors.is_empty(),
        "an unframed body is a measured row, not a failed fetch: {errors:?}"
    );
    assert_eq!(entries.len(), 1);
    let props = &entries[0].0.props;
    assert_eq!(
        props.bytes,
        xml.len() as u64,
        "the row counts the bytes that arrived, not zero"
    );
    assert_eq!(
        props.stored_path.as_deref(),
        Some(BlobProps::stored_path_for(&c.id).as_str()),
        "the row names the file the store published"
    );
    assert_eq!(props.md5_compressed, hash::md5_hex(&xml));
    assert_eq!(props.sha256_compressed, hash::sha256_hex(&xml));
    assert!(
        props
            .decode_error
            .as_deref()
            .unwrap_or_default()
            .contains("not a zstd frame"),
        "the row says why it will not decode: {:?}",
        props.decode_error
    );
    let v = props.verified.expect("the body was measured");
    assert!(!v.md5_decompressed_matches && !v.decoded_size_matches);

    let on_disk = std::fs::read(root.join(BlobProps::stored_path_for(&c.id)))
        .expect("the indexed path exists on disk");
    assert_eq!(
        on_disk, xml,
        "the published bytes are exactly what the CDN served"
    );
}

/// **A successful empty body is evidence, not a failed fetch.** A `2xx` whose
/// body is zero bytes still says something about the origin, so the empty file
/// is published and measured. The row keeps `stored_path`, `bytes: 0` and a
/// `decode_error`, and the run does not count the chunk among the failed
/// fetches. Only a *failed* fetch's zero-byte temp is removed.
#[tokio::test]
async fn a_successful_empty_body_is_stored_and_measured_as_undecodable() {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).expect("nonblocking accept");
    let server = std::thread::spawn(move || {
        // Bounded rather than a blocking `accept`: a future regression that
        // makes the fetch fail before connecting must fail the assertions,
        // not hang the suite on `join`.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(pair) => break pair,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => panic!("accept: {e}"),
            }
        };
        // The accept loop needs a nonblocking listener so a regression that
        // never connects cannot hang the suite. An accepted socket on Windows
        // can inherit that, and a mock that reads before the client has sent
        // its request would then answer and close mid-send. Back to blocking
        // so the request is read first and the reply cannot race it.
        stream.set_nonblocking(false).expect("blocking stream");
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).ok();
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                break;
            }
        }
        // A successful answer that simply carries nothing: the origin was
        // reached and said "no bytes", which is a fact worth a row.
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        stream.flush().unwrap();
    });

    let c = chunk(b"a chunk the CDN answers with an empty body", 0, 0x5c5c);
    let p = FilePair {
        capture_id: "empty".into(),
        path: "dll".into(),
        old: None,
        new: Some(chunks::NewFile {
            tag: "7.1.0".into(),
            build_id: "sMXGW2ll3Fuu".into(),
            entry: entry("dll", vec![c.clone()]),
        }),
    };
    let w = chunks::wants(&[p]);
    assert_eq!(w.len(), 1, "the fixture asks for exactly one chunk");

    let root = girpr_tempdir::TempDir::new("gircap-k2", "empty_body_kept");
    let store = Store::new(
        &root,
        Sources::from_pairs([("7.1.0".to_string(), format!("http://127.0.0.1:{port}/cdn"))]),
        crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
    );
    let fetcher = crate::http::Fetcher::new(
        crate::http::Transport::local(),
        crate::http::RequestLog::new(),
    )
    .expect("a fetcher with no configured headers to reject");

    let (entries, errors) = store.fill(&fetcher, &w, 1).await;
    server.join().expect("the mock served the empty body");

    assert!(
        errors.is_empty(),
        "a successful empty body is not a failed fetch: {errors:?}"
    );
    assert_eq!(entries.len(), 1, "the chunk still has a row");
    let props = &entries[0].0.props;
    assert_eq!(
        props.stored_path.as_deref(),
        Some(BlobProps::stored_path_for(&c.id).as_str()),
        "the empty body was published like any other"
    );
    assert_eq!(props.bytes, 0, "the row counts the bytes that arrived");
    assert!(
        !props.partial,
        "the fetch completed, so nothing is a prefix"
    );
    assert!(
        props.fetch_error.is_none(),
        "nothing failed to fetch: {:?}",
        props.fetch_error
    );
    assert!(
        props
            .decode_error
            .as_deref()
            .unwrap_or_default()
            .contains("not a zstd frame"),
        "the row says why it will not decode: {:?}",
        props.decode_error
    );
    assert!(
        !props.is_usable(),
        "zero bytes cannot be assembled into the chunk the manifest named"
    );

    let on_disk = std::fs::read(root.join(BlobProps::stored_path_for(&c.id)))
        .expect("the indexed path exists on disk");
    assert!(
        on_disk.is_empty(),
        "the published file is exactly the empty body: {} bytes",
        on_disk.len()
    );
}

/// **F11.** A fetch that stops after writing bytes publishes those bytes as
/// an indexed, unusable row: `partial: true`, a structured `fetch_error`,
/// and the digests of exactly what arrived. The bytes stay on disk so a
/// reader can compare them with a later capture.
#[tokio::test]
async fn a_truncated_response_keeps_its_prefix_and_marks_the_row_partial() {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    let payload = b"a chunk the CDN will stop serving half way through".repeat(64);
    let full = zstd::encode_all(payload.as_slice(), 1).expect("encode");
    let prefix_len = full.len() / 2;
    let prefix = full[..prefix_len].to_vec();
    let declared = full.len();

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).expect("nonblocking accept");
    let served = prefix.clone();
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(pair) => break pair,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => panic!("accept: {e}"),
            }
        };
        // The accept loop needs a nonblocking listener so a regression that
        // never connects cannot hang the suite. An accepted socket on Windows
        // can inherit that, and a mock that reads before the client has sent
        // its request would then answer and close mid-send. Back to blocking
        // so the request is read first and the reply cannot race it.
        stream.set_nonblocking(false).expect("blocking stream");
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).ok();
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                break;
            }
        }
        // The declaration promises the whole body; the connection closes
        // after the prefix, which is the real shape of a truncated CDN read.
        let head =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n");
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(&served).unwrap();
        stream.flush().unwrap();
    });

    let c = chunk(&payload, 0, 0x5b5b);
    let p = FilePair {
        capture_id: "truncated".into(),
        path: "dll".into(),
        old: None,
        new: Some(chunks::NewFile {
            tag: "7.1.0".into(),
            build_id: "sMXGW2ll3Fuu".into(),
            entry: entry("dll", vec![c.clone()]),
        }),
    };
    let w = chunks::wants(&[p]);
    let root = girpr_tempdir::TempDir::new("gircap-f11", "truncated_kept");
    let store = Store::new(
        &root,
        Sources::from_pairs([("7.1.0".to_string(), format!("http://127.0.0.1:{port}/cdn"))]),
        crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
    );
    let fetcher = crate::http::Fetcher::new(
        crate::http::Transport::local(),
        crate::http::RequestLog::new(),
    )
    .expect("a fetcher with no configured headers to reject");

    let (entries, errors) = store.fill(&fetcher, &w, 1).await;
    server.join().expect("the mock served the prefix");

    assert_eq!(errors.len(), 1, "the failed fetch is reported once");
    assert_eq!(entries.len(), 1, "and its bytes still have a row");
    let props = &entries[0].0.props;
    assert!(props.partial, "the row says the body is a prefix");
    assert!(props.stored_path.is_some(), "the prefix was published");
    assert_eq!(
        props.bytes, prefix_len as u64,
        "the row counts the bytes that arrived"
    );
    assert_eq!(props.md5_compressed, hash::md5_hex(&prefix));
    let failure = props.fetch_error.as_ref().expect("and why it stopped");
    assert!(
        failure.transport.is_some(),
        "a connection that closed early is a transport failure: {failure:?}"
    );
    assert!(
        failure.detail.contains("via 7.1.0"),
        "the source is named: {}",
        failure.detail
    );
    assert!(!props.is_usable(), "a prefix must never look complete");
    assert!(
        props
            .why_unusable()
            .unwrap_or_default()
            .contains("fetch failed"),
        "and the reason is the fetch, not a decode"
    );
    let on_disk = std::fs::read(root.join(BlobProps::stored_path_for(&c.id)))
        .expect("the indexed path exists");
    assert_eq!(on_disk, prefix, "the bytes are exactly what arrived");
}

/// **A truncated newest source does not end the loop.** The older source is
/// asked; its complete body wins outright, replaces the parked prefix and is
/// the only attempt that claims `stored_as`. The losing prefix leaves no file.
#[tokio::test]
async fn a_complete_older_source_wins_over_a_truncated_newer_one() {
    let payload = b"a chunk the newest CDN truncates but the older one serves".repeat(64);
    let full = zstd::encode_all(payload.as_slice(), 1).expect("encode");
    let prefix_len = full.len() / 2;
    let prefix = full[..prefix_len].to_vec();
    let (new_port, new_server) = serve_blob_once(full.len(), prefix);
    let (old_port, old_server) = serve_blob_once(full.len(), full.clone());

    let c = chunk(&payload, 0, 0xca11);
    let p = pair_declared_by_both(&c, "complete_fallback");
    let w = chunks::wants(&[p]);
    assert_eq!(
        w[0].sources
            .iter()
            .map(|s| s.tag.as_str())
            .collect::<Vec<_>>(),
        vec!["7.1.0", "7.0.0"],
        "the newest source is tried first"
    );
    let root = girpr_tempdir::TempDir::new("gircap-f11", "complete_fallback");
    let store = Store::new(
        &root,
        Sources::from_pairs([
            (
                "7.1.0".to_string(),
                format!("http://127.0.0.1:{new_port}/cdn"),
            ),
            (
                "7.0.0".to_string(),
                format!("http://127.0.0.1:{old_port}/cdn"),
            ),
        ]),
        crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
    );
    let fetcher = crate::http::Fetcher::new(
        crate::http::Transport::local(),
        crate::http::RequestLog::new(),
    )
    .expect("a fetcher with no configured headers to reject");

    let (entries, errors) = store.fill(&fetcher, &w, 1).await;
    new_server
        .join()
        .expect("the newest server served its prefix");
    old_server
        .join()
        .expect("the older server served the whole body");

    assert!(
        errors.is_empty(),
        "a complete fallback is a success, not a failed fetch: {errors:?}"
    );
    let props = &entries[0].0.props;
    let rel = BlobProps::stored_path_for(&c.id);
    assert!(!props.partial, "the complete body is not a prefix");
    assert!(
        props.fetch_error.is_none(),
        "nothing failed after the older source completed: {:?}",
        props.fetch_error
    );
    assert_eq!(props.stored_path.as_deref(), Some(rel.as_str()));
    assert!(
        props.is_usable(),
        "the whole chunk arrived and measured green"
    );
    assert_eq!(
        std::fs::read(root.join(&rel)).expect("the published blob"),
        full,
        "the complete body replaced the prefix"
    );

    let rows = fetcher.log().records();
    assert_eq!(rows.len(), 2, "both sources were asked: {rows:?}");
    let newer = rows
        .iter()
        .find(|r| r.url.contains(&format!(":{new_port}/")))
        .expect("the newest request is logged");
    let older = rows
        .iter()
        .find(|r| r.url.contains(&format!(":{old_port}/")))
        .expect("the older request is logged");
    assert_eq!(
        older.stored_as.as_deref(),
        Some(rel.as_str()),
        "only the complete winner claims the published blob"
    );
    assert_eq!(
        newer.stored_as, None,
        "the losing prefix must not claim the path"
    );
    assert_eq!(
        blob_dir_names(&root),
        vec![format!("{}.zst", c.id)],
        "no temp or candidate file survives"
    );
}

/// A partial is retried, and the longest prefix wins even when it came from
/// the older source. The row carries that attempt's `fetch_error` and bytes.
#[tokio::test]
async fn the_longest_prefix_wins_when_no_source_completes() {
    let payload = b"both CDNs truncate this chunk at different points".repeat(64);
    let full = zstd::encode_all(payload.as_slice(), 1).expect("encode");
    let newer_len = full.len() / 3;
    let older_len = full.len() * 2 / 3;
    let newer_prefix = full[..newer_len].to_vec();
    let older_prefix = full[..older_len].to_vec();
    let (new_port, new_server) = serve_blob_once(full.len(), newer_prefix);
    let (old_port, old_server) = serve_blob_once(full.len(), older_prefix.clone());

    let c = chunk(&payload, 0, 0xca12);
    let p = pair_declared_by_both(&c, "longest_older");
    let w = chunks::wants(&[p]);
    let root = girpr_tempdir::TempDir::new("gircap-f11", "longest_older");
    let store = Store::new(
        &root,
        Sources::from_pairs([
            (
                "7.1.0".to_string(),
                format!("http://127.0.0.1:{new_port}/cdn"),
            ),
            (
                "7.0.0".to_string(),
                format!("http://127.0.0.1:{old_port}/cdn"),
            ),
        ]),
        crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
    );
    let fetcher = crate::http::Fetcher::new(
        crate::http::Transport::local(),
        crate::http::RequestLog::new(),
    )
    .expect("a fetcher with no configured headers to reject");

    let (entries, errors) = store.fill(&fetcher, &w, 1).await;
    new_server
        .join()
        .expect("the newest server served its prefix");
    old_server
        .join()
        .expect("the older server served its prefix");

    assert_eq!(errors.len(), 1, "only the published failure is reported");
    let props = &entries[0].0.props;
    let rel = BlobProps::stored_path_for(&c.id);
    assert!(props.partial);
    assert_eq!(
        props.bytes, older_len as u64,
        "the longest prefix is the one published"
    );
    assert_eq!(props.stored_path.as_deref(), Some(rel.as_str()));
    let failure = props.fetch_error.as_ref().expect("and why it stopped");
    assert!(
        failure.detail.contains("via 7.0.0"),
        "the row names the source whose prefix won: {}",
        failure.detail
    );
    assert_eq!(
        std::fs::read(root.join(&rel)).expect("the published blob"),
        older_prefix,
        "the file is exactly the winning prefix"
    );

    let rows = fetcher.log().records();
    let newer = rows
        .iter()
        .find(|r| r.url.contains(&format!(":{new_port}/")))
        .expect("the newest request is logged");
    let older = rows
        .iter()
        .find(|r| r.url.contains(&format!(":{old_port}/")))
        .expect("the older request is logged");
    assert_eq!(
        older.stored_as.as_deref(),
        Some(rel.as_str()),
        "only the winning prefix claims the published blob"
    );
    assert_eq!(
        newer.stored_as, None,
        "the shorter prefix must not claim the path"
    );
    assert_eq!(
        blob_dir_names(&root),
        vec![format!("{}.zst", c.id)],
        "no temp or candidate file survives"
    );
}

/// The mirror image: the newer source's prefix is longer, so a shorter later
/// prefix is discarded and the newest one keeps the row and the file.
#[tokio::test]
async fn a_shorter_later_prefix_keeps_the_newest_candidate() {
    let payload = b"the newest CDN gets further before it truncates".repeat(64);
    let full = zstd::encode_all(payload.as_slice(), 1).expect("encode");
    let newer_len = full.len() * 2 / 3;
    let older_len = full.len() / 3;
    let newer_prefix = full[..newer_len].to_vec();
    let older_prefix = full[..older_len].to_vec();
    let (new_port, new_server) = serve_blob_once(full.len(), newer_prefix.clone());
    let (old_port, old_server) = serve_blob_once(full.len(), older_prefix);

    let c = chunk(&payload, 0, 0xca13);
    let p = pair_declared_by_both(&c, "longest_newer");
    let w = chunks::wants(&[p]);
    let root = girpr_tempdir::TempDir::new("gircap-f11", "longest_newer");
    let store = Store::new(
        &root,
        Sources::from_pairs([
            (
                "7.1.0".to_string(),
                format!("http://127.0.0.1:{new_port}/cdn"),
            ),
            (
                "7.0.0".to_string(),
                format!("http://127.0.0.1:{old_port}/cdn"),
            ),
        ]),
        crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
    );
    let fetcher = crate::http::Fetcher::new(
        crate::http::Transport::local(),
        crate::http::RequestLog::new(),
    )
    .expect("a fetcher with no configured headers to reject");

    let (entries, errors) = store.fill(&fetcher, &w, 1).await;
    new_server
        .join()
        .expect("the newest server served its prefix");
    old_server
        .join()
        .expect("the older server served its prefix");

    assert_eq!(errors.len(), 1, "only the published failure is reported");
    let props = &entries[0].0.props;
    let rel = BlobProps::stored_path_for(&c.id);
    assert!(props.partial);
    assert_eq!(
        props.bytes, newer_len as u64,
        "the newest, longer prefix is kept"
    );
    assert_eq!(props.stored_path.as_deref(), Some(rel.as_str()));
    let failure = props.fetch_error.as_ref().expect("and why it stopped");
    assert!(
        failure.detail.contains("via 7.1.0"),
        "the row names the source whose prefix won: {}",
        failure.detail
    );
    assert_eq!(
        std::fs::read(root.join(&rel)).expect("the published blob"),
        newer_prefix,
        "the file is exactly the winning prefix"
    );

    let rows = fetcher.log().records();
    let newer = rows
        .iter()
        .find(|r| r.url.contains(&format!(":{new_port}/")))
        .expect("the newest request is logged");
    let older = rows
        .iter()
        .find(|r| r.url.contains(&format!(":{old_port}/")))
        .expect("the older request is logged");
    assert_eq!(
        newer.stored_as.as_deref(),
        Some(rel.as_str()),
        "only the winning prefix claims the published blob"
    );
    assert_eq!(
        older.stored_as, None,
        "the shorter prefix must not claim the path"
    );
    assert_eq!(
        blob_dir_names(&root),
        vec![format!("{}.zst", c.id)],
        "no temp or candidate file survives"
    );
}

/// A later source that fails without writing anything leaves the first
/// prefix as the row; its temp must not displace the candidate.
#[tokio::test]
async fn a_zero_byte_failure_after_a_prefix_keeps_the_prefix() {
    let payload = b"the older CDN refuses this chunk entirely".repeat(64);
    let full = zstd::encode_all(payload.as_slice(), 1).expect("encode");
    let prefix_len = full.len() / 2;
    let prefix = full[..prefix_len].to_vec();
    let (new_port, new_server) = serve_blob_once(full.len(), prefix.clone());

    let c = chunk(&payload, 0, 0xca14);
    let p = pair_declared_by_both(&c, "zero_after_prefix");
    let w = chunks::wants(&[p]);
    let root = girpr_tempdir::TempDir::new("gircap-f11", "zero_after_prefix");
    let store = Store::new(
        &root,
        Sources::from_pairs([
            (
                "7.1.0".to_string(),
                format!("http://127.0.0.1:{new_port}/cdn"),
            ),
            // Port 0 is never bound: an immediate refusal that writes nothing.
            ("7.0.0".to_string(), "http://127.0.0.1:0".to_string()),
        ]),
        crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
    );
    let fetcher = crate::http::Fetcher::new(
        crate::http::Transport::local(),
        crate::http::RequestLog::new(),
    )
    .expect("a fetcher with no configured headers to reject");

    let (entries, errors) = store.fill(&fetcher, &w, 1).await;
    new_server
        .join()
        .expect("the newest server served its prefix");

    assert_eq!(errors.len(), 1, "only the published failure is reported");
    let props = &entries[0].0.props;
    let rel = BlobProps::stored_path_for(&c.id);
    assert!(props.partial);
    assert_eq!(props.bytes, prefix_len as u64);
    assert_eq!(props.stored_path.as_deref(), Some(rel.as_str()));
    let failure = props.fetch_error.as_ref().expect("and why it stopped");
    assert!(
        failure.detail.contains("via 7.1.0"),
        "the row keeps the prefix's failure: {}",
        failure.detail
    );
    assert_eq!(
        std::fs::read(root.join(&rel)).expect("the published blob"),
        prefix,
        "the zero-byte attempt did not replace the prefix"
    );
    assert_eq!(
        blob_dir_names(&root),
        vec![format!("{}.zst", c.id)],
        "no temp or candidate file survives"
    );
}

/// **N1/K2.** A measurement that does not return loses neither its cause nor
/// the bytes: the panic payload reaches the row as `measurement_error`, and
/// the already-published blob stays on disk with `stored_path` set. Only
/// `is_usable` rejects it, so a reader can tell "measurement failed" from
/// "no bytes were fetched".
///
/// The real [`BlobProps::measure_file`] cannot be made to panic on demand,
/// so this drives the same seam [`Store::stream_from`] uses with a closure
/// that panics deterministically.
#[tokio::test]
async fn a_panicking_measurement_keeps_the_published_blob_and_the_cause() {
    let dir = scratch("measure-panic");
    let id = "00000000_00000000000000000000000000000000";
    let bytes = b"published but never measured";
    let final_path = dir.join(BlobProps::stored_path_for(id));
    std::fs::create_dir_all(final_path.parent().expect("blob dir")).expect("create blob dir");
    std::fs::write(&final_path, bytes).expect("seed the blob");

    let chunk = crate::manifest::proto::SophonChunk {
        id: id.to_string(),
        ..Default::default()
    };
    let mut props =
        Store::measure_published(&final_path, id, hash::md5_sha256(bytes), &chunk, || {
            panic!("measure exploded at frame 7")
        })
        .await;
    props.set_stored(BlobProps::stored_path_for(id));

    let why = props
        .measurement_error
        .as_deref()
        .expect("the row says the measurement failed");
    assert!(why.starts_with(id), "the site's context survives: {why}");
    assert!(
        why.contains("the measure task panicked"),
        "the shared rule names the site: {why}"
    );
    assert!(
        why.contains("measure exploded at frame 7"),
        "the payload is the part worth reporting: {why}"
    );
    assert_eq!(
        props.bytes,
        bytes.len() as u64,
        "the write's byte count survives"
    );
    assert_eq!(
        props.md5_compressed,
        hash::md5_hex(bytes),
        "the write's digest survives"
    );
    assert!(
        props.verified.is_none(),
        "nothing measured the decoded side"
    );
    assert!(props.stored_path.is_some(), "the row still names its bytes");
    assert!(final_path.exists(), "the published blob must survive");
    assert!(
        !props.is_usable(),
        "bytes are not usable just because a path is set"
    );
    assert!(
        props
            .why_unusable()
            .unwrap_or_default()
            .contains("measurement failed"),
        "the reason distinguishes this from a fetch that never arrived"
    );
}
