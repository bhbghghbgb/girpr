---
id: T10
title: Cache memory knob and one test-cache decision
batch: D
checkpoint: D
report: [L370-L388, L536-L540, L550-L555]
design_notes: "Tests need an explicit small cache; production should not pay for a hardcoded per-handle budget it cannot tune"
depends_on: []
size: S
risk: low
---

# T10 — Cache memory knob and one test-cache decision

## Goal

- Make the redb page-cache size configurable with `GIRSYNC_CACHE_SIZE_MIB`,
  resolved once, defaulting to the current 64 MiB.
- Resolve `GIRSYNC_TEST_CACHE` once in `CommonOpts::try_from`, so
  `open_folder_cache` no longer reads the environment per open and the
  precedence between the field and the variable is obvious.

## Why

Every redb handle uses a hardcoded 64 MiB. That is too much for a tiny cache
and may be too little for a multi-hundred-thousand-row one. Separately,
`common.test_cache` and `GIRSYNC_TEST_CACHE` are two sources for one answer;
reading the variable at every open makes the precedence accidental.

## Files to read

- `src/cache/db/backend.rs` — `CACHE_SIZE`, `redb_builder`
- `src/config/common.rs` — `CommonOpts`, `test_cache`
- `src/effective/side.rs` — the per-open `GIRSYNC_TEST_CACHE` read
- `src/cache/db/cachedb.rs` — opens that go through `redb_builder`
- `README.md` — cache and test sections

## Constraints

- The default must stay 64 MiB, because the test suite's parallel nextest
  processes depend on it.
- `GIRSYNC_TEST_CACHE=memory` keeps its exact meaning; values other than
  `memory` keep the current “file backend” behaviour.
- Invalid or zero `GIRSYNC_CACHE_SIZE_MIB` must not panic. Decide and document
  one rule: fall back to the default, or refuse at startup. Prefer the fallback
  with a one-time warning if a subscriber exists, and a pure parser that can be
  tested without mutating process env.
- Resolve both values once per process, not once per `CacheDb`.
- Do not change the cache file format, meta, or codec.

## Implementation

1. In `backend.rs`, add:
   ```rust
   pub(in crate::cache) fn cache_size_mib() -> usize {
       static SIZE: OnceLock<usize> = OnceLock::new();
       *SIZE.get_or_init(|| parse_cache_size_mib(std::env::var("GIRSYNC_CACHE_SIZE_MIB").ok()))
   }
   ```
   with a pure `parse_cache_size_mib(Option<String>) -> usize` that returns 64
   when absent/unparseable/zero, and `redb_builder` calling
   `set_cache_size(cache_size_mib() * 1024 * 1024)`. Keep the constant as
   `DEFAULT_CACHE_SIZE_MIB`.
2. Add unit tests for the parser: absent, valid, zero, non-numeric, and a very
   large value (must not overflow `usize` on 32-bit; saturate or document the
   `usize::try_from` bound).
3. In `CommonOpts::try_from`, resolve:
   ```rust
   let test_cache = if std::env::var("GIRSYNC_TEST_CACHE").as_deref() == Ok("memory") {
       TestCacheBackend::Memory
   } else {
       TestCacheBackend::File
   };
   ```
   and set `CommonOpts.test_cache` to it. Record the precedence in the field
   doc: field is the resolved answer; the environment is read only here.
4. In `open_folder_cache`, replace the per-call environment read with
   `common.test_cache == TestCacheBackend::Memory`. Remove the local
   `want_memory` expression.
5. Update `README.md`:
   - a short “Cache memory” note naming `GIRSYNC_CACHE_SIZE_MIB`, its default
     64, and that it is per handle;
   - the test-only `GIRSYNC_TEST_CACHE=memory` mention, saying it is resolved
     once at command start.

## Commit plan

1. `feat(cache): make the redb cache size configurable`
2. `refactor(config): resolve the test cache backend once`
3. `docs(sync): document the cache size knob and test backend`
4. `test(cache): pin the cache size parser`

## Verification

```text
cargo fmt --all -- --check
cargo test -p girsync cache::
cargo test -p girsync
GIRSYNC_CACHE_SIZE_MIB=8 cargo test -p girsync cache::
```

## Acceptance

- The default page cache is still 64 MiB.
- `GIRSYNC_CACHE_SIZE_MIB=128` is visible to every redb builder in the process.
- `GIRSYNC_TEST_CACHE=memory` still switches folder caches to the shared
  in-memory backend, and is read once.
- An invalid value cannot panic or overflow.
- No cache format or meta change.

## Reviewer focus

- Is the environment read exactly once per process for each knob?
- Does the field win over the environment consistently?
- Does a test that constructs `CommonOpts` directly still get `File`?
- Is the 64 MiB default preserved for the nextest workload?
- Is the invalid-value rule documented and tested?
