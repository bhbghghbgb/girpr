//! A narrow integration consumer of the fetch crate alone.
//!
//! This is a `girpr-fetch` package target: it links only `girpr-fetch`, not the
//! `girpr` repair facade, so an edit to the repair, report or CLI modules
//! cannot rebuild it, and a fetch edit cannot rebuild the facade on its way
//! through here. (A root-package test would link the facade implicitly even
//! when it names only fetch types.) It also pins the two byte spellings the
//! fetch crate owns: `chuncks` on the wire and the opaque tag 6.

use girpr_fetch::sophon::stream::join_url;
use girpr_fetch::sophon::{SophonChunk, SophonChunkFile, SophonChunkManifest};
use prost::Message as _;

#[test]
fn the_fetch_crate_is_consumable_without_the_repair_facade() {
    let base = "https://sophon.example.com/manifest/5.1.0/";
    let url = join_url(base, "abc123").expect("join the manifest id");
    assert_eq!(
        url.as_str(),
        "https://sophon.example.com/manifest/5.1.0/abc123"
    );

    let manifest = SophonChunkManifest {
        chuncks: vec![SophonChunkFile {
            file: "a/b.dat".into(),
            chunks: vec![SophonChunk {
                id: "chunk".into(),
                uncompressed_md5: String::new(),
                offset: 0,
                compressed_size: 0,
                uncompressed_size: 0,
                unknown_field6: 150,
                compressed_md5: String::new(),
            }],
            is_folder: false,
            size: 0,
            md5: String::new(),
        }],
    };
    let bytes = manifest.encode_to_vec();
    // Tag 6 is field 6, wire type 0 (`0x30`), and its value survives the
    // fetch-side decode boundary the same way girpr-proto pins it.
    assert!(bytes.windows(3).any(|w| w == [0x30, 0x96, 0x01]));
    let back = SophonChunkManifest::decode(bytes.as_slice()).expect("decode");
    assert_eq!(back.chuncks[0].chunks[0].unknown_field6, 150);
}
