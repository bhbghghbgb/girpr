# ADVERSARIAL REVIEW — ninth pass (`gircap`)

**Baseline:** HEAD `6561044` (`docs(review): fold the eighth pass into the consolidated review`), branch `fix-gaps/gircap`, tree clean before this note. Reviewed 2026-10-29.

**Method:** this pass asked the eighth pass's question in reverse: do the rules
fixed in L1/L2 (and the earlier F4 panic rule they sit beside) hold at every
*sibling* site, and did the fixes leave a new gap? Full re-reads of
`emit/index.rs`, `verify/mod.rs`, `http/fetcher.rs`, `blobs/mod.rs`,
`select/{mod,budget}.rs`, `config/mod.rs`, `paths.rs`, `run/mod.rs`,
`manifest/*`, `hyp/mod.rs` and `http/probe.rs`; `rg` for plain sums, `let _ =`,
`if let Ok` over a `JoinError`, and every `map_concurrently` caller. The four
standard checks were run on this tree (results at the end). No source, test,
doc, README or `Cargo.toml` change was made; this note is the only file added.

Both findings are **Low** and both are the same shape: a rule the project has
already stated and implemented once, applied at one site but not its sibling.

## Verdict

| ID | Severity | Class | Location | One-line summary |
|----|----------|-------|----------|------------------|
| M1 | Low | bug | `src/emit/index.rs:437` (read boundary `src/verify/mod.rs:178`, call `:538`) | `Checksums::findings` sums untrusted `entries[].bytes` with plain `.sum()`, so a crafted `verify/checksums.json` panics `gircap verify` in debug and wraps in release, where it can satisfy the `total_bytes` check. |
| M2 | Low | bug | `src/http/fetcher.rs:430` (sibling fix `src/blobs/mod.rs:323`, `:642`) | `map_concurrently` discards the `JoinError` payload and reports every non-returning task as `"a request task panicked"`, so a panicking metadata job loses its cause and a cancellation is mislabelled. |

Neither finding was rejected. M1 is the L2 class at the one remaining untrusted
total; M2 is the F4 panic rule at the other `JoinSet` loop.

**Closing work (2026-10-29, branch `fix-gaps/gircap`):** both findings were
re-validated against the current source, reproduced where a fixture was
possible, fixed and pinned — M1 `88407c0`, M2 `47b0dc2`. Each section below
carries the evidence and the precise status; the note is folded into
`ADVERSARIAL_REVIEW.md` and deleted at the end of this pass.

---

### M1 — A crafted `checksums.json` makes A16 panic on a plain sum

**Severity: Low. Class: bug. Location:** `src/emit/index.rs:437`; read
boundary `src/verify/mod.rs:178`; call `src/verify/mod.rs:538`. Trusted
siblings: `src/emit/index.rs:173`, `:369`.

`Checksums` is deserialized straight from the bundle's own document:

```rust
// src/emit/index.rs:291-304
/// `verify/checksums.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checksums {
    /// The artifact schema version.
    pub schema_version: u32,
    /// Which digest `sha256` is, stated so a reader need not assume.
    pub algorithm: String,
    /// Every file, sorted by path, excluding this document itself.
    pub entries: Vec<FileEntry>,
    /// How many.
    pub file_count: u64,
    /// `Σ bytes`.
    pub total_bytes: u64,
}
```

and `FileEntry::bytes` is an unchecked `u64` (`src/emit/index.rs:184-188`).
`Bundle::open` parses the document at `src/verify/mod.rs:178` and then validates
only the *path* fields it knows about:

```rust
// src/verify/mod.rs:189
validate_document_paths(&manifest, &index, &patchmaps)?;
```

`validate_document_paths` covers `MANIFEST.json capture.id`, the index's
`stored_path`/`referenced_by[].path` and each patchmap's `path`
(`src/verify/mod.rs:305-337`). It is not given `checksums`, so
`entries[].bytes` reaches `findings` unvalidated. The total there is a plain
`Sum`:

```rust
// src/emit/index.rs:430-439
if self.file_count as usize != self.entries.len() {
    out.push(format!(
        "file_count {} but {} entries",
        self.file_count,
        self.entries.len()
    ));
}
if self.total_bytes != self.entries.iter().map(|e| e.bytes).sum::<u64>() {
    out.push("total_bytes disagrees with the entries".into());
}
```

`reverify` calls it before any digest check and promises not to fail:

```rust
// src/verify/mod.rs:526-538
/// Re-derive every claim the bundle's own bytes can support.
///
/// Never fails. A bundle that cannot even be opened fails in [`Bundle::open`];
/// a bundle that opens and then does not hold is the *finding*, and returning it
/// as a report rather than an error is what lets the summary name every broken
/// row instead of only the first.
pub fn reverify(bundle: Bundle) -> Report {
    let root = bundle.root.clone();
    let mut rows: Vec<Assertion> = Vec::new();
    let mut findings: Vec<String> = Vec::new();

    // 1. A16.
    let found = bundle.checksums.findings(&root);
```

The write-side siblings are safe because their entries are measured, not read
back: `build_manifest` sums `files` it just digested
(`src/emit/index.rs:173`) and `scan_reusing` sums `digest_file` output
(`src/emit/index.rs:369`). Line 437 is the only one of the three that folds a
document supplied by whoever handed the operator the bundle.

**Scenario.** A bundle is copied or edited before `verify` (the command's own
premise: `verify` runs on "a bundle they may not have produced",
`src/verify/mod.rs:183-184`). Set two entries' `bytes` to `u64::MAX` and
`total_bytes` to `u64::MAX - 1`:

```json
{"schema_version": 2, "algorithm": "sha256",
 "entries": [{"path": "a", "bytes": 18446744073709551615, ...},
             {"path": "b", "bytes": 18446744073709551615, ...}],
 "file_count": 2, "total_bytes": 18446744073709551614}
```

`gircap verify --artifact <dir>` opens the bundle, `reverify` calls
`findings`, and the sum overflows. On a debug build this is
`attempt to add with overflow` (exit `101`, no summary); on release it wraps to
`u64::MAX - 1`, which equals the crafted `total_bytes`, so the
`"total_bytes disagrees with the entries"` finding is suppressed as well. The
per-entry digest comparisons still catch entries that do not match real files,
but the crash happens before any of them, and an impossible total can be
certified instead of named.

**Why the standard fails.** `reverify`'s own contract is "Never fails", and
`fetcher.rs:592` states the project rule that "no production path in this tool
can panic". More specifically, K1's binding decision is that *measurement
totals saturate* (`ADVERSARIAL_REVIEW.md:657`, `:693`), and the eighth pass
fixed exactly this class in `bytes_admitted`/`total_admitted` (L2,
`ADVERSARIAL_REVIEW.md:668`, `:679`). A byte total read from an untrusted
document is the same kind of number; the one site left on plain `.sum()` is
also the one site whose input is not the tool's own measurement. This is also
the "strict boundary validation" rule: a field that becomes arithmetic must be
checked at the door, and `validate_document_paths` already establishes the door
for the other document-supplied fields.

**Suggested fix.** Fold with `checked_add` and turn an overflow into a finding
rather than a panic (saturating would make the crafted `total_bytes` above
compare equal, so checked is the right shape for an equality check):

```rust
match self
    .entries
    .iter()
    .try_fold(0u64, |total, e| total.checked_add(e.bytes))
{
    Some(sum) if sum == self.total_bytes => {}
    Some(_) => out.push("total_bytes disagrees with the entries".into()),
    None => out.push("entries' bytes overflow u64".into()),
}
```

Add one unit test with two `u64::MAX` entries that asserts `findings` returns
the overflow finding without panicking (and a release run of it, since that is
where the wrap currently hides). If the project prefers a saturating fold for
consistency with K1, that also removes the panic, but then the comparison must
not treat `u64::MAX` as agreement.

**Status: fixed — `88407c0` (`fix(verify): saturate the untrusted checksums total`).**
Reproduced first: with the pre-fix `.sum()`, the new unit test
`emit::index::tests::a_crafted_checksums_total_cannot_overflow_or_certify_an_impossible_sum`
panicked in debug at `core/src/iter/traits/accum.rs:206:1` (`attempt to add with
overflow`), and the release run of the same test reported a wrapped sum instead
of the overflow. The fix folds with `checked_add`, keeps the L2 *saturating*
figure for reporting, and turns the overflow into its own finding
(`entries' bytes overflow u64; the total saturates at u64::MAX`). Because the
overflow branch is taken before the equality test, a crafted
`total_bytes: u64::MAX` cannot certify an overflowing sum; an exact `u64::MAX`
sum with no overflow remains legal, and `u64::MAX - 1` is still a disagreement.
The test covers the review's two-`u64::MAX` wrapping pair, the two
`u64::MAX - 1` pair, the saturated boundary, the exact boundary, and one byte
below it, and passes in both profiles. No change was needed at
`src/verify/mod.rs:178`: `reverify` now reports the crafted document as an A16
finding rather than failing to open it, which is its stated contract.

---

### M2 — The other `JoinSet` loop still throws the panic payload away

**Severity: Low. Class: bug. Location:** `src/http/fetcher.rs:430`, `:441-451`;
sibling fix `src/blobs/mod.rs:323-328`, `:642-653`; callers
`src/manifest/fetch.rs:126`, `src/hyp/mod.rs:335`, `:386`,
`src/http/probe.rs:157`, `:558`.

The chunk-store loop handles the panic arm explicitly and logs the payload:

```rust
// src/blobs/mod.rs:318-328
// A panicking task takes its slot number with it, so its hole is
// filled below. Pushing a second, slotless error here would count
// the same failure twice, which is the bug this function had.
// The JoinError itself would be dropped here, so name the panic
// in the log before falling through to the generic row text.
Err(e) => {
    tracing::error!(
        error = %join_failure(e),
        "a chunk fetch task did not report; its slot is filled below"
    );
}
```

with the helper and its reason:

```rust
// src/blobs/mod.rs:633-653
/// The most useful text a [`tokio::task::JoinError`] carries.
///
/// **`JoinError`'s `Display` does not include the panic payload.** It says only
/// that the task panicked, so a message the failing task wrote — the reason an
/// operator actually needs — has to be downcast out of [`into_panic`]. The
/// index row stays generic on purpose (it is a stable document), but the log
/// must not throw the cause away with the task.
fn join_failure(e: tokio::task::JoinError) -> String {
    if !e.is_panic() {
        return format!("the chunk fetch task was cancelled: {e}");
    }
    let payload = e.into_panic();
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a non-string panic payload".to_string());
    format!("the chunk fetch task panicked: {message}")
}
```

and a test pins it (`src/blobs/mod.rs:802-816`). The other `JoinSet` loop is
the metadata path, and it uses `if let Ok`:

```rust
// src/http/fetcher.rs:429-439
while let Some(joined) = set.join_next().await {
    if let Ok((i, r)) = joined {
        returned[i] = true;
        out[i] = Some(r);
    }
    if next < n {
        let i = next;
        next += 1;
        set.spawn(f(self.clone(), i, items[i].clone()));
    }
}
```

The hole is then filled with a fixed string that does not distinguish a panic
from a cancellation:

```rust
// src/http/fetcher.rs:441-451
out.into_iter()
    .enumerate()
    .map(|(i, r)| {
        r.unwrap_or_else(|| {
            Err(if returned[i] {
                format!("item {i} was dispatched but never returned")
            } else {
                "a request task panicked".to_string()
            })
        })
    })
    .collect()
```

`map_concurrently`'s doc already says a panic is recorded, but only as an
`Err` at that index (`src/http/fetcher.rs:388-390`); nothing names the cause.
Every metadata fetch goes through this loop: manifest/patchmap downloads
(`src/manifest/fetch.rs:126`), hypothesis and plain bodies
(`src/hyp/mod.rs:335`, `:386`), and error-shape probes
(`src/http/probe.rs:157`, `:558`).

**Scenario.** A metadata job panics (an internal invariant, a slice index, an
`unwrap` in a helper — the same kind of defect the F4 test simulates with
`panic!("chunk store exploded")`). `JoinSet` yields
`Err(JoinError { is_panic: true, .. })`; the `if let Ok` arm is skipped,
`returned[i]` stays `false`, and the slot becomes
`"a request task panicked"`. The panic message, the only text that says *why*,
is dropped, and no log line carries it. A task that was merely cancelled gets
the identical text. Compare `Store::fill`, where the same event is logged with
`join_failure(e)` before the generic row is written.

**Why the standard fails.** The review standard forbids an `if let Ok` that
discards a meaningful failure, and F4's fix is precisely the decision that a
`JoinError`'s payload must not be thrown away
(`ADVERSARIAL_REVIEW.md:595`, `:575`). `join_failure`'s own doc says "the log
must not throw the cause away with the task". That rule is implemented in
`blobs` and not in the sibling loop in `http`, so the F4 fix does not hold at
the site a ninth pass is supposed to check. The conflation of cancellation
with panic is the second half of the same omission.

**Suggested fix.** Match the join result and log the cause, reusing a
generalised helper:

```rust
while let Some(joined) = set.join_next().await {
    match joined {
        Ok((i, r)) => {
            returned[i] = true;
            out[i] = Some(r);
        }
        Err(e) => tracing::error!(
            error = %join_failure(e),
            "a request task did not report; its slot is filled below"
        ),
    }
    // ...
}
```

Move `join_failure` (or a parameterised version of it) to a shared module so
both loops use one rule; `blobs`'s "chunk fetch task" wording becomes a label
argument. Because `map_concurrently`'s slot string is what reaches
`fetch_errors`/the summary rather than a stable artifact row, put the payload in
the slot error too, or at minimum log it. Add one test that drives a panicking
job through `map_concurrently` and asserts the cause is visible (the sibling of
`a_panicking_chunk_task_reports_its_panic_message`); a cancellation case should
not say "panicked".

**Status: fixed — `47b0dc2` (`fix(http): keep the cause and index of a failing
request task`).** The `JoinSet` loop now joins with the task id
(`join_next_with_id`, `src/http/fetcher.rs:433`), maps the id back to the spawn's
item index (`:431`, `:467`) before the error is consumed, and records
`item {i} did not return: {cause}` in that slot; the log line carries both the
index and the cause. `join_failure` moved to `src/tasks.rs:18` with a label
argument, so `blobs::Store::fill` (`src/blobs/mod.rs:325`, "chunk fetch") and
`map_concurrently` ("request") share one rule; a cancellation is named as a
cancellation, not a panic. Pinned by
`http::tests::a_panicking_request_task_keeps_its_index_and_cause` (real
`map_concurrently`, panic in the middle job, asserts index, payload and that the
other jobs still run) and by the helper's own tests in `src/tasks.rs`
(payload downcast; an aborted task is reported as cancelled and not as
panicked). Coverage limit: cancellation is exercised on the helper, not through
`map_concurrently` itself, because the loop has no cancellation source — every
spawned task is awaited and the `JoinSet` is never aborted.

---

## What held

The eighth pass's fixes do hold at the sites they named:

- **L1.** `tag_shape_problem` calls the shared `paths::is_windows_device_name`
  (`src/config/mod.rs:618`), so the flag path, `VersionPair::select`
  (`src/config/mod.rs:121`) and `manifest::dump::write` (`src/manifest/dump.rs:94`)
  all share one rule; `CON`/`con`/`NUL.txt`/`COM1.bin`/`CONIN$` are refused and
  `CONSOLE`/`COM0`/`LPT10` stay legal (`config::tests::a_tag_cannot_name_a_windows_device`).
- **L2.** `Selection::bytes_admitted` (`src/select/mod.rs:805-807`) and
  `budget::total_admitted` (`src/select/budget.rs:128-135`) fold with
  `saturating_add`; the two-maximum-cost select test and the budget test pass in
  debug, and the debug assertion tying the total to the charged ledger holds by
  construction.

Also re-checked without a finding: `Budget::charge`/`admits` still saturate
(`src/select/budget.rs:51-64`); chunk retry/temp cleanup in `Store::fill` and
`Store::one`; error-body drain keeps the status and counts truncated bodies
(`src/http/fetcher.rs:809-849`); `attempt` builds a record on every branch
(`src/http/fetcher.rs:948-949`); `run/mod.rs:414`'s `bytes_fetched` sums the
capture's own measured index, not a document; `verify`'s other arithmetic
(`rebuilt_bytes`, `src/verify/mod.rs:516-523`) sums in-memory lengths, not
document fields; `emit/index.rs:863` is a test.

## Verification

Run on this closing tree (`47b0dc2` plus the doc commit; both fixes in place):

| Command | Result |
|---|---|
| `cargo build -p gircap` | `Finished \`dev\` profile ... in 18.27s`; no warnings; exit `0` |
| `cargo test -p gircap` | **696 passed, 0 failed, 0 ignored** (552 lib + 144 integration; main and doc-tests 0); exit `0` |
| `cargo clippy -p gircap --all-targets` | `Finished \`dev\` profile ... in 0.72s`; no diagnostics; exit `0` |
| `cargo fmt -p gircap --check` | no output; exit `0` |

Integration total: 3 + 8 + 32 + 2 + 30 + 10 + 29 + 15 + 15 = 144.

Release-only checks: `cargo test -p gircap --release --lib
a_crafted_checksums_total` (1 passed), `... --lib tasks::tests` (2 passed), and
`... --lib a_panicking_request_task` (1 passed). The pre-fix reproduction was
also run: debug `cargo test -p gircap --lib a_crafted_checksums_total` panicked
at `accum.rs:206:1`, and the release run reported a wrapped total.

## Not verified

- **M1 bundle through the command.** The crafted document is exercised directly
  through `Checksums::findings` in the unit test above, not written into a
  temporary bundle and run through `gircap verify`. The `Bundle::open` →
  `reverify` → `findings` path is unchanged and is a straight call
  (`src/verify/mod.rs:178`, `:538`).
- **M1 release certification pair.** The pre-fix debug panic was observed
  directly. The pre-fix release run observed a wrapped sum
  (two `u64::MAX - 1` entries produced `u64::MAX - 3`, not the crafted total),
  which confirms the wrap but is not the exact two-`u64::MAX` pair that wraps to
  the crafted `u64::MAX - 1`; that pair's arithmetic is two's-complement
  wrapping and its post-fix behavior is executed in both profiles.
- **M1 other consumers.** `Checksums::a16` also calls `findings`, but on the
  capture's own measured document; only the `verify` path was exercised. The
  fold is shared, so both now saturate.
- **M2 cancellation through `map_concurrently`.** The helper's cancellation arm
  is executed directly; the loop has no way to cancel a task, so the slot text
  for a cancelled metadata job was not driven end to end.
- **M2 operator output.** Which of the five callers writes the slot string into
  a document versus only into the summary was not traced to the last sink; all
  five wrap the `Err` with their own context, and the slot now carries the
  failing item index and cause.
- **Live inputs.** No live tag, manifest, metadata response or bundle was
  sampled; both findings are about locally accepted crafted values.
