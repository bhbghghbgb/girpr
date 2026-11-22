//! A narrow integration consumer of the audio crate alone.
//!
//! This is a `girpr-audio` package target: it links only `girpr-audio`, not the
//! `girpr` repair facade, so an edit to the repair, report or CLI modules
//! cannot rebuild it, and an audio edit cannot rebuild the facade on its way
//! through here. (A root-package test would link the facade implicitly even
//! when it names only audio types.) It also pins the one byte contract this
//! crate owns: the scan file's display names in the game's canonical order.

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;

use girpr_audio::{read_scan_file, resolve_effective_audio, write_scan_file};
use girpr_tempdir::TempDir;

#[test]
fn the_audio_crate_is_consumable_without_the_repair_facade() {
    let d = TempDir::new("girpr_it", "audio_direct");

    // All four codes, supplied out of order, must be written as display names
    // in the game's canonical order — the byte spelling the game reads.
    let all: HashSet<String> = ["ko-kr", "zh-cn", "ja-jp", "en-us"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    write_scan_file(&d, "Audio/scan.txt", &all).expect("write the scan file");
    let bytes = std::fs::read(d.join("Audio/scan.txt")).expect("read the scan file back");
    assert_eq!(bytes, b"Chinese\nEnglish(US)\nJapanese\nKorean");

    // The narrow consumer sees the same three read outcomes the pipeline does.
    assert_eq!(
        read_scan_file(&d, "Audio/scan.txt").expect("read the scan file"),
        Some(all.clone())
    );
    assert_eq!(
        read_scan_file(&d, "").expect("an unconfigured dir is absence"),
        None
    );
    assert_eq!(
        resolve_effective_audio(&d, "Audio/scan.txt", None).expect("resolve the selection"),
        all
    );
}
