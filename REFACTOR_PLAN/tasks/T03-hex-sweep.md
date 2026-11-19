# T03 — Hex encode/decode sweep

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §A3 (lines 110–130) — the
  sweep and its site list.
- Related: §F6 (lines 684–686), §G2 (lines 772–776).

## Scope
- Packages: root `girpr`, `tools/capture`, `tools/sled2redb`, plus the test
  harnesses named in §A3.

## Implementation plan
1. Adopt the `hex` crate (`encode`, `encode_upper`, `decode`, `decode_to_slice`)
   and keep existing public wrapper names as thin shims so call sites do not
   change.
2. girpr `md5.rs`: keep `parse_hex`'s fixed `[u8;16]` and its "not a hex MD5"
   message, implemented with `decode_to_slice`.
3. sled2redb `hex_to_bytes`: preserve the
   `.with_context(|| format!("decode {algo} hex of {rel}"))` wrapping.
4. gircap hash wrappers (`hex_bytes`, `hex_bytes_upper`, `hex_with`) and the test
   harness `md5_hex` per-byte `format!("{b:02x}")` anti-pattern use the crate.

## Constraints / validation
- Digest hex output is byte-identical; `hex::decode` error wording may differ.
- Tests named in §A3; keep the bad-filename/algorithm context in sled2redb so
  its tests still see it.

## Done when
No hand-rolled hex conversion remains at the listed sites and the named tests
pass.
