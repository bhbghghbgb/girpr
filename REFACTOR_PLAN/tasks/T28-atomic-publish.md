# T28 — Generic `AtomicPublish` leaf

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F33 (lines 489–499) — atomic publish
  exists in three forms with incompatible contracts; candidate boundary is a generic
  `AtomicPublish` with explicit durable/parent-creation/replace-vs-noclobber parameters.
  Confidence medium-high.
- Assumptions to validate: §A13 (122–128), §A5 (57–67), §A2 (29–36), §A14 (129–135), §N2 (701–703).

## Scope
- `crates/util-path/src/atomic.rs` (`NamedTempFile` + `persist` + durable flag),
  `crates/sync-cache/src/open.rs` and `backup/publish.rs` (working names, link-into-place,
  hold records, 1-syscall gaps), `tools/sled2redb/src/convert.rs` (random
  `girpr-cache-convert-*.redb`, deliberately leftover namespaces), product `src/repair/file/temp.rs`
  (`_tmp` rename). Locate before editing.

## Implementation plan
1. Validate §F33: confirm the three contracts and their incompatibilities.
2. If attempted, define a generic primitive with explicit durable/parent-creation/
   replace-vs-noclobber parameters; leave each caller's gap/hold policy local.
3. Migrate callers directly (§A2); move through §A14 seams.
4. Given incompatible contracts, a validated skip per caller is legitimate if the
   parameters cannot express the differences safely.

## Constraints / validation
- High-risk (§F33): sync-cache's *bound* reads published names as whole copies and hold
   records protect a pre-lock gap; sled2redb's namespace is deliberately unreclaimed
   (§A13). Do not unify these into one behavior.
- `_tmp` resume/rollback and the audio move must not be absorbed (N2).
- Preserve publish names/paths bytes (§A5).

## Done when
A generic primitive is shared with all parameter differences preserved — or a validated
skip with the one-line reason is recorded.
