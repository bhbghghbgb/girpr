# ADVERSARIAL REVIEW NOTE - fifth pass

**Baseline:** branch `fix-gaps/gircap`, HEAD `778aca6`, clean tree. The diff
against `main` is confined to `tools/capture`. Reviewed 2026-10-28. The four
earlier passes remain in `ADVERSARIAL_REVIEW.md`; this file records only the
fifth pass. No source, test, Cargo or documentation file was changed.

## Findings

| ID | Severity | Class | Location | One-line summary |
|----|----------|-------|----------|------------------|
| I1 | medium | bug | `src/select/mod.rs:540`, `:552` | Two distinct forced `--files` paths with the same primary class get the same `capture_id`; the second patchmap overwrites the first and one requested file disappears. |
| I2 | low | bug (accounting) | `src/http/fetcher.rs:863`, `:893` | A `2xx` body that fails after N bytes is recorded with `bytes_in: 0`, losing the bytes the contract promises to count. |
| I3 | low | omitted error | `src/verify/mod.rs:245`, `:252` | `read_patchmaps` drops `read_dir` entry errors and silently overwrites two patchmaps that declare the same `capture_id`. |
| I4 | low | omitted error | `src/verify/mod.rs:1050`-`:1056` | `list` silently skips an unreadable directory and drops failed directory entries, so `Index::unreadable` is not the complete inventory it claims to be. |
| I5 | low | latent bug | `src/manifest/dump.rs:78`, `:84`, `:93` | `dump::write` validates `matching_field` before using it as a path component but interpolates `tag` raw into four output paths. |

## Fix log

Each entry names the source change, its regression test and the commit that
carries them. The findings above are left as written so the fixes can be read
against the report.

### I1 - fixed

`forced_ordinals: BTreeMap<Class, usize>` in phase 0 numbers every forced path
per primary class, admitted or rejected, so two `added` paths become `added` and
`added+1` (`src/select/mod.rs`). Phase 1's `taken` is deliberately unchanged:
starting it at the forced count would turn a budget-refused class into
`Unavailable` and break `a_class_whose_capture_was_refused_is_over_budget_not_absent`.
`emit/fixture.rs::record` additionally refuses a duplicate `capture_id` before
the first write, so a hand-built `Selection` cannot overwrite a patchmap either.
Tests: `select::tests::two_forced_paths_of_one_class_get_distinct_ids` (unit;
before the fix the ids were `["added", "added"]`) and
`two_forced_paths_of_one_class_keep_distinct_patchmaps` (e2e; before the fix the
bundle held six patchmaps and `verify` rebuilt six, not seven).

---

### I1 - Forced paths of one primary class share a capture id

**Severity: medium. Class: bug. Location:** `src/select/mod.rs:540`, `:552`;
`src/emit/mod.rs:192`; `src/emit/fixture.rs:248`-`:259`.

Phase 0 admits every forced path with ordinal zero:

```rust
// select/mod.rs:551-553
admitted.push(Admission {
    capture_id: c.candidate.capture_id(primary, 0),
    primary_class: primary,
```

and `capture_id` only appends the ordinal when it is nonzero:

```rust
// select/mod.rs:66-71
pub fn capture_id(&self, primary: Class, nth: usize) -> String {
    if nth == 0 {
        primary.id().to_string()
    } else {
        format!("{}+{nth}", primary.id())
    }
}
```

`validate_paths` rejects only *identical* strings (`src/config/mod.rs:498`).
Two distinct paths whose `classes.first()` is the same class are accepted and
both receive, for example, `added`. `write_patchmap` names the file from the id:

```rust
// emit/mod.rs:192
write_doc(root, &format!("patchmaps/{capture_id}.json"), map)
```

**Failure scenario.** `--files GenshinImpact_Data/x.bin --files
GenshinImpact_Data/y.bin`, both `added`, unlimited budget. Both are admitted as
`added`. The assembler writes `patchmaps/added.json` twice; the second write
replaces the first. `side_digests.insert` (`emit/fixture.rs:251`) and
`assembled_ids.push` (`:248`) both see two entries, so `MANIFEST.json` reports
`files_captured: 2` (`emit/index.rs:165`) and `patchmaps_written: 2`
(`run/mod.rs:420`) while one patchmap exists. The matrix resolves the class to
the *first* admission (`select/mod.rs:771`-`:776`), while the surviving
patchmap describes the second path. `verify` keys captures by the patchmap's
declared id, so it rebuilds only the second path and finds nothing inconsistent;
nothing compares a matrix row's `path` with the patchmap's `path` for the same
id. A requested file is therefore absent from the bundle, and three documents
disagree about how many captures it holds.

This is exactly the collision F9 described ("the second overwriting the first",
`src/config/mod.rs:487`-`:491`); the fix rejected repeated strings without
rejecting distinct paths that resolve to one primary class.

**Why it fails the standard.** A forced path is a caller's explicit request.
Silently dropping one, and recording two captures for one artifact, violates
fail-fast and makes the manifest's totals false.

**Suggested fix.** Number forced admissions per class, e.g. a
`BTreeMap<Class, usize>` counter used at `:540`/`:552`, and start phase 1's
`taken` at the number already forced for that class. Alternatively reject two
forced paths with the same primary class as a usage error. Independently, assert
at emit time that `assembled_ids` contains no duplicates and that the number of
unique ids equals the number of patchmap files.

---

### I2 - `bytes_in: 0` after a partial `2xx` body

**Severity: low. Class: bug (accounting). Location:**
`src/http/fetcher.rs:853`-`:868`, `:883`-`:898`.

`drain_capped` counts into a local `seen` and loses it on every error:

```rust
// fetcher.rs:146-163
let mut seen = 0_u64;
loop {
    match resp.chunk().await {
        Ok(Some(chunk)) => {
            seen += chunk.len() as u64;
            if seen > cap {
                return Err(too_big(url, seen, cap));
            }
            ...
        }
        Ok(None) => return Ok(seen),
        Err(e) => return Err(body_failure(url, attempts, true, e)),
    }
}
```

Both success destinations then hard-code zero in their failure record:

```rust
// fetcher.rs:853-867
let fail = |err: ApiError| {
    let record = self.record(
        ...
        0,
        attempts,
        started,
    );
    (err, record)
};
```

**Failure scenario.** A `206` chunk response sends 10 KiB and the origin closes
the connection. `drain_capped` returns `Err(Transport)` with `seen == 10240`;
`fail` records `bytes_in: 0`; the retry succeeds. `api/requests.ndjson` says the
first request cost nothing, `RequestLog::bytes_in` (`http/log.rs:111`) sums the
zero, and `MANIFEST.json` `totals.bytes_downloaded` (`run/mod.rs:351`, `:444`)
under-reports. The non-2xx path is correct (`drain_error_body`, `:821`); only a
failing `2xx` body is mispriced.

**Why it fails the standard.** `http/mod.rs:22`-`:25` says `bytes_in` counts
"**every** body received, including the body of a failed request", and the
comment at `fetcher.rs:817`-`:820` repeats it. This is the same class of lie F5
fixed for error statuses, left in the success branch.

**Suggested fix.** Return the partial count with the error, e.g.
`Result<u64, (ApiError, u64)>` or an out-parameter, and pass it to `fail`
instead of `0`. A test needs a body source that yields bytes and then fails; the
existing offline HTTP tests cannot inject a mid-body transport error.

---

### I3 - Duplicate patchmap ids are silently collapsed

**Severity: low. Class: omitted error. Location:** `src/verify/mod.rs:244`-`:253`.

```rust
// verify/mod.rs:244-253
let mut paths: Vec<PathBuf> = rd
    .filter_map(|e| e.ok().map(|e| e.path()))
    .filter(|p| p.extension().is_some_and(|e| e == "json"))
    .collect();
paths.sort();
let mut out = BTreeMap::new();
for p in paths {
    let map: PatchMap = read_json(&p)?;
    out.insert(map.capture_id.clone(), map);
}
```

`BTreeMap::insert` returns the previous value, which is discarded. Keys come
from each document's declared `capture_id`, not from the file name, and nothing
checks the two agree.

**Failure scenario.** A bundle (hand-merged, restored from two captures, or
produced after a writer regression such as I1) holds `patchmaps/added.json` and
`patchmaps/added+1.json`, both declaring `capture_id: "added"`. The second file
in sorted order replaces the first. `verify` counts one patchmap, rebuilds one
capture, runs A20 against one document, and exits `0`; the other capture and
its blob references are never examined. A file named `added+1.json` declaring
`added` is accepted the same way.

**Why it fails the standard.** Two captures claiming one id is a structural
defect. Dropping one silently is the omitted-error class, and the bundle's own
`verify` is the last place it can be named.

**Suggested fix.** Keep the source path with the map and return an error naming
both files when an id repeats. Also reject a document whose declared
`capture_id` does not equal its file stem, so the directory listing and the
documents cannot disagree.

---

### I4 - `list` drops unreadable directories

**Severity: low. Class: omitted error. Location:** `src/verify/mod.rs:1050`-`:1056`.

```rust
// verify/mod.rs:1050-1056
let Ok(rd) = std::fs::read_dir(&dir) else {
    continue;
};
let mut subs: Vec<PathBuf> = rd
    .filter_map(|e| e.ok().map(|e| e.path()))
    .filter(|p| p.is_dir())
    .collect();
```

**Failure scenario.** `--root artifacts` where `artifacts/hk4e_global` cannot be
read (ACL, disconnected share, I/O error). The walk reaches
`artifacts/hk4e_global`, `read_dir` fails, and `continue` drops the whole subtree
without recording it. An entry error from the parent's `read_dir` drops one
child the same way. `list` returns an empty `bundles` and `unreadable` list and
succeeds.

**Why it fails the standard.** `verify/mod.rs:1013`-`:1019` promises that a
directory holding a bundle this build refuses is named so "the operator's
inventory is complete". An unreadable directory may hold bundles and is not
named at all; the operator gets a confident, incomplete answer.

**Suggested fix.** Record `(path, error)` for both the `read_dir` failure and
each failed entry in a `skipped` list (or reuse `unreadable`), print it on
stderr, and make the exit status nonzero when the walk could not finish. Fail
fast is preferable for an audit command: return `Err` naming the directory.

---

### I5 - `dump::write` validates the field but not the tag

**Severity: low. Class: latent bug. Location:** `src/manifest/dump.rs:78`,
`:84`-`:89`, `:93`.

```rust
// dump.rs:78-93
let tag = captured.tag.clone();
let field = captured.matching_field.clone();
...
if let Some(problem) = crate::paths::field_component_problem(&field) {
    return Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!("manifest field {problem}"),
    ));
}
...
let archive = crate::emit::write_rel(root, &archive_rel(&tag, &field), &captured.raw)?;
```

`archive_rel`/`dir` interpolate `tag` raw, and `write_parts` does
`root.join(rel)` with no further check (`emit/mod.rs:319`).

**Failure scenario.** An in-crate caller builds `CapturedManifest` directly
(tests already do) with `tag: "../../escape"` and calls `dump::write`. The first
write lands outside `root`. The CLI cannot reach this today: `VersionPair::select`
validates both resolved tags (`config/mod.rs:117`-`:124`) before the pair is
named. The gap is in the writer's own defence: its comment says the field check
exists so "a caller that built a `CapturedManifest` itself gets the same refusal
the capture path does", and `tag` is the other path component in the same four
names.

**Why it fails the standard.** The last sink validates one of the two
interpolated components. H2 closed exactly this asymmetry for the field; the
tag remains a latent traversal if a second caller appears.

**Suggested fix.** Run `config::tag_shape_problem` (or the shared path rule) on
`tag` before the first `create_dir_all`, with an error naming `manifest tag`,
and add a test mirroring `a_field_that_names_a_path_is_refused_before_anything_is_written`.

---

## Prior fixes re-checked

- **F9 (duplicate `--files`)**: still rejects only identical strings; I1 is the
  distinct-path residue of the same defect.
- **F5 (failed bodies are counted)**: correct for non-2xx; I2 is the `2xx`
  branch.
- **G1/H1 (document paths)**: `paths.rs` and `validate_document_paths` still
  guard manifest, index and patchmap paths; no regression found.
- **H2 (field is a path component)**: `dump::write` and `manifest::capture`
  still validate `matching_field`; I5 is the tag beside it.

## Verification

Run on the current tree (HEAD `778aca6`, clean) on 2026-10-28; logs in
`E:\Symlinks\Temp\opencode\gircap-*.log` (outside the repository).

| Command | Result |
|---|---|
| `cargo build -p gircap` | success, `Finished dev profile ... in 49.91s`, no warnings |
| `cargo test -p gircap` | **658 passed, 0 failed** (518 lib; 3 `bundle_consistency`, 8 `bundle_diff`, 32 `capture_layout`, 2 `case_coverage`, 30 `cli_contract`, 9 `matrix`, 29 `measured_fixture`, 12 `offline_bundle`, 15 `schema`; 0 doc-tests) |
| `cargo clippy -p gircap --all-targets` | success, `Finished dev profile ... in 22.93s`, no diagnostics |
| `cargo fmt -p gircap --check` | exit 0, no output |

This note was added after those runs and changes no code; `git status` was clean
before it was written.

## Not verified

- No live-network run. I2 needs a mid-body transport failure; the offline tests
  do not inject one, so it is source-verified only.
- I1 was not reproduced end-to-end: it needs two real diff paths whose primary
  class matches. The collision is established from the admission, write and
  matrix code, not from a produced bundle.
- I3 and I4 were not exercised against a crafted bundle with duplicate ids or an
  ACL-denied directory. The Windows permissions needed for I4 were not set up.
- I5 is not reachable through the CLI; no test calls `dump::write` with a
  hostile tag.
- The audit did not re-read every line of the four earlier passes. Spot checks
  are listed above; the rest is covered by the passing suite, not by a new
  line-by-line review.
- Not reviewed: root `girpr`, `tools/sync`, `crates/log`, `crates/tempdir`
  beyond their use as dependencies; generated artifacts and fixture data.
- No fuzzing, property testing, or Windows-vs-Unix comparison.
