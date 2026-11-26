# T12 — Protobuf → prost decode + prost-build codegen

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §APP-8 (lines 359–369) — the hand-rolled
  varint/wire decoder in dispatch becomes `prost::encoding::decode_varint` /
  `decode_key`.
- §LEAF-4 (lines 451–465) — the nine hand-written `#[prost(...)]` structs in
  `girpr-proto` become generated code via `prost-build` + a vendored `protoc`.
- Validate assumptions first: §A1 (15–16), §A2 (17–18), §A5 (26–28). Keep items:
  `guidance/DO-NOT-TOUCH.md` — the frozen dump keys are a pinned contract
  (§LEAF-4 lines 461–464).

## Scope
- `app/dispatch/src/proto.rs` — `read_varint` (`:89–98`), `walk_fields` (`:100–157`),
  error mapping and `walker_reads_every_wire_type_and_rejects_a_group` (`:256`).
- `crates/proto/src/lib.rs` — nine hand-written structs (`:29–166`), tests at
  `:175–190` and `:195–237`; `crates/proto/Cargo.toml:23–30`.
- A vendored pinned `.proto`, a new `build.rs`, and the affected manifests.

## Implementation plan
1. Replace only the decoding primitives in dispatch with prost's functions; keep
   `WireValue`/`OpaqueFields` and the "never used for placement" policy. Map
   `DecodeError` into `DispatchError::Proto`; keep the wire-type test and accept
   changed error text (§APP-8 lines 367–369).
2. Vendor the pinned upstream `Sophon.proto` and add a `build.rs` using
   `prost-build` with a vendored `protoc` (e.g. `protoc-bin-vendored`) so Windows and
   Linux need no system protobuf. Generate serde derive via a type attribute if the
   report's hint applies; keep explanatory docs as module comments.
3. Delete the hand-written structs; keep `tag_six_is_the_opaque_int64_passthrough`
   and `patch_manifest_delete_tag_round_trips`; add a regression for dump keys
   `chuncks` and `unknown_field6`.
4. Verify both crates build and the consumer tests pass; check that wire bytes and
   serde JSON keys are unchanged.

## Constraints / validation
- The misspelled `chuncks`, opaque tag-6 and serde JSON keys are pinned by frozen
  artifacts (§LEAF-4 lines 451–465). No wire or key change.
- Prost rejecting overlong encodings is acceptable (§APP-8 line 369).
- Adding a build script/protoc to the toolchain story is acceptable; verify the
  workspace still builds on both target platforms.

## Done when
No hand-written varint/tag decoder or hand-maintained protobuf struct remains, generated
code preserves the frozen keys and wire layout, and both crates' tests pass. If a
premise is stale or a keeper would change, **stop** and record `skipped`.
