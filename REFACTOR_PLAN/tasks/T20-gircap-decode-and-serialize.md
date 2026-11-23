# T20 — Memoize decoded blobs and serialize the manifest once

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §9 **TOOL-03** (lines 661–669) — `verify` reads whole
  blobs and zstd-decompresses each twice; stream with `zstd_measure_from` and populate
  a shared `BTreeMap<String, OnceLock<Vec<u8>>>` memo accepted by
  `ChunkSource::memoizing`.
- §9 **TOOL-04** (671–678) — capture decompresses each blob twice (measure then
  assemble); memoize decoded bytes for the bounded window with an explicit bound on
  RSS. This is a deliberate memory trade-off worth revisiting, not a mandate.
- §9 **TOOL-11** (716–720) — redundant `Option<String>` clones per measured blob;
  consume `measured` with `.ok()` and move `m.md5`.
- §9 **TOOL-12** (721–728) — full manifest decode is serialized twice (`json_len`
  then `write_json_gz`); serialize once into a temporary file with a counting writer,
  then gzip to the final name and delete the temp.
- §11 **assumption 15** (lines 878–880) — memoization and temp-file serialization
  change RSS vs. disk-I/O trade-offs; measure on a real capture bundle before choosing.
- Keep items: deterministic artifact bytes, bounded memory, §12,
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `tools/capture/src/verify/checks.rs` (~79–93, 119–137), `assemble/source.rs`
  (~201–232, memoizing source), `blobs/props.rs` (~338–375, 486–505),
  `emit/fixture.rs` (~156–168), `manifest/dump.rs` (~137, 150), `emit/mod.rs`
  (~444–449, 481–532).
- Locate current code before editing; the list above is a hint.

## Implementation plan
1. Validate assumption 15 on a real capture bundle: confirm the memoization window and
   temp-file serialize are acceptable memory/disk trade-offs. If they are not bounded,
   skip the memoization sub-change and keep only the safe parts.
2. Use streaming `zstd_measure_from` for verification and share a bounded decoded-blob
   memo between measure and assemble (TOOL-03/04).
3. Remove the redundant clones by consuming `measured` (TOOL-11).
4. Serialize the manifest once to a counted temp file, then gzip to the final name and
   delete the temp (TOOL-12).
5. Deliver in several commits if useful: streaming verify, bounded memo, serialize once.

## Constraints / validation
- Artifact bytes and documented write order must stay deterministic.
- Peak RSS must remain bounded (explicit memo bound); do not undo the memory trade-off
  the report calls deliberate without measuring (assumption 15).
- Do not change any §12 non-finding.

## Done when
Each blob is decoded once within a bounded memo, the manifest is serialized once, and
artifact bytes are byte-identical with tests passing. If the memory/disk trade-off is
unacceptable or cannot be bounded, **stop** and record `skipped` with a one-line
reason.
