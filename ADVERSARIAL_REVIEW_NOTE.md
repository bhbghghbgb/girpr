# Seventh adversarial pass

Tree `fix-gaps/girpr` @ `b842c6c`, clean. Scope: `src/**`, `tests/**`, `docs/**`,
root `README.md`, root `Cargo.toml`. The sixth-pass fixes hold (deferred audio
write, BOM/unrecognized refusal, metadata gate, `--plan-out` help). Three
residual findings; one Medium.

| ID | Severity | Class | Location | Summary |
|---|---|---|---|---|
| S-1 | Medium | scope / destructive | `src/repair/purge.rs:121-124`, `:164-168`, `:197-211` | The emptied-dir sweep removes an empty directory that the active scope hides, although `--include`/`--exclude` promises it is not deleted. |
| S-2 | Low | arithmetic / panic | `src/repair/file/inplace.rs:213-217` | The staged-tier payload is a non-saturating `.sum()` over server sizes; a corrupt manifest panics in debug and wraps in release, against binding decision 1. |
| S-3 | Low | swallowed error | `src/repair/phases/post.rs:160-166` | `is_file()` turns any stat error into "not a file" and skips a deprecated delete with no log; the comment claims the stat succeeded. |

## S-1 — an empty out-of-scope directory is swept

**Evidence.** `plan_purge` pushes every directory into `all_dirs` before it
computes a rel or consults the scope (`src/repair/purge.rs:121-124`). The scope
is consulted only for files (`:129-131`, classifier at `:343-346`). The sweep
set is `all_dirs` minus `kept_dirs` (`:164-168`), and `kept_dirs` is populated
only by `mark_kept_ancestors`, which walks up from **kept files**
(`:132`, `:197-211`). A directory with no regular file under it therefore has
no marker and is scheduled for `remove_dir` (`apply_purge` at `:247`,
`remove_emptied_dirs` at `:258-271`). `hidden` counts files only (`:157-159`),
so no scope warning is emitted either (`:174-178`).

**Contract.** `README.md:162-164`: a path outside `--include`/`--exclude`
"**does not exist** — it is dropped from the plan *and* kept by the
files-cleanup, so `--purge-*` cannot eat it"; `README.md:284`: non-matching
paths are "**not deleted**". `docs/04-scope-limitation-spec.md:96-97` lists
`remove_emptied_dirs` as a site that must honour the scope ("a directory whose
only contents were filtered away must not be swept"). A directory with no
contents was not "filtered away", but it is still a path the scope hides, and
the blanket README claim has no file-only qualifier.

**Scenario.** Game dir contains an empty `mods/` placeholder plus the planned
file. `girpr --purge-only --include "game/foo.dat"` (same for `--purge-after`
with that include): `all_dirs = [mods]`, no kept file lives under it,
`dirs = [mods]`, and `remove_dir("mods")` succeeds. `deleted_extra_bytes`
stays `0` and the log carries no warning, so the run reports a clean cleanup
while having deleted an out-of-scope directory. The same happens on a scoped
`--purge-before`/`--purge-after` for any zero-file directory (a user-created
placeholder, a previously emptied screenshot tree).

**Why it fails.** The scope is implemented as a keep-set for *files*; the
directory sweep was not brought into it. `mark_kept_ancestors` protects a
directory only when surviving content is below it, so "nothing to keep" is
read as "safe to delete" instead of "not mine". `docs/04` line 110 warns about
exactly this hazard ("an emptied directory has no files left to classify"), and
the chosen ancestor-marking fix leaves the zero-file case outside the keep set.

**Suggested fix.** Record each directory's rel in the walk and keep it when the
active scope hides it — minimally `scope.has_includes() &&
!scope.is_visible(dir_rel)`, so `--exclude` file semantics are untouched — and
count it with `hidden`. Add a regression case beside
`a_purge_under_a_scope_leaves_every_other_fixture_file_alone`
(`tests/repair_offline.rs:1631`), which only covers a *populated* out-of-scope
`log/`.

## S-2 — non-saturating payload sum in the in-place tier choice

**Evidence.** `src/repair/file/inplace.rs:213-217`:

```rust
let payload: u64 = bad
    .iter()
    .filter(|&&i| sources[i].is_some() && scan.source_ok[i])
    .map(|&i| file.chunks[i].uncompressed_size.max(0) as u64)
    .sum();
```

`uncompressed_size` is a server `i64` (`src/sophon.rs:47`), clamped to `>= 0`
but not bounded. Every sibling estimate saturates —
`src/repair/phases/plan.rs:175-180` ("three chunks at `i64::MAX`"),
`src/repair/scan.rs:126-131`, `src/repair/scope/report.rs:102-105` — because
binding decision 1 is "Saturate, never wrap, in every server-size estimate"
(`ADVERSARIAL_REVIEW.md:841`). This is the only `.sum()` over those sizes.

**Scenario.** One planned file whose bad chunks have `uncompressed_size`
`i64::MAX, i64::MAX, 2`. The sum is `2^64`, which wraps to `0` in release:
`payload == 0` selects `(Tier::Staged, None)` (`inplace.rs:221`), i.e. the "stage
nothing at all" branch, with no RAM reservation at all — the budget was
bypassed, not answered. In a debug build the same input panics on the overflow
at the `.sum()`; the panic guard turns that into a failed file / exit `3`,
which is a repair refusal invented by local arithmetic, not by the manifest's
readability. The layout warning at `plan.rs:266-273` does not stop the file
from being repaired.

**Why it fails.** The value is used to decide a memory reservation; wrapping
changes that decision's meaning (including to the "zero payload" sentinel),
and the panic path is reachable from server data, which this tree treats as
untrusted everywhere else.

**Suggested fix.** Replace the `.sum()` with a saturating fold, and add a unit
test mirroring `an_overflowing_download_estimate_saturates`
(`src/repair/phases/plan.rs:557`). Saturation makes the sum fail closed: a
huge payload fails the reservation and picks the ordered tier.

## S-3 — a stat failure in the deprecated delete is silent and mislabelled

**Evidence.** `src/repair/phases/post.rs:160-166`:

```rust
if !p.is_file() {
    continue;
}
// A stat failure is not "0 bytes". `p.is_file()` already succeeded, so
// this is a race or an ACL, not a missing file: ...
```

`Path::is_file` is `metadata().map(|m| m.is_file()).unwrap_or(false)`, so
`PermissionDenied`, a sharing violation, or any other stat error takes the same
silent `continue` as a directory or an absent path. The warning at `:167-177`
is unreachable for exactly the failure it describes, and the comment at
`:163-164` asserts the opposite of what `is_file()` did. Nothing is counted and
nothing is logged, so the deprecated file is left in place and the run still
exits `0`.

**Scenario.** The server's deprecated list names `old/locked.dll`; `old/`
denies list/traverse (or the file is locked by a scanner). `delete_deprecated`
skips it with no `tracing` event; `SUMMARY` and `deleted_extra_bytes` are the
same as if the file had not been listed. Binding decision 9 says "a stat
failure is never a zero" and "the post-phase delete warns and omits the size"
(`ADVERSARIAL_REVIEW.md:849`); the skip path never reaches that warn.

**Why it fails.** `is_file()` collapses three states (file, not-a-file, stat
error) and the code documents the collapsed state as if it were the first.

**Suggested fix.** Call `std::fs::metadata(&p)` once and match:
`Err(e) if e.kind() == NotFound` → continue (already gone), `Err(e)` → warn and
continue (or count it), `Ok(m) if !m.is_file()` → continue, `Ok(m)` → delete
and use `m.len()` for the counter. This deletes the second stat and the false
comment, and keeps the existing "size unavailable" honesty.

## Verification

Run on this tree, Windows, debug profile (no source changes were made for this
pass):

| Command | Result |
|---|---|
| `cargo build` | ok (`Finished dev profile`) |
| `cargo test -p girpr` | **327 passed, 0 failed, 7 ignored** (189 lib, 7 `artifact_server`, 17 `cli_contract`, 25 `repair_machinery`, 89 `repair_offline`; ignored: 6 `artifact_bridge` + 1 `peak_heap`; 0 `main`, 0 doc-tests) |
| `cargo clippy -p girpr --all-targets` | clean, no warnings |
| `cargo fmt --check` | exit `0` |

## Not verified

- The three findings are code-reading claims; no reproducer was committed and
  no source, test, or doc change was made in this pass, so none was executed
  end-to-end. S-1 needs a filesystem probe (empty dir plus a scoped purge);
  S-2 needs a corrupt-manifest unit test; S-3 needs an injected stat failure
  (no seam exists).
- The `#[cfg(unix)]` test blocks (containment/mode) do not run on this Windows
  host.
- The 7 ignored tests (`artifact_bridge`, `peak_heap`) were not run.
- No release-profile run, so S-2's release wrap is stated from Rust semantics,
  not observed.
- No Linux/macOS host and no live CDN traffic; all tests use the in-process
  mock.
