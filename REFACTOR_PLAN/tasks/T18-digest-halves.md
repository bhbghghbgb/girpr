# T18 — Split the disjoint halves of `girpr-digest`

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F34 (lines 401–410) —
  `girpr-digest` has visibly disjoint consumer halves: fan-in 4; sled2redb uses
  only hex decoding, girsync keeps its own `AlgoSet`/`DigestSet`, root uses both
  `Md5` and the sink, gircap uses hex + `Sink`. An edit to
  `crates/digest/src/stream.rs` (304 lines) rebuilds sled2redb although it does
  not use it. `pub mod vectors` is a separate low-churn surface.
- Assumptions to validate: §A2 (27–33), §A3 (35–38), §A6 (55–61), §A7 (63–70).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (§N5 digest keepers via §A13).

## Scope
- `crates/digest/src/lib.rs:18-20,220-279`, `crates/digest/src/stream.rs`,
  `pub mod vectors`.
- Consumers: `tools/sled2redb/Cargo.toml:34-38`, `tools/sync/Cargo.toml:50-53`,
  root and gircap manifests.
- Locate all edges in the current tree before editing.

## Implementation plan
1. Validate §F34: confirm which consumers use hex only vs the streaming sink,
   and that `AlgoSet`/`DigestSet` remain girsync-local (§A13/§N5).
2. Separate the hex/newtype half from the streaming half so a consumer that
   needs only hex does not recompile the stream code when it changes (§A2).
   Prefer split crates (or a feature) that actually remove the rebuild edge.
3. Keep proto/digest bytes and hex casing byte-identical (§A6); do not fold
   `DigestSet`/`AlgoSet`/`HashAlg` into a single semantic type (§A13/§N5).
4. If gircap's leaf set changes, update §A5/§F39 doc tables; update any nextest
   filters if tests move (§F31).

## Constraints / validation
- §A2 payoff: sled2redb must no longer rebuild the stream half. If the facade
   still forces both halves on every consumer, stop.
- Byte contracts unchanged (§A6); deliberate digest divergences stay (§A13).
- Keep the crate logic-free and the independence tables consistent (§A5).

## Done when
The hex-only consumer no longer depends on the streaming half, digest bytes and
hex casing are unchanged, and all digest/stream tests pass.
