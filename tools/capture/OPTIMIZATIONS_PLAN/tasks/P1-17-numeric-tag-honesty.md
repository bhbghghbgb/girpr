# P1-17 — Numeric and tag-length honesty

Report: `../../OPTIMIZATIONS_REPORT.md` lines 342-354 (A17) and 723-734 (D4).

Dependencies: none.

## Goal

Two small honesty fixes:

1. `IntOrStr::from(u64)` must not turn values above `i64::MAX` into negative
   numbers.
2. The tag-length diagnostic must not say "characters" when the rule counts
   bytes.

## Where to start

- `src/hyp/types.rs`: `IntOrStr` (around line 43), `From<u64>` (around line 78),
  `From<i64>`, `as_u64`, `as_i64`, serialization tests.
- `src/config/mod.rs`: `tag_shape_rule_inner` (around line 623) and its tests.
- `src/config/tests.rs` for tag-shape messages.

## Decision for `IntOrStr`

Keep the wire representation (`Int(i64) | Str(String)`). For `u64` values that
fit `i64`, produce `Int`. For values above `i64::MAX`, produce
`Str(v.to_string())`, which is the faithful form and round-trips through
`as_u64`. Do not change the serde representation or add a new variant.

## Decision for the tag length

Preserve the current 32-byte limit and change the message to say "32 bytes".
This is the minimal, behavior-preserving fix: no previously rejected tag
becomes accepted. If the permanent docs say "characters" in a normative way,
update that wording to bytes as well.

## Steps

1. Change `impl From<u64> for IntOrStr` to use `i64::try_from(v)`; on success
   `Int`, on failure `Str(v.to_string())`.
2. Add a constructor or helper if it makes the intent clearer, but keep the
   `From` impl for existing call sites.
3. Update `tag_shape_rule_inner`'s message to "longer than 32 bytes" and keep
   `tag.len() > 32`.
4. Search the permanent docs for "32 characters" and correct them if they
   describe this rule.
5. Do not change any other tag rule, and do not change the limit.

## Tests

- `IntOrStr::from(u64::MAX)` -> `Str("18446744073709551615")`;
  `as_u64()` returns `Some(u64::MAX)`.
- `IntOrStr::from(i64::MAX as u64)` -> `Int(i64::MAX)`.
- Serialization of the large value is a JSON string; a normal value remains a
  JSON number (or whatever the current serde representation is).
- Tag: a 33-byte ASCII tag is rejected with a message containing "32 bytes";
  a 32-byte tag is accepted; existing control/separator/drive tests unchanged.
- Add a multibyte tag case (for example 17 two-byte characters) and assert the
  current behavior plus the byte-count wording.

## Acceptance

- No silent wrap to negative for a large `u64`.
- The tag diagnostic matches the rule.
- No change to the accepted tag length.
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(hyp): keep large u64 values faithful in IntOrStr`
2. `fix(config): say the tag limit is bytes, as it is`
3. `test(hyp,config): cover large integers and multibyte tags`

## Notes

- Do not change `IntOrStr`'s serde shape; the string form is already accepted.
- Do not reference the report or plan in code comments.
