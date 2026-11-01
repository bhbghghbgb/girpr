---
id: T18
title: Feature-gate sled and trim windows-sys features
batch: G
checkpoint: G
report: [L597-L610]
design_notes: "sled is used only by the one-shot converter; normal builds should not carry it"
depends_on: [T14, T17]
size: S
risk: low-medium
---

# T18 — Feature-gate `sled` and trim `windows-sys` features

## Goal

- Make `sled` an optional dependency behind a `convert` feature that also
  builds the `sled2redb` binary and its conversion tests.
- Remove `Win32_Security` from `windows-sys` if nothing in the crate uses it.

Normal `girsync` builds must not compile `sled` or the converter binary.
`cargo build --features convert --bin sled2redb` must still work.

## Why

Report §6: `sled` is used only by `src/convert.rs` and the `sled2redb` binary.
Feature-gating it removes a heavy dependency from normal builds and the shipped
binary. `Win32_Security` is enabled but the hard-link code only uses
`Win32_Foundation` and `Win32_Storage_FileSystem`; trimming unused features is
a small build win.

## Files to read

- `Cargo.toml` — dependencies, bins, tests
- `src/lib.rs` — module tree and `pub mod convert`
- `src/convert.rs` — the only `sled` user
- `src/bin/sled2redb.rs` — the converter binary
- `tests/convert.rs` — integration test that calls `sled::open`
- `src/hardlink.rs` — the `windows_sys` imports
- `README.md` — build/run instructions and the legacy-cache remedy

## Constraints

- Default features must not change `girsync`'s behaviour. The feature is
  additive and off by default.
- The converter binary and `tests/convert.rs` are available **only** with the
  feature; Cargo must encode that so `cargo test` without it does not try to
  build them.
- Keep the library API `girsync::convert::sled_to_redb` when the feature is on.
- Do not move conversion logic into the binary or duplicate it.
- Do not remove `sled` from `Cargo.lock`; it stays as an optional dependency.
- Verify `Win32_Security` is genuinely unused before removing it. If a
  transitive macro or a `windows-sys` module needs it, keep it and document
  why.

## Implementation

1. `Cargo.toml`:
   ```toml
   [features]
   default = []
   convert = ["dep:sled"]

   [dependencies]
   sled = { version = "0.34", optional = true }

   [[bin]]
   name = "sled2redb"
   path = "src/bin/sled2redb.rs"
   required-features = ["convert"]

   [[test]]
   name = "convert"
   path = "tests/convert.rs"
   required-features = ["convert"]
   ```
   Keep the existing `[[bin]] girsync`, `[lib]`, and dev-dependencies. If
   `tests/convert.rs` needs `sled` as a dev-dependency, add it there with
   `optional` semantics not available for dev-deps — instead rely on the
   feature enabling the normal optional dependency, which integration tests
   can use.
2. `src/lib.rs`: gate the module:
   ```rust
   #[cfg(feature = "convert")]
   pub mod convert;
   ```
   Check for any other `convert::` reference in the library and gate or
   remove it. `src/bin/sled2redb.rs` already only uses the public path, so it
   compiles under the feature.
3. Trim `windows-sys`:
   ```toml
   features = ["Win32_Foundation", "Win32_Storage_FileSystem"]
   ```
   Run `cargo check --target x86_64-pc-windows-msvc` (or the host) to prove
   nothing else needs `Win32_Security`. Search `rg "Win32_Security|Security"`
   first.
4. Update `README.md`:
   - the `sled2redb` build/run line adds `--features convert`;
   - the legacy-cache remedies that say `sled2redb` mention the feature;
   - a short “Build features” note: default build has no `sled`, `convert`
     adds the converter.
5. Verify the matrix:
   ```text
   cargo check -p girsync
   cargo check -p girsync --features convert
   cargo test -p girsync
   cargo test -p girsync --features convert --test convert
   cargo build -p girsync --features convert --bin sled2redb
   ```
   Confirm `cargo tree -p girsync` without the feature no longer contains
   `sled` (or that it is marked optional and not built).

## Commit plan

1. `build(sync): put sled and sled2redb behind a convert feature`
2. `build(sync): drop the unused windows-sys security feature`
3. `docs(sync): document the convert feature`
4. `test(sync): gate the converter tests on the feature`

## Verification

```text
cargo fmt --all -- --check
cargo check -p girsync
cargo check -p girsync --features convert
cargo test -p girsync
cargo test -p girsync --features convert --test convert
```

## Acceptance

- A default build has no `sled` compilation and no `sled2redb` binary.
- `--features convert` builds the binary and runs the converter tests.
- `girsync::convert::sled_to_redb` is available exactly when the feature is
  on.
- The Windows build still compiles after trimming `Win32_Security`.
- README tells a user how to build the converter.

## Reviewer focus

- Does a default `cargo test` try to build `tests/convert.rs`?
- Is `convert` gated in `lib.rs` as well as in Cargo?
- Is `sled` reachable from a non-convert code path?
- Was `Win32_Security` actually unused (not just unused by name)?
- Does the README still say `sled2redb` exists without qualification?
