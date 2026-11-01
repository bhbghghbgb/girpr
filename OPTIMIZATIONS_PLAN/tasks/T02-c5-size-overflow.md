# T02 - C5: reject a JSON size larger than `i64::MAX`

Report: `../../OPTIMIZATIONS_REPORT.md` lines 165-183 (C5).

Dependencies: none.

## Goal

`hyp::num_from_string` accepts a manifest size as either a JSON number or a
numeric string. Its unsigned visitor casts `u64` to `i64` with `as`, which wraps
a value above `i64::MAX` into a negative number. The rest of the code clamps
negative sizes to zero, so a corrupt or hostile manifest silently becomes free
downloads and zero reservations.

Make an out-of-range size a visible metadata parse error. A legitimate size
above `i64::MAX` is not a real manifest, so failing the run is the correct
trade.

## Where to start

- `src/hyp.rs`: `num_from_string` (around line 120), the `V` visitor's
  `visit_u64` and `visit_str`/`visit_string`; `ManifestFile` (around line 147).
- `src/hyp.rs` tests: `num_from_string_both_shapes` (around line 508).
- Callers that consume `compressed_size` / `uncompressed_size` only need to
  keep working for valid values; do not change their `.max(0)` clamps in this
  task.

## Steps

1. In `visit_u64`, use `i64::try_from(v)` and map the failure to
   `E::custom(...)` with a message that names the value and says it is larger
   than `i64::MAX`. Do not use `as`.
2. In `visit_str` / `visit_string`, keep parsing as `i64`, but make the failure
   message name the raw string. A string spelling a value above `i64::MAX`
   must be rejected, not truncated or re-parsed as a float.
3. Keep `visit_i64` as-is; negative sizes are a separate corruption question
   and are still clamped by the existing consumers. Do not add a new range
   rule for negatives here.
4. Do not change the `#[serde(default)]` behavior: an absent size is still
   `0`/default. Only a present out-of-range value is an error.
5. If there is a shared helper for numeric parsing in the file, keep the two
   paths (number and string) using the same `i64` bound.

## Tests

- Extend the existing deserialization test with:
  - `i64::MAX` as a JSON number and as a string: both accepted;
  - `i64::MAX + 1` as a JSON number: `Err`, message mentions the value;
  - `u64::MAX` as a JSON number: `Err`;
  - `"9223372036854775808"` as a string: `Err`;
  - a normal positive number and a normal numeric string still accepted.
- If a malformed `ManifestFile` can be driven through the mock artifact server,
  add one metadata-failure assertion there. Otherwise the unit tests are
  enough; the error mapping is already the metadata exit path.

## Acceptance

- No `u64` to `i64` cast remains in the number visitor.
- An out-of-range size produces a metadata error, never a negative value.
- Valid manifests parse exactly as before.
- `cargo test -p girpr` passes.

## Commit plan

1. `fix(hyp): reject manifest sizes that do not fit an i64`
2. `test(hyp): cover numeric and string size overflow`

## Notes

- The error is a parse error, so it surfaces through the existing
  `RunFailure::metadata` path with exit `2`. Do not invent a new exit code.
- Do not add a `u64` size type across the codebase; that is a larger change and
  is not needed to stop the silent corruption.
