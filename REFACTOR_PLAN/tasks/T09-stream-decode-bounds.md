# T09 — Bound streaming decode output and validate blob IDs

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §4 **MEM-01** (lines 297–308) — every manifest fetch
  reserves the full clamped `uncompressed_size` up front (4 × 64 MiB from untrusted
  hints); start with a modest capacity and grow geometrically, keeping the clamp only
  as a hard ceiling.
- §5 **NET-05** (lines 386–394) — `stream_body(..., expected = None, ...)` gives the
  manifest decode no decoded-size ceiling and no compressed-byte cap; pass a hard
  `expected` cap and a compressed-byte cap so the existing `HashingWriter` limits
  apply.
- §5 **NET-06** (lines 395–403) — raw manifest blob IDs are joined into URLs
  unvalidated; an absolute `http://…`, leading `/`, `..`, `?` or `#` can redirect the
  fetch. Reject unsafe shapes and assert the resulting URL's origin/path prefix.
- §11 **assumption 8** (lines 859–861) — the decode cap must exceed the real largest
  manifest (16.8 MB for build 7.1.0) and the blob-id shape check must not reject any
  production id. Validate against captured bundles before enforcing.
- Keep items: wire behavior, §12, `guidance/DO-NOT-TOUCH.md`.

## Scope
- `app/fetch/src/sophon/stream.rs` (buffer reservation ~38–39, 76–85, decode call
  ~117, 505, URL joins ~33–34, 415–419, 579–587).
- `src/repair/phases/manifest.rs` (`MAX_MANIFEST_CONCURRENCY = 4` ~42).
- Captured HTTP/JSON response fixtures and capture bundles for cap/id validation.
  Locate current fixtures before editing.

## Implementation plan
1. Validate assumption 8: confirm the cap is above the largest real manifest and the
   id shape check admits every production id. If a cap or check would break real
   traffic, adjust or stop.
2. Replace up-front full-size reservation with modest initial capacity and geometric
   growth; keep the clamp only as a hard ceiling (MEM-01).
3. Pass a hard `expected` decode cap and a compressed-byte cap to `stream_body` for
   the manifest path (NET-05), letting `HashingWriter` limits apply.
4. Add blob-id shape validation (reject empty, absolute, `://`, leading `/`, `..`,
   `?`, `#`) and assert the joined URL's origin and path prefix before sending
   (NET-06).
5. Deliver in several commits if useful: buffer growth, decode caps, id validation.

## Constraints / validation
- Caps must be above the real largest manifest and id checks must not reject
  production ids (assumption 8); validate on captured bundles.
- Do not change the fetched bytes or on-disk representation.
- Do not change data-plane output or any §12 non-finding.

## Done when
Manifest decode is size-capped, buffers grow geometrically, and blob IDs are
validated without rejecting production traffic, with tests passing. If the cap/id
assumption fails, **stop** and record `skipped` with a one-line reason.
