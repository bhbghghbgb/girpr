# Ninth adversarial pass — `girpr`

- Branch: `fix-gaps/girpr` (reviewed in place; no worktree, no branch change)
- Review HEAD: `8003226` (`docs(review): fold the eighth pass into the review`)
- Scope: `src/**`, `tests/**`, `docs/**`, root `README.md`, root `Cargo.toml`
- Result: **3 actionable findings** (2 bugs, 1 doc error). No source, test, or doc file
  was changed; this note is the only file added by this pass.

## Verdict

| ID | Severity | Class | Location | Summary |
|---|---|---|---|---|
| U-1 | Medium | bug | `src/repair/phases/post.rs:143`, `:280`, `:328`, `:341`; `src/repair/phases/metadata.rs:199` | The post-phase sinks and the `--audio` pre-flight test the **raw** server/config spelling against the scope, while the filesystem sink cleans it first; a `./`-spelled path therefore passes a gate that must hide it (and is refused when it is legitimately in scope). |
| U-2 | Medium | bug | `src/repair/lock.rs:70-75` | A stale lock that cannot be removed makes `Lock::acquire` recurse until the process aborts: `remove_file(...).ok()` swallows the reason and the retry has no bound or state change. |
| U-3 | Low | doc error | `README.md:51-60`, `:385`; `docs/04-scope-limitation-spec.md:738-739` | The current module map and the touch-point table still name the pre-split paths (`src/repair/{metadata,manifest,check,post}.rs`, `src/repair/plan/...`, `src/repair/file.rs`). |

---

## U-1 — the scope is matched on the raw rel spelling at the delete/move/write sinks

### Quoted code

```rust
// src/repair/phases/post.rs:142-147
    for n in list {
        if !scope.is_visible(n) {
            tracing::debug!("deprecated path out of scope, kept: {n}");
            continue;
        }
        let p = match util::contained_join(game_dir, n) {
```

```rust
// src/repair/phases/post.rs:195
            match std::fs::remove_file(&p) {
```

```rust
// src/repair/phases/post.rs:277-283
    // Rel paths are `/`-separated everywhere except at the filesystem boundary.
    let cache_rel = cache_dir.replace('\\', "/");
    let res_rel = res_dir.replace('\\', "/");
    if !scope.is_visible(&cache_rel) || !scope.is_visible(&res_rel) {
        tracing::debug!("audio cache->res out of scope, skipped");
        return;
    }
```

```rust
// src/repair/phases/post.rs:341
        if !scope.is_visible(&join_rel(&cache_rel, rel)) {
```

```rust
// src/repair/phases/metadata.rs:198-199
        let scan_rel = config.audio_pkg_scan_dir.replace('\\', "/");
        if ctx.path_scope.is_active() && !ctx.path_scope.is_visible(&scan_rel) {
```

The correct sibling is the plan sink, which scopes the **cleaned** form:

```rust
// src/repair/phases/plan.rs:222-229
            let rel = util::contained_rel(&f.file)
                .with_context(|| format!("manifest path {:?}", f.file))?;
            let rel = rel.to_string_lossy().replace('\\', "/");
            // ...
            if blacklist.contains(&rel) || !scope.is_visible(&rel) || !seen.insert(dedup_key(&rel))
```

`PathScope::is_visible` matches the string it is handed, verbatim
(`src/repair/scope/filter.rs:79-85`). `util::contained_rel` — called by
`contained_join` at `src/util.rs:410` — deliberately rebuilds the path without
`.` components (`src/util.rs:343-378`), so `./old/legacy.dll` and
`old/legacy.dll` are the same on-disk file and the same rel string everywhere
except at these three gates.

### Failure scenario

Probe (external crate outside the repo, path dependency on this checkout; see
Verification):

```
is_visible("old/legacy.dll")    = false
contained_rel("old/legacy.dll") = "old\\legacy.dll"
is_visible("./old/legacy.dll")  = true
contained_rel("./old/legacy.dll") = "old\\legacy.dll"
```

with `excludes = ["old/*"]`. The two spellings name one file; only the plain
one is hidden.

- **Delete (fail-open).** `girpr --exclude "AudioAssets/*"` against a
  `deprecated_file_configs[].name` of `./AudioAssets/voice.bank`: `:143` admits
  it, `:147` resolves it to `<game>/AudioAssets/voice.bank`, `:195` deletes it.
  A path the user excluded from the run is removed.
- **Audio move (fail-open).** The same spelling in `audio_pkg_cache_dir`: the
  subtree passes `:280`, every file passes `:341`, and an excluded cache is
  moved into `res`.
- **Audio scan file (fail-open write).** `--exclude "AudioAssets/Scan/*"` with
  `audio_pkg_scan_dir = "./AudioAssets/Scan/scan.bin"` does **not** trip
  `:199`; the scan file is then written by
  `src/repair/mod.rs:384` → `audio::write_scan_file`, which has no scope gate of
  its own.
- **Spurious refusal (fail-closed).** `--audio --include "AudioAssets/Scan/*"`
  with the same `./`-spelled scan dir is refused at `:200-205` with exit `1`
  and advice to "include it" — which the user already did.

Both directions contradict the documented contract: filtered-out paths are
"treated as nonexistent on both sides" (`filter.rs:3-4`) and the matched string
is "the same string `plan::build_plan` stores; `util::contained_rel` validates
it at the filesystem boundary" (`filter.rs:10-12`, `docs/04-scope-limitation-spec.md:191-192`).

### Why it fails

The gate and the sink disagree about what the path is. Containment is checked on
the cleaned form; scope is checked on the raw form. Any `.` component — a
leading `./`, or `a/./b` — splits them. This is exactly the failure mode
`contained_rel` documents at `src/util.rs:343-349` for `expected_paths()`, fixed
there but not at these sinks.

### Test gap

The existing tests use plain spellings only, so none of them can catch this:
`deprecated_files_outside_the_scope_are_not_deleted`
(`tests/repair_offline.rs:1834`) and
`explicit_audio_under_a_scope_is_refused_unless_the_scan_file_is_in_scope`
(`tests/repair_offline.rs:1779`).

### Suggested fix

Clean once and scope the cleaned form. Add a lexical
`util::clean_rel(&str) -> String` (replace `\`, drop `CurDir` components, the
same rules `contained_rel` already applies, without the filesystem step) and use
it at the three gates:

- `delete_deprecated`: `let rel = util::clean_rel(n);` then
  `if !scope.is_visible(&rel)`, keeping `contained_join` for the boundary —
  or gate on the rel that `contained_rel` returns, since it is already the
  canonical form.
- `move_audio_cache`: `let cache_rel = util::clean_rel(cache_dir);` and the same
  for `res_rel`.
- the `--audio` pre-flight: `let scan_rel = util::clean_rel(&config.audio_pkg_scan_dir);`.

Add the missing regression cases: exclude `old/*` while serving
`./old/legacy.dll` and assert the file survives with the "out of scope" debug
line; and an in-scope `./`-spelled scan dir accepted under an
`AudioAssets/Scan/*` include.

---

## U-2 — an unremovable stale lock recurses until the stack overflows

### Quoted code

```rust
// src/repair/lock.rs:70-75
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if is_stale(&path) {
                    // Its run is gone: take it over.
                    std::fs::remove_file(&path).ok();
                    return Lock::acquire(game_dir);
                }
```

### Failure scenario

`girpr.lock` is older than `STALE_AFTER` and cannot be deleted — on Windows, a
process that opened it without `FILE_SHARE_DELETE` (an AV scanner, an editor, a
crashed tool's leaked handle), a read-only attribute, or a delete-denying ACL.
`create_new` returns `AlreadyExists`; `is_stale` returns `true`; `remove_file`
fails; `.ok()` discards the error; `Lock::acquire` calls itself. Nothing about
the state changes, so every frame repeats.

Reproduced (see Verification):

```
thread 'main' (3752) has overflowed its stack
EXIT=-1073741571
```

The run aborts with no `SUMMARY` line, no exit `1`, and no message naming the
lock. This also contradicts the module's own contract — "A lock that outlives
its run ... is taken over after `STALE_AFTER`" (`lock.rs:22-23`) — and puts the
process outside the documented exit-code table.

On Unix an open file does not block `unlink`, so the same recursion needs a
delete-denied directory or file, but the code path is portable; only the
easiest trigger is Windows-specific.

### Why it fails

An error that is supposed to end the run cleanly (exit `1`) is discarded, and
the retry is unbounded with no state change. The failure mode is worse than the
refusal it replaces: a lock the user could have been told about aborts the
process.

### Test gap

`a_stale_lock_is_taken_over` (`lock.rs:158-175`) covers only the case where the
removal succeeds. Nothing covers a stale lock whose removal fails.

### Suggested fix

Do not swallow the error, and bound the retry:

```rust
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if !is_stale(&path) {
                    return Err(anyhow::anyhow!(
                        "another girpr run holds {} (pid {}); refusing to run",
                        path.display(),
                        holder_pid(&path)
                    ));
                }
                std::fs::remove_file(&path).with_context(|| {
                    format!("take over the stale lock {} (it cannot be removed)", path.display())
                })?;
                return Lock::acquire(game_dir);
            }
```

The recursive call is then bounded in practice: after a successful removal the
next `create_new` either succeeds or finds a *fresh* lock (a 24 h age cannot
recur), which takes the refusal branch. A three-attempt loop returning the last
contextual error is equivalent and makes the bound explicit. Add a test that
ages a lock, makes it undeletable (read-only is enough on Windows), and asserts
`Err` with the path in the message.

---

## U-3 — README/docs still point at the pre-split module paths

### Quoted text

```markdown
<!-- README.md:51-60 -->
| `src/repair/metadata.rs` | local state + server metadata (docs/02 1–2) | `fetch`, `Meta` |
| `src/repair/manifest.rs` | manifest fetch + local chunk-reuse map (3) | `fetch_latest`, `local_reuse_map`, `LocalChunkMap` |
| `src/repair/plan/mod.rs` | work list (4) + the plan module root | `build_plan`, ... |
| `src/repair/plan/purge.rs` | Collapse files-cleanup: plan (5, 7) + apply | `plan_purge`, ... |
| `src/repair/check.rs` | `--check-only` verification (6.5) | `verify_plan` |
| `src/repair/file.rs` | per-file repair: in-place patch, the temp mode, reuse vs download (6) | `repair_all`, ... |
| `src/repair/post.rs` | deprecated/audio/`config.ini` + cleanup-after (7) | `run` |
```

```markdown
<!-- README.md:385 -->
  `sophon_base_override` (`src/config.rs`; resolved in `src/repair/metadata.rs`;
```

```markdown
<!-- docs/04-scope-limitation-spec.md:738-739 -->
| `src/repair/post.rs` | scope gate on deprecated deletes and audio move |
| `src/repair/file.rs` | write-bytes ledger + download-bytes hard stop + fail-fast |
```

### Why it fails

The tree has no `src/repair/{metadata,manifest,check,post}.rs`, no
`src/repair/plan/`, and no `src/repair/file.rs`. The same symbols live at:
`phases/plan.rs:203` (`build_plan`), `purge.rs:82` (`plan_purge`),
`phases/check.rs:27` (`verify_plan`), `file/mod.rs:167` (`repair_all`),
`phases/metadata.rs:23,60` (`Meta`, `fetch`), `phases/manifest.rs:30,52`
(`fetch_latest`, `local_reuse_map`). README is the reader's module map; every
path in rows 51-60 is a dead end, and `:385` sends a reader looking for the
metadata resolution in a file that does not exist.

This matters more than a typo: the rows are also the index a contributor uses
before editing the delete/move sinks (U-1) or the ledger, and `docs/04:738-739`
is the touch-point table for exactly those changes.

### Suggested fix

Update the eight README rows and `README.md:385` to `phases/...`, `purge.rs`,
and `file/mod.rs`, and the two `docs/04` rows to `phases/post.rs` and
`file/mod.rs`.

`docs/03-repair-rust-implementation-plan.md:32-42` lists the same pre-split
layout. It is an implementation *plan*, so whether it should track the as-built
tree is a policy call, not part of this finding.

---

## Verification

All commands run in `E:\hoccode\hocrust\girpr` on `fix-gaps/girpr`, HEAD
`8003226`, working tree clean at the start (`git status --porcelain=v1 -b` →
`## fix-gaps/girpr`).

| Command | Result |
|---|---|
| `cargo build` | `Finished \`dev\` profile [unoptimized + debuginfo] target(s) in 1.80s` |
| `cargo test -p girpr` | **338 passed, 0 failed, 7 ignored** |
| `cargo clippy -p girpr --all-targets` | `Finished` — no warnings (re-check forced by touching `src/lib.rs`; no content change) |
| `cargo fmt --check` | exit `0` |

Per-binary test counts: lib 199 passed; `artifact_bridge` 0 run / 6 ignored;
`artifact_server` 7; `cli_contract` 17; `peak_heap` 0 run / 1 ignored;
`repair_machinery` 25; `repair_offline` 90; binary `girpr` 0; doc-tests 0.

The 7 ignored tests are the expensive real-bundle and heap suites documented at
`tests/artifact_bridge.rs:27-53` and `tests/peak_heap.rs:19-24`. No test is
skipped for being Unix-only: `rg "cfg\(unix\)" tests` finds no match, so nothing
in the suite was unrunnable on this Windows host.

### Probes (outside the repo; the workspace is untouched)

`E:\Symlinks\Temp\girpr-scope-probe` is a scratch crate with a path dependency
on this checkout. It is not added to the repo or its workspace.

```
$ ...\girpr-scope-probe.exe scope
is_visible("old/legacy.dll") = false
contained_rel("old/legacy.dll") = "old\\legacy.dll"
is_visible("./old/legacy.dll") = true
contained_rel("./old/legacy.dll") = "old\\legacy.dll"
EXIT=0

$ ...\girpr-scope-probe.exe lockshare
holding a stale lock open without FILE_SHARE_DELETE

thread 'main' (3752) has overflowed its stack
EXIT=-1073741571
```

The first probe compiles `PathScope::new(&[], &["old/*"])` and calls the public
`is_visible` / `contained_rel`; the second ages a `girpr.lock`, holds it open
without `FILE_SHARE_DELETE`, and calls `Lock::acquire`.

## Checked and found sound this pass

- Eighth-pass F-7 wording is consistent: the write-bytes description agrees
  across `src/cli.rs:89-95`, `docs/04:311-316` and `ledger.rs:387-400` — it is
  the disk cost, the in-place target and `{path}_tmp` both count, and it is not
  network cost.
- The `--audio`/scope refusal runs in the metadata phase before execution
  (`metadata.rs:186-207`), and its message names the scan-file escape hatch,
  which the code accepts for plain spellings (the `./` spelling is U-1).
- `stat_deprecated`'s four-way split (`post.rs:160-188`) keeps a failed stat
  distinct from "not a file" and from "already gone"; the delete still runs and
  the counter is not under-reported.
- `contained_rel` still cleans and refuses exactly what it documents
  (`util.rs:326-378`), including the `a/./b` case, and its tests pin it
  (`util.rs:883-1037`).
- Audio scan-file parsing: BOM stripping and the absent-vs-unreadable split are
  shared with `read_config_list` and pinned by
  `config_lists_strip_a_bom_and_warn_per_unparseable_line` (`util.rs:1322`) and
  `an_all_malformed_config_list_is_refused` (`util.rs:1366`).
- `resolve_mode`'s `Err(_) => Mode::Inplace` (`file/mod.rs:560`) is not a
  fail-open: an unopenable target answers `Inplace::Unavailable` and the caller
  falls back to the temp path with a warning (`file/mod.rs:480`, `:541-544`).
- Fresh and removable-stale lock behaviour is correct and tested
  (`lock.rs:141-175`); only the unremovable path is U-2.

## Not verified

- The live HoYoPlay API's actual spelling of `deprecated_file_configs[].name`
  and `audio_pkg_{cache,res,scan}_dir`. U-1 needs a `.` component there; no live
  response was inspected, and the probe exercises the functions directly rather
  than a full run against a mock server.
- U-1 end to end. The path from `:143`/`:280`/`:199` to the delete/move/write is
  straight-line and was traced by reading, but no CLI run with a `./`-spelled
  server value was executed.
- U-2 on Unix. The probe is Windows-only; there the trigger would be a
  delete-denied directory or file rather than an open handle.
- The 7 ignored suites (`artifact_bridge`, `peak_heap`) were not run; they need
  the real bundle and are excluded by design.
- `read_blacklist` only replaces `\` and does not drop `.` components
  (`util.rs:530-535`), while the blacklist is compared against cleaned rels
  (`phases/plan.rs:229`). Possibly a second normalization gap on the same theme,
  but reachability depends on the launcher's actual blacklist strings; not
  confirmed, so not counted as a finding.
- Already-recorded open items in `ADVERSARIAL_REVIEW.md` 8.10 (case-sensitive
  `expected_paths`/`classify_purge_path`, injection seams, S-1, dry-run skipping
  `validate_chunk_ranges`, N-19, R-2, T-1, T-2) were not re-litigated and are
  not repeated here.
