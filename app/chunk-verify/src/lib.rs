//! Chunk-verification mechanics, held at an acyclic leaf.
//!
//! This crate owns the value type a scan produces about a file, the range/owner
//! remaps that turn a scan's per-range answers into per-chunk verdicts (and
//! back), and the in-place writer's verify pass. It depends only on other
//! leaves — the wire chunk, the hash/scan mechanics and `anyhow` — so an edit
//! here leaves the repair facade cached and a narrow test target can link it
//! without the binary. It holds no scan-planning policy, no write-direction
//! decision and no published bytes.
//!
//! The file-reuse guard (the coarse-MTIME proof) deliberately stays with the
//! writer in the facade: it reads the plan and the scan report together, so it
//! is not a leaf concern. What moves here is the data it guards — [`Verdict`],
//! including its `modified` timestamp — and the remaps the scan and the writer
//! share. The verify pass re-derives the answer from a fresh read; it never
//! reuses an observation buffer to assert "verified".
//!
//! [`verify_whole_file`] is the other shared shape: one presence → length →
//! digest decision over values the caller supplies. The presence probe and the
//! digest read are injected, so this crate never opens a file or decides what
//! a program does with a failure. Every caller maps [`WholeFileVerdict`] to its
//! own error type and keeps its own next action, warning and exit behaviour.

use std::time::SystemTime;

use anyhow::Result;
use girpr_chunk_cost::nonneg;
use girpr_proto::SophonChunk;
use girpr_util_hash::{Md5, Scan, ScanRange};

/// What one file's scan found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// The whole-file MD5 of what was on disk, as a digest. Hex is produced
    /// only where a report or a log needs text.
    pub file_md5: Md5,
    /// Per chunk, in the plan's order: is it correct where it belongs?
    pub dest_ok: Vec<bool>,
    /// Per chunk, in the plan's order: is its reuse source intact?
    pub source_ok: Vec<bool>,
    /// The file length the scan read. The in-place writer compares this
    /// against its open handle before reusing the verdict.
    pub file_len: u64,
    /// The file's modification time when the scan was taken, from one
    /// `metadata` call. `None` when that stat failed, which leaves the
    /// verdict unusable for reuse.
    pub modified: Option<SystemTime>,
}

impl Verdict {
    /// The chunks that are wrong where they belong.
    pub fn damaged_chunks(&self) -> u64 {
        self.dest_ok.iter().filter(|ok| !**ok).count() as u64
    }
}

/// Build the destination/source range lists a one-read scan needs, plus the
/// chunk owner of each source range.
///
/// A chunk without a source contributes no source range: coercing the absent
/// ones to offset 0 would ask the scan to verify the chunk's bytes at the head
/// of the file, so unrelated bytes that happen to match (the file reflowed, or
/// the head holds the content under a different layout) would mark a
/// non-existent source intact while the writer, which gates on a real source,
/// downloads the chunk anyway. `source_owner[k]` is the chunk `source[k]`
/// belongs to, so the scan's per-source verdicts can be mapped back to one
/// verdict per chunk.
pub fn scan_ranges<'a>(
    chunks: &'a [SophonChunk],
    sources: &[Option<u64>],
) -> (Vec<ScanRange<'a>>, Vec<ScanRange<'a>>, Vec<usize>) {
    let dest_ranges = chunks
        .iter()
        .map(|c| {
            (
                nonneg(c.offset),
                nonneg(c.uncompressed_size),
                c.uncompressed_md5.as_str(),
            )
        })
        .collect();
    let mut source_ranges: Vec<ScanRange<'_>> = Vec::new();
    let mut source_owner: Vec<usize> = Vec::new();
    for (i, (c, src)) in chunks.iter().zip(sources.iter()).enumerate() {
        let Some(offset) = src else { continue };
        source_owner.push(i);
        source_ranges.push((
            *offset,
            nonneg(c.uncompressed_size),
            c.uncompressed_md5.as_str(),
        ));
    }
    (dest_ranges, source_ranges, source_owner)
}

/// Map the scan's per-source-range verdicts back onto one verdict per chunk.
///
/// `scan_source_ok` is indexed by the range list [`scan_ranges`] built, which
/// has one entry per chunk **with a source**; `source_owner[k]` is the chunk
/// that range `k` belongs to. A chunk with no source has no intact source, by
/// definition, so it stays `false`. A source verdict that is missing keeps
/// that same conservative answer rather than panicking.
///
/// A discarded plan replaces a real scan with an absent scan, whose tuple is
/// `(dest ranges, source ranges)`; this remap is what keeps that per-range
/// shape from being read as per-chunk. Both the download estimate and the
/// in-place writer use this one definition.
pub fn source_ok_by_chunk(
    chunks_len: usize,
    source_owner: &[usize],
    scan_source_ok: &[bool],
) -> Vec<bool> {
    let mut source_ok = vec![false; chunks_len];
    for (k, &i) in source_owner.iter().enumerate() {
        source_ok[i] = scan_source_ok.get(k).copied().unwrap_or(false);
    }
    source_ok
}

/// The inverse of [`source_ok_by_chunk`]: one verdict per source range, in the
/// order [`scan_ranges`] built them, from one verdict per chunk.
///
/// The in-place writer uses this when it replaces a fresh scan with the plan's
/// per-chunk verdict. Both directions share the same ownership rule, and they
/// are pinned against each other in one test so the two cannot drift.
pub fn source_ok_by_range(chunk_source_ok: &[bool], source_owner: &[usize]) -> Vec<bool> {
    source_owner
        .iter()
        .map(|&i| chunk_source_ok.get(i).copied().unwrap_or(false))
        .collect()
}

/// What one verification run over the in-place target answered.
#[derive(Debug, PartialEq, Eq)]
pub enum Verified {
    /// The whole-file digest is the manifest's; the chunk verdicts were not
    /// needed.
    Matches(Md5),
    /// The digest did not match. `still_bad` names the chunks whose verdicts
    /// the follow-up scan found wrong, in plan order.
    Mismatch { md5: Md5, still_bad: Vec<usize> },
}

/// Verify the target: one pass for the digest, a second only on a mismatch.
///
/// The digest alone answers success, and success is the normal outcome, so the
/// gate is a whole-file read that hashes nothing else. When the digest does
/// not match, the per-chunk verdicts are needed to name what to redo; that
/// second read is the rare path, and it is the only one that pays for the
/// range digests. Asking the first pass for the ranges as well would hash
/// every chunk range on every repaired file to save the second read only when
/// something is wrong.
///
/// `read` takes the ranges to hash and returns the scan. It is called once
/// with no ranges and, only on a mismatch, once with `dest_ranges`. Each call
/// is a fresh read — the verify is an independent re-derivation, never a reuse
/// of the observation the writer gathered while patching.
pub fn verify_in_place(
    dest_ranges: &[ScanRange<'_>],
    mut read: impl FnMut(&[ScanRange<'_>]) -> Result<Scan>,
    chunks_len: usize,
    expect: &str,
) -> Result<Verified> {
    let scan = read(&[])?;
    if Md5::try_parse_hex(expect).is_some_and(|expected| expected == scan.file_md5) {
        return Ok(Verified::Matches(scan.file_md5));
    }
    let scan = read(dest_ranges)?;
    let still_bad = (0..chunks_len)
        .filter(|&i| !scan.dest_ok.get(i).copied().unwrap_or(false))
        .collect();
    Ok(Verified::Mismatch {
        md5: scan.file_md5,
        still_bad,
    })
}

/// Case-insensitive equality of two hex digest spellings.
///
/// This is the one comparison every whole-file gate uses. Both arguments are
/// hex from the digest leaf (`Md5::to_hex`, `format_xxhash64_hex`), so a spelling
/// that differs only in case names the same digest. Which algorithm a row
/// prefers, and how a mismatch is reported, stay with the caller.
pub fn digest_matches(expected_hex: &str, observed_hex: &str) -> bool {
    expected_hex.eq_ignore_ascii_case(observed_hex)
}

/// The result of a whole-file presence → length → digest gate.
///
/// `E` is the caller's own "the digest source could not be read" error. Carrying
/// it in the verdict is what lets each program keep its own error spelling and
/// decide for itself whether that failure is fatal, a retry, or a repair.
#[derive(Debug, PartialEq, Eq)]
pub enum WholeFileVerdict<E> {
    /// The presence probe found no file; the digest source was not called.
    Absent,
    /// The file is present but its length is not the declared one; the digest
    /// source was not called. A short file and an overlong file share this arm.
    LengthMismatch { expected: u64, actual: u64 },
    /// The length passed but the injected digest source failed.
    DigestUnavailable(E),
    /// The length passed but the observed digest differs from the declared one.
    DigestMismatch { expected: String, actual: String },
    /// Present, declared length, declared digest. `observed` is the spelling
    /// the source produced, so a caller that logs the checked digest can log
    /// what it actually read rather than re-formatting the declared one.
    Match { observed: String },
}

impl<E> WholeFileVerdict<E> {
    /// Whether every gate passed.
    pub fn is_match(&self) -> bool {
        matches!(self, Self::Match { .. })
    }
}

/// Apply the presence → length → digest decision over caller-supplied values.
///
/// `observed_size` is the presence and length probe's answer: `None` means the
/// file is not there, `Some(n)` its current length. `expected_hex` is the
/// declared digest in whatever spelling this program uses (MD5 or XXH64 hex),
/// and `source` produces the observed digest spelling. The gate calls `source`
/// only after an absent file and a wrong length are ruled out, so a file that
/// fails an earlier gate is never hashed.
///
/// The gate performs no I/O and holds no policy: the caller injects the probe
/// and the digest read, picks the digest algorithm, and maps the verdict to its
/// own error type and next action.
pub fn verify_whole_file<E>(
    expected_size: u64,
    expected_hex: &str,
    observed_size: Option<u64>,
    source: impl FnOnce() -> Result<String, E>,
) -> WholeFileVerdict<E> {
    let Some(actual_size) = observed_size else {
        return WholeFileVerdict::Absent;
    };
    if actual_size != expected_size {
        return WholeFileVerdict::LengthMismatch {
            expected: expected_size,
            actual: actual_size,
        };
    }
    let observed_hex = match source() {
        Ok(hex) => hex,
        Err(e) => return WholeFileVerdict::DigestUnavailable(e),
    };
    if digest_matches(expected_hex, &observed_hex) {
        WholeFileVerdict::Match {
            observed: observed_hex,
        }
    } else {
        WholeFileVerdict::DigestMismatch {
            expected: expected_hex.to_owned(),
            actual: observed_hex,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Seek, SeekFrom};
    use std::path::Path;

    use girpr_tempdir::TempDir;
    use girpr_util_hash::{md5::md5_digest, scan_reader};

    use super::*;

    /// One remap pins the ownership rule: only chunks with a source have an
    /// entry in `scan_source_ok`, and a chunk without one stays `false`. A
    /// verdict list that is shorter than the owner list is a missing answer,
    /// not a panic.
    #[test]
    fn the_source_verdicts_land_on_the_chunk_that_owns_the_range() {
        assert_eq!(
            source_ok_by_chunk(3, &[0, 2], &[true, true]),
            vec![true, false, true],
            "chunk 1 has no source range, so it has no intact source"
        );
        assert_eq!(
            source_ok_by_chunk(3, &[0, 2], &[true]),
            vec![true, false, false],
            "a missing per-range verdict is false, not a panic"
        );
    }

    /// The per-range view is the exact inverse of the shared remap: a
    /// round-trip through both leaves every range verdict unchanged, and a
    /// chunk the owner list does not claim is simply absent from the result.
    #[test]
    fn the_per_range_view_inverts_the_shared_remap() {
        let owner = [0usize, 2];
        for ranges in [[true, true], [true, false], [false, true], [false, false]] {
            let by_chunk = source_ok_by_chunk(3, &owner, &ranges);
            assert_eq!(
                source_ok_by_range(&by_chunk, &owner),
                ranges.to_vec(),
                "every per-range verdict survives the round-trip"
            );
        }

        assert_eq!(
            source_ok_by_range(&[true, false, true], &owner),
            vec![true, true]
        );
        assert_eq!(
            source_ok_by_range(&[true], &owner),
            vec![true, false],
            "a chunk with no verdict is not an intact source"
        );
    }

    /// A no-source chunk stays `false` even when the scan's source list carries
    /// an entry the owner list does not claim.
    #[test]
    fn a_stray_source_verdict_never_marks_a_no_source_chunk() {
        assert_eq!(
            source_ok_by_chunk(3, &[2], &[true, true]),
            vec![false, false, true],
            "the extra answer belongs to no chunk, and chunk 1 has no source"
        );
    }

    /// The verify pass pins the whole-file digest on success and names the
    /// chunks to redo on a mismatch. The corrupt byte is in the chunk the
    /// patch did not write, so the digest has to fail and the follow-up pass
    /// has to report that chunk — not the one the patch rewrote.
    #[test]
    fn the_verify_pass_pins_the_digest_and_names_the_still_bad_chunks() {
        let dir = TempDir::new("girpr_test_chunk_verify", "verify_digest");
        let path = dir.join("f.dat");
        verify_the_pass_at(&path);
    }

    fn verify_the_pass_at(path: &Path) {
        let mut good = vec![0x41u8; 64];
        good.extend_from_slice(&[0x42u8; 64]);
        std::fs::write(path, &good).unwrap();

        let md5_0 = md5_digest(&good[..64]).to_hex();
        let md5_1 = md5_digest(&good[64..]).to_hex();
        let dest: Vec<ScanRange<'_>> = vec![(0, 64, &md5_0), (64, 64, &md5_1)];
        let expect = md5_digest(&good).to_hex();

        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        // The verify starts at the beginning of the file even when the last
        // write left the handle at EOF.
        f.seek(SeekFrom::End(0)).unwrap();
        let mut reads = 0;
        match verify_in_place(
            &dest,
            |ranges| {
                reads += 1;
                scan_reader(&mut f, ranges, &[])
            },
            2,
            &expect,
        )
        .unwrap()
        {
            Verified::Matches(md5) => assert_eq!(md5.to_hex(), expect),
            Verified::Mismatch { md5, .. } => {
                panic!("an intact file must match, got {md5}")
            }
        }
        assert_eq!(reads, 1, "an intact file needs only the digest pass");

        let mut bad = good.clone();
        bad[70] ^= 0xff;
        std::fs::write(path, &bad).unwrap();
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        f.seek(SeekFrom::End(0)).unwrap();
        let mut reads = 0;
        match verify_in_place(
            &dest,
            |ranges| {
                reads += 1;
                scan_reader(&mut f, ranges, &[])
            },
            2,
            &expect,
        )
        .unwrap()
        {
            Verified::Mismatch { md5, still_bad } => {
                assert_eq!(
                    still_bad,
                    vec![1],
                    "the follow-up pass names the chunk that is still wrong"
                );
                assert_eq!(md5, md5_digest(&bad));
            }
            Verified::Matches(md5) => panic!("a damaged file must not match, got {md5}"),
        }
        assert_eq!(
            reads, 2,
            "a mismatch reads once more to name the chunks, not on the success path"
        );
    }

    /// An absent file stops at presence: the injected digest source must never
    /// run. That is what keeps a missing file's empty digest from matching an
    /// empty file's manifest entry.
    #[test]
    fn an_absent_file_stops_before_the_digest_source() {
        let verdict = verify_whole_file(4, "abcd", None, || -> Result<String, ()> {
            panic!("an absent file is never hashed")
        });
        assert_eq!(verdict, WholeFileVerdict::Absent);
        assert!(!verdict.is_match());
    }

    /// A short body and an overlong body are both length failures; neither is
    /// hashed, even when the digest the source would return agrees. Presence is
    /// not "a length of zero" — it is its own gate.
    #[test]
    fn a_short_and_an_overlong_file_are_length_mismatches() {
        let expected = "d41d8cd98f00b204e9800998ecf8427e";
        let short = verify_whole_file(4, expected, Some(3), || -> Result<String, ()> {
            panic!("a short file is never hashed")
        });
        assert_eq!(
            short,
            WholeFileVerdict::LengthMismatch {
                expected: 4,
                actual: 3
            }
        );
        let long = verify_whole_file(4, expected, Some(5), || -> Result<String, ()> {
            panic!("an overlong file is never hashed")
        });
        assert_eq!(
            long,
            WholeFileVerdict::LengthMismatch {
                expected: 4,
                actual: 5
            }
        );
    }

    /// A failing digest source is its own verdict: the caller sees its error
    /// value back and can spell it and classify it however it needs to.
    #[test]
    fn a_failed_digest_source_reaches_the_caller_unchanged() {
        let verdict = verify_whole_file(4, "abcd", Some(4), || {
            Err("the digest read failed".to_string())
        });
        assert_eq!(
            verdict,
            WholeFileVerdict::DigestUnavailable("the digest read failed".to_string())
        );
        assert!(!verdict.is_match());
    }

    /// The comparison is case-insensitive, and a mismatch carries both the
    /// declared and the observed spelling for the caller's error message.
    #[test]
    fn digest_spellings_are_case_insensitive_and_a_mismatch_carries_both() {
        let lower = "d41d8cd98f00b204e9800998ecf8427e";
        let upper = "D41D8CD98F00B204E9800998ECF8427E";
        assert_eq!(
            verify_whole_file(0, upper, Some(0), || Ok::<_, ()>(lower.to_string())),
            WholeFileVerdict::Match {
                observed: lower.to_string()
            }
        );
        assert!(verify_whole_file(0, upper, Some(0), || Ok::<_, ()>(lower.to_string())).is_match());
        assert_eq!(
            verify_whole_file(0, lower, Some(0), || Ok::<_, ()>(upper.to_string())),
            WholeFileVerdict::Match {
                observed: upper.to_string()
            }
        );
        assert_eq!(
            verify_whole_file(0, lower, Some(0), || {
                Ok::<_, ()>("00000000000000000000000000000000".to_string())
            }),
            WholeFileVerdict::DigestMismatch {
                expected: lower.to_string(),
                actual: "00000000000000000000000000000000".to_string(),
            }
        );
    }

    /// The comparison primitive the gate uses is also the one digest-only
    /// callers use: case folds, but a different digest or a malformed spelling
    /// never matches.
    #[test]
    fn digest_matches_ignores_case_but_not_content_or_spelling() {
        let lower = "d41d8cd98f00b204e9800998ecf8427e";
        assert!(digest_matches(lower, &lower.to_ascii_uppercase()));
        assert!(digest_matches(&lower.to_ascii_uppercase(), lower));
        assert!(!digest_matches(lower, "d41d8cd98f00b204e9800998ecf8427f"));
        assert!(!digest_matches("", lower));
        assert!(!digest_matches("not a digest", lower));
    }
}
