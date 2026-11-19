# T01 — Delete the stale zstd `Decoder` workaround

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §B1 (lines 351–372) — the
  workaround to delete and the intended replacement.
- Assumption to validate: §I.10 (lines 844–846).

## Scope
- Package: root `girpr`.
- Report names `stream_body` in `src/sophon/stream.rs` (lines 484–536). Locate it
  in the current tree before editing.

## Implementation plan
1. Confirm the pinned `zstd = "0.14.4"` exposes `write::Decoder::new`,
   `finish(self) -> Result<W>` and `do_finish(&mut self)`, and that `finish` is
   how a truncated frame is detected.
2. Replace the raw decoder + `zio::Writer` wiring with
   `write::Decoder::new(&mut hashing)?`; keep the existing `write_all` loop;
   end with `let mut hashing = dec.finish()?;` so the `HashingWriter` digest and
   byte count stay readable.
3. Remove the stale workaround comment.

## Constraints / validation
- No public API change; truncated-frame semantics identical.
- Tests named in §B1 pass unchanged; the multi-write test rules out simplifying
  to `decode_all`.
- If `finish` is unavailable or returns the wrong type in the pinned version,
  stop — the task is stale.

## Done when
The hand-rolled raw-decoder wiring is gone and the report's tests still pass
with identical behavior.
