# T32 — Duplicated BOM stripping → unicode-bom

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §APP-12 (lines 404–409) — two one-line
  `strip_prefix('\u{feff}')` calls may become `unicode-bom`.
- Validate assumptions first: §A1 (15–16), §A2 (17–18). This finding is explicitly
  **low payoff and may be deliberately left alone** (§APP-12 line 409).

## Scope
- `app/util-ini/src/ini.rs:156`; `app/audio/src/lib.rs:133–135`.
- Tests at `app/audio/src/lib.rs:296`, `:317` and `ini.rs:708–748` pin BOM behavior.
- Affected manifests. `unicode-bom 2.0.3` is not currently locked.

## Implementation plan
1. Validate the premises: both sites strip only a UTF-8 BOM at the start of a byte
   buffer, and the surrounding consumers expect exactly that. If either site handles a
   different encoding or position, stop.
2. If judged worth doing, replace both calls with the `unicode-bom` API and keep the
   existing BOM-behavior tests unchanged (§APP-12 line 408). Add the dependency and
   verify a resolved build.
3. If the churn outweighs the benefit (one-line calls, new dependency), stop and record
   `skipped` with a one-line reason — this is the report's own guidance (line 409).

## Constraints / validation
- No behavior change: tests pinning BOM handling must stay (§APP-12 line 408).
- Do not expand into other text-encoding cleanup; the scope is exactly these two sites.

## Done when
Either both sites use `unicode-bom` with unchanged BOM tests, or the task is recorded as
`skipped` as low-payoff churn. If a premise is stale or a keeper would change, stop and
record `skipped`.
