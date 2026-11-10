---
id: T18
title: Feature-gate sled
batch: G
checkpoint: G
report: [L597-L610]
design_notes: "sled is used only by the one-shot converter; normal builds should not carry it"
depends_on: [T14, T17]
size: S
risk: low-medium
---

# T18 — Feature-gate `sled`

## Goal

Make `sled` an optional dependency behind a `convert` feature that also
builds the `sled2redb` binary and its conversion tests.

Normal `girsync` builds must not compile `sled` or the converter binary.
`cargo build --features convert --bin sled2redb` must still work.

## Why

`sled` is used only by the one-shot converter: `src/convert.rs` and
`src/bin/sled2redb.rs`. Feature-gating it removes a heavy dependency from
normal builds and from the shipped `girsync` binary.

## The `windows-sys` half of this task is withdrawn

The original task also proposed trimming the `windows-sys` feature list, on the
assumption that `Win32_Security` was unused. **That assumption is wrong.**
In `windows-sys` 0.61 the `CreateFileW` binding is gated behind
`#[cfg(feature = "Win32_Security")]`, and `src/hardlink.rs` calls
`CreateFileW`. Dropping the feature breaks the hard-link count on Windows.

The crate happens to build today because `clap`'s dependency tree enables
`windows-sys/Win32` transitively — but that is an accident of the current
graph, not a licence to depend on it. **Keep all three features**
(`Win32_Foundation`, `Win32_Security`, `Win32_Storage_FileSystem`) exactly as
they are, and do not add a note claiming the feature is unnecessary.

## Files to read

- `Cargo.toml` — dependencies, the two `[[bin]]` entries, `[lints]`
- `src/lib.rs` — the module tree, and `pub mod convert`
- `src/convert.rs` — the only `sled` user in the library
- `src/bin/sled2redb.rs` — the converter binary
- `tests/convert.rs` — the integration test that calls `sled::open` directly
- `README.md` — build/run instructions and the legacy-cache remedy

## Constraints

- Default features must not change `girsync`'s behaviour. The feature is
  additive and off by default.
- The converter binary and `tests/convert.rs` are available **only** with the
  feature; Cargo must encode that so a plain `cargo nextest run -p girsync`
  does not try to build them.
- Keep the library API `girsync::convert::sled_to_redb` when the feature is
  on. Do not move conversion logic into the binary or duplicate it.
- **Do not remove `sled` from `Cargo.lock`.** It stays as an optional
  dependency.
- `tests/convert.rs` calls `sled::open` directly, so it needs `sled` as a
  dev-dependency as well as an optional normal dependency. An optional
  dev-dependency is not expressible; the usual shape is to declare `sled`
  optional in `[dependencies]` **and** list it in `[dev-dependencies]`
  unconditionally, or to give the test target `required-features` and rely on
  the feature. Pick one and say which in the commit message.
- Leave the `windows-sys` features alone (see above).
- Leave `md-5`, `redb = "=4.1.0"`, `rust-version = "1.89"`, and the
  `[lints]` sections untouched.
- Do not change the `girsync` binary's behaviour in any way.

## Implementation

1. `Cargo.toml`:
   - `sled = { version = "0.34", optional = true }` in `[dependencies]`.
   - A `[features]` table with `default = []` and
     `convert = ["dep:sled"]`.
   - Add `required-features = ["convert"]` to the **existing** `[[bin]]`
     `sled2redb` entry. Do not add a second `[[bin]]` block for it — the
     manifest already declares that binary.
   - The `girsync` `[[bin]]` entry stays as it is.
   - `tests/convert.rs` is auto-discovered as an integration test named
     `convert`. To gate it, declare an explicit `[[test]]` entry for it with
     `required-features = ["convert"]`, and add `sled` to `[dev-dependencies]`
     so the test can still call `sled::open`.

2. `src/lib.rs`: gate the module:

   ```rust
   #[cfg(feature = "convert")]
   pub mod convert;
   ```

   Check for any other `convert::` reference in the library and gate or
   remove it. `src/bin/sled2redb.rs` already only uses the public path, so it
   compiles under the feature.

3. Update `README.md`:
   - the `sled2redb` build/run line adds `--features convert`;
   - the legacy-cache remedies that say `sled2redb` mention the feature;
   - a short "Build features" note: the default build has no `sled`, and
     `convert` adds the converter.

4. Verify the matrix:

   ```text
   cargo check -p girsync
   cargo check -p girsync --features convert
   cargo nextest run -p girsync
   cargo nextest run -p girsync --features convert --test convert
   cargo build -p girsync --features convert --bin sled2redb
   ```

   Confirm `cargo tree -p girsync` without the feature no longer contains
   `sled` (or that it is marked optional and not built).

## Commit plan

1. `build(sync): put sled and sled2redb behind a convert feature`
2. `docs(sync): document the convert feature`
3. `test(sync): gate the converter tests on the feature`

## Verification

```text
cargo fmt --all -- --check
cargo check -p girsync
cargo check -p girsync --features convert
cargo nextest run -p girsync
cargo nextest run -p girsync --features convert --test convert
```

## Acceptance

- A default build has no `sled` compilation and no `sled2redb` binary.
- `--features convert` builds the binary and runs the converter tests.
- `girsync::convert::sled_to_redb` is available exactly when the feature is
  on.
- The `windows-sys` feature list is unchanged.
- `Cargo.lock` still contains `sled`.
- README tells a user how to build the converter.

## Reviewer focus

- Does a default `cargo nextest run -p girsync` try to build
  `tests/convert.rs`?
- Is `convert` gated in `lib.rs` as well as in Cargo?
- Is `sled` reachable from a non-convert code path?
- Did the `windows-sys` features survive the edit unchanged?
- Is there exactly one `[[bin]]` entry for `sled2redb`?
- Does the README still say `sled2redb` exists without qualification?
