# T16 — strum enum tables

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §LEAF-7 (lines 496–505) — hand-written
  `as_str`/`Display`/`parse` tables in `girpr-model::region` and `girpr-log::level`
  become `strum` derives (`Display`, `EnumString`, `EnumIter`).
- §SYNC-2 (lines 551–561) — `HashAlg` name/parse/format plumbing becomes strum as well,
  with `digest_len()`/`bit()` kept local.
- §X4 (lines 127–134) — cross-cutting small mechanics theme.
- Validate assumptions first: §A1 (15–16), §A2 (17–18), §A12 (48–50). Keep items:
  `guidance/DO-NOT-TOUCH.md` — shared crates must keep working for unrelated consumers,
  and the `AlgoSet` Debug/`{:<12}` padding contract (§SYNC-2 lines 557–559) is pinned.

## Scope
- `crates/model/src/region.rs:76–83`, `:140–144`; test `:163–250`.
- `crates/log/src/level.rs:40–82`; test `:88–104` (includes the `warning` alias and
  custom error text).
- `tools/sync/cache/src/hash/algo.rs:25–47`, `:75–86`, `:115–119`, `:155–198`,
  `:211–269`; padding pins in `planner/coverage.rs:110–112` and `tests.rs:324–344`.
- Affected manifests. Note `strum` is not currently locked.

## Implementation plan
1. Validate the exact spellings and canonical order the current tables produce (region
   ids/sets, level alias and error text, `HashAlg::ALL` order and `names()`).
2. Annotate variants with `#[strum(serialize = "...")]` and derive the needed traits;
   keep thin `parse` wrappers for aliases/error text and the existing `clap::ValueEnum`
   derive. Keep `endpoints()`, `digest_len()` and `bit()` local.
3. Decide deliberately whether manual `Display` stays to preserve the `{:<12}` padding
   and the hand-written `Debug` spelling (both pinned); if so, derive only the other
   traits. Map `strum::ParseError` into `UnknownHash` so its behavior survives.
4. Keep or delete the const `from_name` + compile-time assertion block deliberately
   (strum cannot generate it); verify serde/postcard wire bytes for `DigestSet` are
   unchanged. Run the named tests and the broader consumers.

## Constraints / validation
- These are shared crates: no behavior change for unrelated consumers (§A8 lines 35–36).
- Strum derives must not touch serde; `postcard` wire bytes stay identical (§SYNC-2 line 561).
- Keep pinned padding and Debug wording unless a task explicitly allows a change.

## Done when
The hand-written name tables are gone (except deliberate thin wrappers), aliases/errors/
order/padding survive, wire bytes are unchanged, and all named tests pass. If a premise
is stale or a keeper would change, **stop** and record `skipped`.
