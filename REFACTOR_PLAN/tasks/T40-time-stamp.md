# T40 — Time/stamp leaf with named formats

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F45 (lines 607–615) — timestamp
  formatting has one shared compact stamp and two RFC 3339 spellings; candidate boundary is
  a time/stamp leaf with named formats. Confidence medium-high.
- Assumptions to validate: §A5 (57–67), §A2 (29–36), §A14 (129–135).

## Scope
- `crates/log/src/plan.rs` / `lib.rs` (compact `%Y%m%dT%H%M%S%.6fZ` stamp used by capture ids
  and sync-cache backup names), `tools/capture/http/src/log.rs`
  (`to_rfc3339_opts(Millis, true)`), `crates/lock/src/lib.rs` (`to_rfc3339()` in lock body).
  Locate before editing.

## Implementation plan
1. Validate §F45: confirm the three formats and that all three strings are on-disk or
   artifact byte contracts.
2. Define a time/stamp leaf exposing the named formats (compact, RFC3339-millis-Z,
   RFC3339) as pure formatters; do not change any format's spelling.
3. Migrate callers directly (§A2); move through §A14 seams if gircap calls it.
4. Commit leaf, migrations, seams.

## Constraints / validation
- High-risk (§F45): all three strings are on-disk or artifact byte contracts (§A5) —
   fractional digits, `Z` suffix and separators must stay identical.
- Do not collapse the formats into one; keep named variants.

## Done when
The formatters are shared under named formats, all stamped strings stay byte-identical, and
doc seams are updated.
