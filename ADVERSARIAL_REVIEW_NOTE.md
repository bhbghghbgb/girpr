# ADVERSARIAL REVIEW NOTE — tenth pass

**Reviewed tree:** `fix-gaps/girpr`, HEAD `7eb6268` (`docs(review): drop the
vestigial section numbering`), clean worktree.
**Scope:** `src/**`, `tests/**`, `docs/**`, root `README.md`, root `Cargo.toml`;
the ninth-pass fixes U-1…U-3 and the 14 decisions that bind in
[`ADVERSARIAL_REVIEW.md`](ADVERSARIAL_REVIEW.md). `tools/sync`, `tools/capture`,
`crates/log` and `crates/tempdir` were ignored except where `girpr` depends on
them.
**Method:** source reading from the wire/config boundary through the plan, the
filesystem sinks and the exit code, including the failure and retry paths;
parity checked against Starward `GamePackageService.cs` at the commit the docs
cite. Nothing outside this file was changed.

## Verdict

| ID | Severity | Class | Site | Summary |
|---|---|---|---|---|
| V-1 | high | purge keep-set / documented claim false | `src/repair/phases/plan.rs:128-130,228`, `src/repair/purge.rs:88,146,352-369` | A correctly spelled blacklist entry drops the file from the plan and therefore from `expected_paths()`, so `--purge-before`/`--purge-after`/`--purge-only` **deletes** a file the docs say is never deleted. |
| V-2 | medium | config boundary / fail-open | `src/util.rs:539-549` (`:546`), `src/repair/phases/plan.rs:222-228` | Blacklist entries are not normalised while the plan compares them against the cleaned rel, so a `./`- or `//`-spelled entry silently stops protecting the file (decision 14). |
| V-3 | low | server boundary / fail-safe cost | `src/sophon.rs:661-683` (`:670`), consumers `temp.rs:119`, `inplace.rs:77`, `scan.rs:106,285`, `plan.rs:153` | The local reuse map is keyed on the raw manifest spelling while every consumer looks up the cleaned plan rel, so a non-canonical manifest loses all reuse and can trip `--max-download-bytes` (exit `5`). |

V-1 and V-2 are two sides of one asymmetry: the manifest path is cleaned at its
boundary, the blacklist spelling is not, and the purge keep set is derived from
the *filtered* plan. Neither is in
[`OPEN_FOR_MAINTAINERS.md`](OPEN_FOR_MAINTAINERS.md): item 15 covers the
`blacklist_dir` **directory field**, not the entries inside the file, and no item
covers the purge keep set or the reuse-map key. V-3 is likewise distinct from
item 1 (case sensitivity of `expected_paths()`/`classify_purge_path`).

---

## V-1 — a blacklisted file is purged under `--purge-*`

**Evidence.**

- `build_plan` cleans the manifest path, then tests the blacklist and `continue`s
  before the file reaches `files` (`src/repair/phases/plan.rs:222-231`, filter at
  `:228`).
- `expected_paths()` is exactly `self.files.iter().map(|f| f.rel.clone())`
  (`src/repair/phases/plan.rs:128-130`).
- `plan_purge` passes that set as the manifest keep set
  (`src/repair/purge.rs:88`); `classify_purge_path` keeps only a path in
  `expected`, in `keep_set()` (`config.ini` + the lock), in the audio-pattern
  rules, or outside the scope (`src/repair/purge.rs:352-369`). `keep_set()` does
  not contain manifest files (`src/repair/purge.rs:317-325`).
- The post-repair pass re-plans from the same `plan`
  (`src/repair/phases/post.rs:59-60`), so both purge timings are affected.

**Failure path.** `config.ini` has `enable_resource_blacklist=true` and
`blacklist_dir=blacklist.txt`; the file contains `{"fileName":"game/foo.dat"}`;
the manifest lists `game/foo.dat`; the run passes `--purge-after` (or
`--purge-before`, or `--purge-only`). The blacklist test at `plan.rs:228` drops
`game/foo.dat` from `files`, `expected_paths()` no longer contains it, the walk
sees it on disk and classifies it `Purge`, and `apply_purge` deletes it. The file
was never repaired (correct for a blacklist) and then removed (wrong): the user's
"exclude from the work list" became "delete".

This is a documented-claim contradiction, not a trade-off:
`docs/01-repair-findings-and-verdict.md:101-103` says `res_category_dir` and
`blacklist_dir` "only *exclude* entries from the download work list … they are
never deleted", and `docs/02-repair-chunk-spec.md:342-345` defines the cleanup's
expected set as "`{latest manifest paths}` ∪ `{config.ini}`". The narrowing
argument in `plan.rs:199-202` and `docs/04-scope-limitation-spec.md:86-93` is
about the **scope**, not the blacklist; the scope's narrowing is already
neutralised by `classify_purge_path` rule 0, so the blacklist is the only filter
whose narrowing changes the delete decision.

**Suggested fix.** Make the blacklist a purge keep input, so the documented
"never deleted" holds even for an entry that has since left the manifest.
`plan_purge` already takes the manifest set, `keep_set()` and the scope
(`src/repair/purge.rs:82-87`); add the cleaned blacklist as one more input and
check it beside them in `classify_purge_path` (a named parameter, not folded
into `server_keep`, so the doc comment can keep the provenance distinct):

```rust
pub fn plan_purge(game_dir: &Path, plan: &RepairPlan, server_keep: &HashSet<String>,
                  blacklist: &HashSet<String>, scope: &PathScope) -> PurgePlan
// classify: scope → manifest → keep_set → blacklist → audio patterns
```

and thread `meta.blacklist` through the two call sites
(`src/repair/mod.rs:177`, `src/repair/phases/post.rs:60`).

The alternative is to keep the plan's expected set unfiltered by the blacklist:
insert every scope-visible cleaned rel into a new `RepairPlan` field **before**
the blacklist test and have `expected_paths()` return that set. It matches
docs/02's "latest manifest paths" literally, but it protects a blacklisted path
only while the manifest still lists it, so it is the weaker of the two.

**Test.** The mock server hardcodes `blacklist_dir: ""` and
`enable_resource_blacklist: false` (`tests/common/mod.rs:736-737`; the same in
`tests/artifact_server.rs:124-125`) and `MockOpts` has no fields for them, so the
blacklist has never been exercised end-to-end. The test needs two `MockOpts`
fields threaded into that JSON, a `game/blacklist.txt` containing
`{"fileName":"game/foo.dat"}`, and a `--purge-after` run asserting
`game/foo.dat` still exists (the blacklist file itself is a legitimate purge
candidate). It fails on the current tree.

---

## V-2 — blacklist entries are not normalised

**Evidence.**

- `read_blacklist` only replaces `\` with `/` (`src/util.rs:539-549`, `:546`);
  it never calls `contained_rel`/`clean_rel`.
- `build_plan` cleans the manifest path (`src/repair/phases/plan.rs:222-223`) and
  compares the cleaned string: `blacklist.contains(&rel)` (`:228`).
- Any entry the launcher wrote in the manifest's raw spelling that is not already
  the cleaned form — `./game/foo.dat`, `game/./foo.dat`, `game//foo.dat` — is
  therefore absent from the comparison set.
- Starward compares the two raw spellings (`x.File == item.FileName`,
  `GamePackageService.cs` at the cited commit), so a `./`-spelled entry matches
  there; girpr cleans one side only, so it fails exactly where Starward
  succeeds.

**Failure path.** A blacklist file containing `{"fileName":"./game/foo.dat"}`
against a manifest that spells the same file `game/foo.dat` (or `./game/foo.dat`)
leaves `blacklist.contains("game/foo.dat")` false. The file is planned, checked
and, if damaged, rewritten — the opposite of the user's request. Because it is
now in the plan, it is also in `expected_paths()`, so the purge spares it; the
two bugs (V-1 and V-2) mean an in-scope blacklist entry either fails to protect
from repair or causes deletion under a purge flag, depending on its spelling.
There is no spelling under which that run does the documented thing.

This violates binding decision 14 ("a server/config spelling is normalised once,
at its boundary") for the blacklist config input, and the `read_blacklist` doc
(`src/util.rs:534-538`) says nothing about the entry form.

**Suggested fix.** Normalise the parsed set once, at the reader boundary, and
fail fast on an entry that cannot be a contained rel. An entry that cannot name
a manifest file is a broken explicit configuration, and silently dropping it
reproduces this very fail-open; the file is launcher-written from already
validated manifest names, so such an entry means corruption or a hand edit, not
the crash-truncated JSON line that T-1's per-line warning exists for. The
existing `replace('\\', "/")` is subsumed by `clean_rel` and can be dropped:

```rust
let set = parse_config_list(path, "blacklist", &text, |v| {
    v.get("fileName").and_then(|n| n.as_str()).map(str::to_string)
})?;
set.into_iter()
    .map(|e| util::clean_rel(&e).with_context(|| format!("blacklist entry {e:?}")))
    .collect::<Result<HashSet<_>>>()
    .map(Some)
```

If maintainers prefer T-1's mixed-file tolerance, warn and skip the entry
instead (the shape `fetch_deprecated` uses at `src/repair/mod.rs:560-575`); the
essential fix is that the set holds the cleaned form, not the raw one. Add a
unit test with `./a/b.dat` and `a//b.dat` asserting the set is `a/b.dat`, and
one with `../a` asserting the error names the entry.

---

## V-3 — the local reuse map is keyed on the raw manifest spelling

**Evidence.**

- `build_local_chunk_map` inserts `f.file.clone()` verbatim
  (`src/sophon.rs:661-683`, key at `:670`). The local manifests come from a
  second fetch (`src/repair/phases/manifest.rs:52-73`) whose files are never
  cleaned.
- Every consumer looks up the **cleaned** plan rel:
  `temp.rs:119`, `inplace.rs:77`, `scan.rs:106` and `:285`, and the estimate at
  `plan.rs:153`.
- `PlannedFile.rel` is the `clean_rel` output (`src/repair/phases/plan.rs:222-223`,
  `:254-259`), so `./game/foo.dat` in the local manifest becomes the key
  `./game/foo.dat` while the lookup is `game/foo.dat`.
- The comment in `src/util.rs:343-349` states the invariant this breaks: the plan
  string is the one "every later comparison (the purge's expected set, **the
  reuse map**, the scope) works on".

**Failure path.** A local manifest whose paths are spelled non-canonically
(`./game/foo.dat`) is enough; a spelling difference between the separately
fetched local and latest manifests also suffices. Either way there is no
`local_map` hit. `try_reuse_slice` receives `&[]`, every chunk is downloaded,
and the scan/plan estimate counts the whole file. That is fail-safe for
correctness, but it is extra traffic and, with `--max-download-bytes`, can
refuse a run at the estimate (exit `5`) that would have reused local bytes;
`--plan-out` reports the inflated figure as well. It also silently defeats the
feature the map exists for.

**Suggested fix.** Clean the key where it enters the map. The function is
already best-effort by contract (`manifest.rs:49-51`), so warn and skip an entry
that cannot be cleaned rather than widening the signature:

```rust
let Ok(key) = util::clean_rel(&f.file) else {
    tracing::warn!(file = %f.file, "local reuse map: skipping a non-contained path");
    continue;
};
let e = map.entry(key).or_default();
```

Two raw spellings that clean to one key then merge, which is the intended
"first occurrence per md5/size" behaviour. Add a unit test to the existing
`sophon.rs` map test (`:1045`) with a `./`-spelled file and assert the key is the
clean form. A reuse integration test would need the fixture to serve a local
manifest whose paths are spelled non-canonically, which the mock's single
per-build spelling does not currently express.

---

## Ninth pass re-check (U-1…U-3)

- **U-1** — `clean_rel` is called once at each boundary it names: the manifest
  rel (`plan.rs:222`), the server's deprecated list (`src/repair/mod.rs:560-575`,
  warn-and-skip) and the audio dirs (`phases/metadata.rs:66-70`,
  `clean_config_dir`). The three `./`-spelling tests
  (`tests/repair_offline.rs:1870-1984`) are genuine: each builds the excluded or
  in-scope path on disk, and each fails on the pre-fix shape (raw-spelling gate,
  cleaned sink). The audio-dir normalisation after the unsupported-download-mode
  check (`27e7a18`) preserves the documented usage error. No defect found in the
  fix itself; V-2 is the same class at a boundary U-1 did not cover.
- **U-2** — `Lock::acquire` makes one bounded takeover attempt; a failed removal
  is a contextual error naming the path and the I/O error, and a raced fresh
  lock is refused (`src/repair/lock.rs:66-112`). The injected-removal unit test
  asserts exactly one attempt. `is_stale`'s metadata error reading as "not
  stale" refuses rather than steals, which is fail-closed. No defect.
- **U-3** — every module path written by the ninth pass resolves in
  `git ls-files` (`phases/…`, `purge.rs`, `file/mod.rs`); no stale pre-split
  path remains in the reviewed docs. No defect.

## Verification

Run on this tree, Windows, debug profile, during this pass:

- `cargo build -p girpr` — ok (warm).
- `cargo test -p girpr` — **342 passed, 0 failed, 7 ignored** (200 lib + 7
  `artifact_server` + 17 `cli_contract` + 25 `repair_machinery` + 93
  `repair_offline`; ignored: 6 `artifact_bridge` + 1 `peak_heap`; 0
  `src/main.rs`, 0 doc-tests).
- `cargo clippy -p girpr --all-targets` — clean, no warnings.
- `cargo fmt --check` — exit `0`.

The three findings were confirmed by source reading and the Starward parity
check; no test or fixture was changed, so the suite result is the tree as
committed.

## Not verified

- V-1, V-2 and V-3 were not reproduced end-to-end. The fixture harness sets
  `enable_resource_blacklist: false`, so there is no blacklist integration path
  to drive; V-3 additionally needs a local manifest whose paths are spelled
  non-canonically. All three are code-reading claims with the cited line numbers.
- The `#[cfg(unix)]` blocks (containment, mode, `file_len`, file-as-parent) were
  not run on this Windows host.
- The 7 ignored tests (`artifact_bridge` ×6, `peak_heap` ×1) were not run.
- No release-profile run, no Linux/macOS host, no live CDN traffic (all tests use
  the in-process mock).
