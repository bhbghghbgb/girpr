# Sixth adversarial pass — review note

Tree: `E:\hoccode\hocrust\girpr`, branch `fix-gaps/gircap`, HEAD
`c8d4ddc` ("docs(review): fold the fifth pass into the review"), clean
worktree before this note.

Scope: `tools/capture/src/**`, `tests/**`, `docs/**`, `README.md`,
`Cargo.toml`. The fifth-pass fixes were re-read at their primary sites and
they hold; this pass found three new items, one behavioural and two
documentation errors.

## Verdict

| ID | Severity | Class | Location | Summary |
|---|---|---|---|---|
| J1 | Medium | bug | `src/verify/mod.rs:1069`, `:1075` | `list` treats a `MANIFEST.json` that cannot be *read* as "not a bundle", so a real bundle vanishes from the inventory with `result=ok` and exit `0`. |
| J2 | Low | doc error | `src/verify/mod.rs:1028` | The `list` doc promises bundles "deepest first"; the implementation sorts them by relative-path string, which is a different order for a mixed-depth tree. |
| J3 | Low | doc error | `src/run/mod.rs:711-732` | The `list` command doc still describes a refused/unopenable bundle as "a skip" and does not mention that an unreadable directory/entry now aborts the command, contradicting `verify::list` and `docs/00-spec.md:889-893`. |

J1 is the only behavioural finding. J2 and J3 are documentation-only, but
both state a contract a reader can rely on and get the opposite behaviour.

**Status (end of this pass):** all three validated and fixed — J1 in `0706984`,
`43d5977` and `3f042a1`, J2 in `28c3c18` plus `d74e40b`, J3 in `73d0cb5`. See
[Resolution](#resolution) for the evidence and [Resolution
verification](#resolution-verification) for the final numbers.

---

## J1 — an unreadable manifest is silently dropped from the `list` inventory

**Severity:** Medium. **Class:** bug (omitted error / bogus exit `0`).

### Quoted code

`src/verify/mod.rs:1069-1087`:

```rust
    while let Some(dir) = stack.pop() {
        if dir.join("MANIFEST.json").is_file() {
            match Bundle::open(&dir) {
                Ok(b) => found.push(listed_of(root, &b)),
                Err(e) => {
                    // A directory that is not a bundle at all is stepped over;
                    // one that is a bundle this build refuses is named.
                    if schema_version_of(&dir.join("MANIFEST.json")).is_ok() {
                        unreadable.push(Unreadable {
                            path: crate::emit::index::rel(root, &dir),
                            reason: e.clone(),
                        });
                    }
                    tracing::warn!(
                        dir = %dir.display(),
                        error = %e,
                        "a directory under --root looks like a bundle but does not open; skipped"
                    );
                }
            }
            // A bundle does not contain another bundle.
            continue;
        }
```

`schema_version_of` reads the manifest and flattens every failure to a
`String` (`src/verify/mod.rs:355-368`):

```rust
fn schema_version_of(path: &Path) -> Result<u32, String> {
    let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
    ...
}
```

`is_ok()` therefore means "the bytes were readable **and** the document
declared a version". Both are treated as the same fact.

### Concrete failure scenario (reproduced)

The repository's own reference bundle was hidden from `gircap list` by an
exclusive Windows file lock on its `MANIFEST.json` — the state a backup
tool, antivirus scanner or editor holds. No bundle bytes are changed.

```powershell
$m = 'E:\hoccode\hocrust\girpr\artifacts\hk4e_global\7.0.0__7.1.0\20261021T141336Z\MANIFEST.json'
$exe = 'E:\hoccode\hocrust\girpr\target\debug\gircap.exe'

# unlocked
& $exe list --root 'E:\hoccode\hocrust\girpr\artifacts'
# SUMMARY ... result=ok ... bundles=1 bundles_passing=1 bundles_unreadable=0 ... exit_code=0

# locked: FileShare.None, exactly what an indexer/AV can hold
$h = [System.IO.File]::Open($m, 'Open', 'Read', 'None')
try { & $exe list --root 'E:\hoccode\hocrust\girpr\artifacts' }
finally { $h.Dispose() }
```

Observed, locked:

```
WARN gircap::verify: a directory under --root looks like a bundle but does
not open; skipped ... error=...MANIFEST.json: The process cannot access the
file because it is being used by another process. (os error 32)
SUMMARY event=list tool=gircap version=0.1.0 result=ok root=...\artifacts
bundles=0 bundles_passing=0 bundles_unreadable=0 bundles_incomplete=0
patchmaps=0 blobs=0 first= last= exit_code=0
```

The same directory, unlocked, is `bundles=1` or (for a manifest that is
readable but incomplete) is named in `bundles_unreadable=1`. The lock alone
turns a known bundle into a confident empty inventory with exit `0`.

`fs::metadata` (used by `Path::is_file`) succeeds while the share lock is
held — `GetAttributes` returned `Archive` — so the first probe passes and
the failure happens at `schema_version_of`'s `fs::read`. A metadata error
other than `NotFound` (for example an entry that vanished between
`read_dir` and the stat) takes the other route: `is_file()` returns
`false`, the bundle is treated as "not a bundle at all", and the walk
records nothing at all, not even the warning.

### Why this fails the standard

- It is an omitted error: `if schema_version_of(...).is_ok()` discards a
  meaningful `std::io::Error`. The reviewer standard names this pattern
  (`if let Ok` discarding a failure) explicitly.
- It produces a bogus exit `0`: the inventory is short and the summary
  says `result=ok`, with `bundles_unreadable=0`, so no consumer can tell
  the bundle was skipped.
- It contradicts the contract written one screen above. `Index::unreadable`
  is "The directories that hold a `MANIFEST.json` this build will not
  read" (`src/verify/mod.rs:1010-1016`), and the walk's own doc says a walk
  that cannot see an entry is an error, not a shorter inventory
  (`src/verify/mod.rs:1043-1048`). `docs/00-spec.md:889-893` repeats it:
  "A walk that cannot **see** a directory ... is not a shorter listing:
  `list` stops with a usage error naming the directory or entry it could
  not read." A locked manifest is not seen, and `list` returns success.
- This is the same class as I4 (unreadable **directory** dropped), one
  level down: I4 fixed the directory read and left the manifest probe
  behind.

### Suggested fix

1. Make the manifest probe fallible and distinguish absence from failure:

   ```rust
   match std::fs::metadata(&manifest) {
       Ok(m) if m.is_file() => { /* try to open the bundle */ }
       Ok(_) => { /* a manifest path that is not a file: not a bundle */ }
       Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
           /* no manifest: not a bundle, step over */
       }
       Err(e) => return Err(format!("cannot read {}: {e}", manifest.display())),
   }
   ```

2. Split `schema_version_of`'s outcomes: a read (`std::io::Error`) is
   "cannot see the manifest" and must fail the walk; a JSON parse error or
   a missing `schema_version` remains "not a bundle at all" and keeps the
   documented step-over behaviour (`a_directory_that_is_not_a_bundle_is_not_named_by_list`).
   Returning a typed probe result, for example
   `enum VersionProbe { Version(u32), NotABundle(String) }` plus
   `Result<VersionProbe, String>` for the I/O error, keeps the two apart
   without changing `Bundle::open`'s public error text.

3. Add a test seam for the manifest read (the injected reader in
   `list_with`/`child_dirs` is the same pattern) and one test that
   `list_with` returns `Err` naming the manifest when its read fails.
   Keep the existing corrupt-JSON test as the negative case for the
   step-over arm.

Do not simply add the directory to `unreadable` on a read error: the spec
distinguishes "a bundle this build refuses" (named and counted) from
"a directory the walk could not see" (fail fast, exit `1`). A locked
manifest is the latter.

---

## J2 — `list`'s "deepest first" order is not the order it returns

**Severity:** Low. **Class:** doc error.

### Quoted code and doc

`src/verify/mod.rs:1028-1033`:

```rust
/// Every bundle under `root`, deepest first.
///
/// Recursive rather than a fixed depth, because the layout is
/// `<biz>/<old>__<new>/<capture_id>/` today and a root that points at
/// `artifacts/hk4e_global` and one that points at `artifacts/` should both work
/// without the caller knowing which.
```

`src/verify/mod.rs:1096`:

```rust
    found.sort_by(|a, b| a.path.cmp(&b.path));
```

The walk is a stack DFS, but the output is re-sorted by the relative-path
string. For bundles at different depths these orders differ: a bundle at
`a/old__new/id` sorts before one at `b/region/old__new/id` under the
implementation, while "deepest first" would put the second one first.
`tests/bundle_consistency.rs` also sorts the comparison scan with
`out.sort()`, so the implementation and the test agree with each other and
both disagree with the doc.

### Failure scenario

A consumer uses the documented order to pick the newest or most specific
bundle from a root that contains both a shallow pair directory and a nested
region layout. It reads `first` (or zip order) and gets the shallow bundle,
not the deepest one. No error is possible: the order contract is simply
false.

### Suggested fix

Say what the code does — "sorted by relative path" — or make the ordering
intentional (depth descending, then path) and add a mixed-depth test. The
spec does not promise an order, so correcting the doc is the smaller,
compatible change.

---

## J3 — the `list` command doc still describes the pre-I4 behaviour

**Severity:** Low. **Class:** doc error.

### Quoted doc

`src/run/mod.rs:711-732`:

```rust
/// `gircap list` — index every capture under a root.
///
/// The only command that takes a root rather than a bundle, because its
/// job is to find bundles. A directory that looks like one and does not open is
/// listed as a skip rather than aborting the walk: `list` exists to be run over a
/// tree that may contain a half-finished capture.
...
/// A directory that does not open stays stderr-only, as before: there is no
/// bundle to describe, so a `BUNDLE` record would have to carry the failure
/// instead of the facts, and a caller asking "which bundles are here" would get
/// an entry that is not one.
///
/// **The summary still counts them.** A bundle this build refuses — an older
/// schema — is a permanent condition with a known fix, and an operator whose
/// inventory quietly omits one has an inventory that is wrong. So the count is
/// on the summary and the path and the reason are on stderr.
```

The fifth pass changed the behaviour in two ways this text does not say:

- A refused bundle is not merely "listed as a skip": it is pushed to
  `Index::unreadable` and counted as `bundles_unreadable` in the summary
  (`src/run/mod.rs:759-771`).
- A walk that cannot read a directory or entry is now a hard failure:
  `verify::list` returns `Err`, and `run::list` maps it to
  `Failure::usage` (`src/run/mod.rs:733-734`), exit status `1` — exactly
  `docs/00-spec.md:889-893`. The doc still says the walk does not abort.

### Failure scenario

An operator reads the command doc, places `gircap list` in a script that
already tolerates the old "skip" behaviour, and gets exit `1` when a share
disappears; or reads "listed as a skip" and concludes that a
`bundles_unreadable` count is not part of the inventory. The command and
the written contract disagree, and the written one is the older one.

### Suggested fix

Rewrite the paragraph to name the three arms explicitly:

1. a directory with no readable bundle manifest that is *not* a bundle is
   stepped over;
2. a bundle this build refuses is named on stderr and counted in
   `bundles_unreadable`;
3. a directory/entry/manifest the walk cannot read stops the command with
   a usage error (exit `1`) naming what could not be read.

The record-shape rationale (no `BUNDLE` record for a non-bundle) can stay.

---

## Resolution

Validated at `ecc81c5` (this note's commit) and fixed on the same branch. Every
finding was confirmed against the current source and the real binary; none was
dismissed.

| ID | Verdict | Commit(s) | Resolution |
|---|---|---|---|
| J1 | valid, fixed | `0706984`, `43d5977`, `3f042a1` | `list_with` probes `MANIFEST.json` through the new fallible `probe_manifest`, which returns `Missing`, `NotABundle(reason)` or `Version(u32)`, plus `Err` naming the path and the I/O error when the stat or read fails. `Missing` and `NotABundle` keep the documented step-over; `Err` fails the walk, so `run::list` maps it to a usage error. `schema_version_of` is now the `Bundle::open` wrapper over the same probe and keeps its existing error texts for a refused schema. The manifest reader is injected in `list_with` like the directory reader, and `a_list_walk_that_cannot_read_a_manifest_fails_instead_of_dropping_the_bundle` fails against the pre-fix code (which silently returned `Ok` with no bundle). `3f042a1` adds the stat-arm probe test. `docs/00-spec.md:889-893` now names an unreadable manifest. `43d5977` keeps the old `is_file() == false` behaviour for a `MANIFEST.json` that is not a file (keep descending) so the fix changes only the failure arm. |
| J2 | valid, fixed (doc) | `28c3c18`, `d74e40b` | The doc now says both lists are sorted by relative path and that the order is **not** deepest-first; the spec promises no order and the summary's `first`/`last` read the sorted list, so changing the walk was the larger, unnecessary change. `list_orders_bundles_by_relative_path_not_by_depth` builds two real fixture bundles, nests one under `z_region/`, and asserts the shallower `hk4e_global/...` sorts first. |
| J3 | valid, fixed (doc) | `73d0cb5` | `run::list` now names the three arms: a non-bundle is stepped over, a bundle this build refuses is named on stderr and counted as `bundles_unreadable`, and anything the walk cannot read (directory, entry or manifest) stops the command with a usage error (exit `1`). The record-shape rationale for not emitting a `BUNDLE` record for a non-bundle stays. |

### J1 reproduction, before and after

Reference bundle `artifacts/hk4e_global/7.0.0__7.1.0/20261021T141336Z`, its
`MANIFEST.json` held by `[System.IO.File]::Open($m, 'Open', 'Read', 'None')`:

- before (`ecc81c5`): `WARN ... looks like a bundle but does not open; skipped`,
  `SUMMARY ... result=ok ... bundles=0 bundles_passing=0 bundles_unreadable=0
  ... exit_code=0`, process exit `0`;
- after (`43d5977`): `SUMMARY ... result=error exit_code=1 exit_code_slug=usage
  ... detail=cannot%20read%20...MANIFEST.json:%20The%20process%20cannot%20access
  %20the%20file...%20(os%20error%2032)`, process exit `1`;
- unlocked after the fix: `result=ok bundles=1 bundles_passing=1
  bundles_unreadable=0`, exit `0`.

The corrupt-JSON fixture still steps over (`a_directory_that_is_not_a_bundle_is_not_named_by_list`)
and the v1 fixture is still counted (`a_v1_bundle_is_named_by_list_rather_than_dropped`),
so the three arms are separated rather than collapsed into "fail on anything".

## Resolution verification

Run on this Windows host at `3f042a1` after all three fixes landed:

| Command | Result |
|---|---|
| `cargo build -p gircap` | `Finished dev profile ... in 0.46s`, exit `0`, no warnings |
| `cargo test -p gircap` | **671 passed, 0 failed, 0 ignored** (528 lib + 143 integration: 3 + 8 + 32 + 2 + 30 + 10 + 29 + 14 + 15; doc-tests 0) |
| `cargo clippy -p gircap --all-targets` | `Finished dev profile ... in 7.02s`, exit `0`, no diagnostics |
| `cargo fmt -p gircap --check` | exit `0`, clean |

Three tests are new over the fifth-pass 668: the J1 injected-reader regression
and the J1 stat-arm probe (lib 526 → 528), and the J2 mixed-depth ordering test
(`offline_bundle` 13 → 14). No test is `#[cfg(unix)]`; the two `#[cfg(windows)]`
tests ran, so nothing was skipped on this host.

## Fifth-pass fixes re-checked

The following were re-read at the current tree and still hold; they are not
findings:

- forced-path budget and ordinals: one cumulative `Budget`, charged at
  `src/select/mod.rs:544-573`, with `forced_ordinals: BTreeMap<Class, usize>`;
  phase 2 reuses the same ledger (`:699-702`).
- duplicate `capture_id` refusal before the first write:
  `src/emit/fixture.rs:226-244`.
- `drain_capped` returning `(ApiError, u64)` and pricing a failed `2xx` body
  at the bytes that arrived: `src/http/fetcher.rs:142-171`, `:851-887`.
- duplicate patchmap-id refusal and entry-error propagation:
  `src/verify/mod.rs:237-277`.
- tag validation before the dump writes: `src/manifest/dump.rs:84-99`.

## Verification before this pass (at `ecc81c5`)

Run on this Windows host at the tree above, after the fifth-pass commits and
before any sixth-pass fix (the note itself changed no code):

| Command | Result |
|---|---|
| `cargo build -p gircap` | `Finished dev profile ... in 0.52s`, exit `0`, no warnings |
| `cargo test -p gircap` | **668 passed, 0 failed, 0 ignored** (526 lib + 142 integration: 3 + 8 + 32 + 2 + 30 + 10 + 29 + 13 + 15; doc-tests 0) |
| `cargo clippy -p gircap --all-targets` | `Finished dev profile ... in 30.22s`, exit `0`, no warnings |
| `cargo fmt -p gircap --check` | exit `0`, clean |

The count matches the fifth-pass claim. No test is `#[cfg(unix)]`; the two
`#[cfg(windows)]` tests (`src/cli/mod.rs:462`,
`tests/cli_contract.rs:173`) ran and passed, so nothing was skipped on this
host.

J1 was additionally reproduced end to end with the reference bundle:

- unlocked: `bundles=1 bundles_passing=1 bundles_unreadable=0`, exit `0`;
- with `[System.IO.File]::Open(..., 'Read', 'None')` on its
  `MANIFEST.json`: `bundles=0 bundles_unreadable=0`, `result=ok`, exit `0`,
  one `WARN ... skipped` on stderr.

The stub-directory contrast showed `bundles_unreadable=1` once the lock was
released and `0` while it was held, with no change to any file's bytes.

## Not verified

- J1: no real Windows ACL or POSIX permission denial was set up; the repro
  uses an exclusive share lock (os error 32). The source path is the same
  for any `fs::read` error, but a metadata failure that takes the
  `is_file() == false` route was reasoned from the code rather than
  induced.
- J2: no mixed-depth tree was captured and listed; the order claim is
  verified by reading `found.sort_by(...)` against the doc, not by a live
  mixed-depth inventory.
- J3: no live `list` run over an unreadable **directory** was repeated this
  pass; the I4 unit tests for `child_dirs`/`list_with` passed as part of
  the suite.
- No live network capture or live API corpus was exercised; this pass
  stayed within the offline/source/test surfaces.
