# Tenth adversarial pass — 2026-10-29

Baseline: branch `fix-gaps/gircap` at `385c4af` (the ninth-pass fixes
`88407c0` and `47b0dc2` are in). The pass re-derived the fail-fast and
no-omitted-errors rules and looked for sites the ninth-pass rule missed, not
for the trade-offs already listed in
[OPEN_FOR_MAINTAINERS.md](OPEN_FOR_MAINTAINERS.md).

## Verdict

| ID | Severity | Class | Location | Summary |
| --- | --- | --- | --- | --- |
| N1 | Low | bug (omitted error cause) / test gap | `src/blobs/mod.rs:530-534`, `src/run/mod.rs:321-325` | The two direct `spawn_blocking` joins format `JoinError` with `Display`, so a panic's payload is discarded and a cancellation is reported as a panic. The M2 rule reached the two `JoinSet` loops but not these two joins. **Fixed** in `1300931` (measurement) and `a84d3c5` (request log). |

## N1 — A panicking `spawn_blocking` task loses its payload

### The code

`src/blobs/mod.rs:525-541`:

```rust
let path = final_path.clone();
// `measure_file` answers `Ok` for a body that is not a zstd frame: that
// is a measured, unusable row, not a failed fetch. Only a real I/O error
// reaches here, and the blob is removed so the index's "not stored" row
// and the directory cannot disagree. A later source may then retry.
let props = match tokio::task::spawn_blocking(move || {
    BlobProps::measure_file(&path, &chunk, digests)
})
.await
.map_err(|e| format!("{id}: measure task panicked: {e}"))?
{
    Ok(props) => props,
    Err(e) => {
        Self::discard_published(&final_path);
        return Err(e.to_string());
    }
};
```

`src/run/mod.rs:318-327`:

```rust
{
    let for_the_pool = log.clone();
    let dir_for_the_pool = dir.clone();
    tokio::task::spawn_blocking(move || {
        emit::write_requests_log(&dir_for_the_pool, &for_the_pool)
    })
    .await
    .map_err(|e| write_err(&dir, std::io::Error::other(format!("{e}"))))?
    .map_err(|e| write_err(&dir, e))?;
}
```

Both use `format!("{e}")` on a `tokio::task::JoinError`. The crate's own
`src/tasks.rs:12-14` states why that is lossy:

> **`JoinError`'s `Display` does not include the panic payload.** It says only
> that the task panicked, so a message the failing task wrote — the reason an
> operator actually needs — has to be downcast out of `into_panic`.

M2 fixed exactly this in `map_concurrently`, and `join_failure` is already the
rule for the two `JoinSet` loops (`src/blobs/mod.rs:325`,
`src/http/fetcher.rs:444`). These two joins bypass it.

### Concrete failure scenarios

1. **Measurement.** `BlobProps::measure_file` decodes an untrusted network body
   through the `zstd` decoder. If that decode (or anything added to the
   measurement path later) panics, `spawn_blocking` returns the panic as a
   `JoinError`. The row and the run's error list then read:

   `chunk <id> via <tag>: chunk <id>: measure task panicked: task <id> panicked`

   The panic message — which chunk, which offset, which frame — is gone. Unlike
   the `Store::fill` arm at `src/blobs/mod.rs:323-328`, this site does not even
   log the payload first. The `Err(..)` arm also never runs, so the blob already
   renamed to `chunks/blobs/<id>.zst` stays on disk while the index row is
   `stored_path: null`. That contradicts the `discard_published` invariant at
   `src/blobs/mod.rs:567-570` ("the index row must either carry them or the
   directory must not"); an I/O error removes the blob, a panic does not.
2. **Request log.** A panic inside `emit::write_requests_log` (a future
   `expect`/`assert`, or a formatting failure) is reported as
   `write <bundle>: task <id> panicked` through `write_err`, again without the
   payload. A `spawn_blocking` task can also be cancelled during runtime
   shutdown; the fixed text calls that a panic, which is the confusion M2's
   helper removed at the other call sites.

### Why the standard fails

- **No omitted errors.** The only surviving text is `Display`, which the crate
  documents as payload-free. The cause is omitted, not merely summarised.
- **Consistency.** The project decided in M2 that a `JoinError` must be run
  through `join_failure`; these two sites silently do otherwise.
- **Test gap.** No test injects a panicking blocking closure at either site.
  The `tasks::tests` cases exercise the helper, not these mappings, so a
  regression here would pass the suite.

### Suggested fix

Route both through the existing helper, and let the blob path keep its
disk/index agreement:

```rust
// src/blobs/mod.rs
let props = match tokio::task::spawn_blocking(move || {
    BlobProps::measure_file(&path, &chunk, digests)
})
.await
{
    Ok(Ok(props)) => props,
    Ok(Err(e)) => {
        Self::discard_published(&final_path);
        return Err(e.to_string());
    }
    Err(e) => {
        Self::discard_published(&final_path);
        return Err(format!("{id}: {}", crate::tasks::join_failure(e, "measure")));
    }
};
```

```rust
// src/run/mod.rs
.map_err(|e| {
    write_err(
        &dir,
        std::io::Error::other(crate::tasks::join_failure(e, "request log write")),
    )
})?
```

If keeping a panicked measurement's bytes is deliberate (F11 accepts orphans
from panics), say so next to the arm; the current comments imply the directory
and index always agree. A single `join_blocking` wrapper in `src/tasks.rs`
would give the next `spawn_blocking` site the rule for free. Add one test per
site that spawns a closure which panics with a known message and asserts that
message reaches the returned error.

### Resolution

Validated against current source: both joins did bypass `join_failure`, and the
measurement `Err` arm did skip `discard_published`. Fixed at the root:

- `src/tasks.rs:40` adds `join_blocking`, which spawns blocking work and maps a
  `JoinError` through `join_failure` with the site's label.
- `src/blobs/mod.rs:531-568` replaces the direct join with `measure_published`.
  Both non-returning and I/O-error arms call `discard_published`, so a
  published blob cannot outlive a `stored_path: null` row; the panic arm keeps
  `<chunk id>: ` context. `discard_published`'s doc (`:598`) now states the
  panic/cancellation case. Commit `1300931`.
- `src/run/mod.rs:321-326` routes the request-log write through
  `write_requests_log_on_the_pool` (`:831`), which uses `join_blocking` with the
  `"request log write"` label; `write_err` still names the bundle. Commit
  `a84d3c5`.
- Tests: `tasks::tests::a_panicking_blocking_task_reports_its_payload` and
  `a_returning_blocking_task_keeps_its_value` (`src/tasks.rs:81`),
  `blobs::tests::a_panicking_measurement_discards_the_published_blob_and_keeps_the_cause`
  (`src/blobs/mod.rs:1476`, asserts the payload, the blob id and that the
  seeded published file is gone), and
  `run::tests::a_panicking_request_log_task_reports_its_payload`
  (`src/run/mod.rs:914`). The test seams are the closures the production call
  sites pass; `BlobProps::measure_file` and `emit::write_requests_log` still
  have no input that panics on demand.

## Verification

After the fixes (`a84d3c5`), Windows (`win32`):

- `cargo build -p gircap` — `Finished \`dev\` profile [unoptimized + debuginfo] target(s) in 23.78s`;
  no warnings.
- `cargo test -p gircap` — 701 passed, 0 failed, 0 ignored, 0 filtered out:
  557 lib + 3 + 8 + 32 + 2 + 30 + 10 + 29 + 15 + 15, plus 0 doc-tests. No
  `#[cfg(unix)]` test was skipped; the `#[cfg(windows)]` tests ran.
- `cargo clippy -p gircap --all-targets` — `Finished \`dev\` profile ... in 9.51s`;
  no warnings or errors (exit 0).
- `cargo fmt -p gircap --check` — no diff (exit 0).

The pre-fix review numbers were 696 passed (552 lib + 144 integration) at
`385c4af`; the five new tests are the two blocking-join helper tests, the
measurement seam test, the request-log seam test and the request-log value test.

## Not verified

- No real panic was injected into `BlobProps::measure_file` or
  `emit::write_requests_log`; neither has a portable input that panics on
  demand. The tests drive the exact closures the production call sites use,
  with a panicking body, so the mapping and the discard branch are exercised,
  but the panicking instruction itself is not the real decoder or writer.
- `spawn_blocking` cancellation at these two sites was not driven; the helper's
  cancellation text is covered by
  `tasks::tests::a_cancelled_task_is_not_reported_as_a_panic`, and
  `join_blocking` routes through the same `join_failure`. A cancelled
  measurement is reasoned to remove the published blob, not observed.
- The orphaned-blob consequence of a measure-task panic was reproduced as a
  seeded published file at the seam, not through a full `Store::fill` run whose
  decoder panicked.

## Considered and dismissed

- `probe_manifest`'s `v as u32` (`src/verify/mod.rs:410`) truncates a crafted
  `schema_version: 4294967297` to `1`. No false acceptance follows:
  `Bundle::open` reads the same file into a `u32` field and the parse fails, and
  `list` treats the directory as an unreadable bundle rather than skipping it.
- `Store::fill`'s `join_next` without a task id (`src/blobs/mod.rs:308`) still
  gives a panicking chunk the generic row text, but F4 records that trade-off
  and the payload is logged at `:324-327`; not re-reported.
- The plain `.sum()` calls on self-measured totals (`src/emit/index.rs:369`,
  `src/run/mod.rs:414`) do not consume a document; M1's untrusted fold is
  `checked_add`-based and reports overflow before equality.
