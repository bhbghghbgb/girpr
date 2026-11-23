# T12 — Validate reuse offsets and parse digests once

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §6 **COR-03** (lines 442–451) — negative/oversized
  local-manifest reuse offsets are silently clamped (`entry.offset.max(0) as u64`);
  validate `offset >= 0`, `size >= 0`, `offset.checked_add(size) <=` local file length
  at map-build time, warn and skip the entry, then drop the `.max(0)`.
- §8 **MIC-01** (lines 564–573) and **MIC-08** (610–613) — `Md5::parse_hex` is redone
  per reuse lookup and allocates an error for a normal miss; add a non-allocating
  `try_parse_hex(&str) -> Option<Md5>` and parse once at decode time, storing the
  parsed `Md5` on the chunk or in the reuse map.
- §8 **MIC-11** (lines 624–629) — `FileReuse::key` builds a fresh `DefaultHasher` per
  lookup; build a `u64` from the two halves of `md5.as_bytes()` or key directly on
  `IdentityKey { digest, size }` (already `Hash + Eq`).
- Keep items: reuse behavior, §12, `guidance/DO-NOT-TOUCH.md`.

## Scope
- `app/fetch/src/sophon/map.rs` (offset handling ~176–180, digest parsing ~112–123,
  `FileReuse::key` / `by_key` ~141–145).
- `src/repair/reuse.rs` (~182–191), `crates/digest/src/lib.rs` (`parse_hex`
  ~101–108, 124–126), and callers in `app/util-hash/src/md5.rs:222`,
  `app/fetch/src/sophon/stream.rs:129`, `src/repair/file/temp.rs:645`.
- Locate every caller before editing; the list above is a hint.

## Implementation plan
1. Confirm the current offset handling and all `parse_hex` call sites. If reuse
   entries are already validated or offsets are known-good, skip the corresponding
   sub-change.
2. Validate offsets/sizes at map-build time with `checked_add` against the local file
   length; warn and skip bad entries; remove the silent `.max(0)`.
3. Add `try_parse_hex` returning `Option<Md5>` and parse once at decode time; update
   callers to use the stored/parsed value and remove the allocation on the common
   miss path.
4. Replace the per-lookup `DefaultHasher` with a cheap `u64` or key directly on
   `IdentityKey`.
5. Deliver in several commits if useful: offset validation, digest API and callers,
   key hashing.

## Constraints / validation
- Reuse decisions must stay correct: a bad offset must result in a skipped entry or a
  download, never in corrupted reuse or a false "intact" verdict.
- The non-allocating parse must produce the same `Md5` as `parse_hex` for all inputs.
- Do not change data-plane output or any §12 non-finding.

## Done when
Offsets are validated with diagnostics, digests are parsed once without
miss-path allocations, and the reuse map is keyed cheaply, with tests passing. If the
premise is stale or validation cannot be added safely, **stop** and record `skipped`
with a one-line reason.
