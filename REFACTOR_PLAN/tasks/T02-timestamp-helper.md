# T02 — One shared timestamp helper

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §C2 (lines 483–494).
- Related: §G3 (lines 778–781).

## Scope
- Packages: `crates/log` (`girpr_log`), `tools/sync`, `tools/sled2redb`.

## Implementation plan
1. Export one `pub fn timestamp()` from `girpr_log` producing the existing
   `%Y%m%dT%H%M%S%.6fZ` format currently duplicated in `tools/sync`'s `ts_now`.
2. Make `tools/sync`'s `ts_now` delegate to it.
3. Make `tools/sled2redb` use the shared helper instead of calling
   `girsync::util::ts_now()`; keep a thin `ts_now` shim in girsync if any callers
   remain.

## Constraints / validation
- Byte-identical output; no behavior change (§C2 "expected change").
- Tests named in §C2 (23-char shape) and the hard-coded stamp in sync plan tests
  must still hold.

## Done when
There is a single source for the timestamp format and all three crates use it.
