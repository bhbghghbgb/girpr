# ADVERSARIAL REVIEW — `girpr` (root crate)

Review of the `girpr` package in workspace `E:\hoccode\hocrust\girpr`:
source under `src/`, tests under `tests/`, shared leaf crates `crates/log` and
`crates/tempdir`, `docs/`, `Cargo.toml`/`Cargo.lock`, and the clap-derived CLI
surface.

**Method.** Read every source file, every test file, and the docs; ran
`cargo build`, `cargo nextest run -p girpr` (270 passed, 0 failed),
`cargo clippy --all-targets -p girpr` (clean), and reviewed the documented
contract (README machine contract, `docs/04-scope-limitation-spec.md`) against
the code rather than the other way around.

Each finding is marked:

- **(a) bug** — incorrect or unexpected behavior, even if documented as
  intentional.
- **(b) intentional behavior lacking test coverage** — behaves as designed but
  not pinned by a test; recommend a pinning test (not written here, per the
  read-only constraint).

The suite is strong: scope, ceilings, retry policy, live/settled byte split,
mode selection, salvage order, trim, resume, and audio are all well covered.
The gaps below are real but small in number.

---

## F-1 (a) Bug — high — Per-file task panics are silently swallowed

**Behavior.** In `repair_all`, every file task is awaited with the result
discarded:

```rust
for h in handles {
    let _ = h.await;
}
```

`src/repair/file/mod.rs:292-294`

If a task panics (not an `anyhow::Error` — those are handled), the join error
is a `JoinError::Panicked` and is thrown away. The panicked file is counted in
`files_total` but in none of `skipped`/`repaired`/`failed`, so `PROGRESS`
never completes for it; the run still proceeds to the post-phase, may bump
`config.ini`, and can exit `0` while a planned file was never repaired and the
tree is left half-repaired. There is no log of the panic either.

**Concrete trigger.** `src/repair/file/temp.rs:193`:

```rust
let end = (c.offset + c.uncompressed_size) as u64;
```

`c.offset` and `c.uncompressed_size` come straight from the (untrusted)
manifest as `i64`. A manifest advertising e.g. offset `i64::MAX`, size `1`
overflows `i64`: a panic in debug builds (which is what `cargo nextest` and CI
run), a silent wrap in release. Either way the task dies. The same silent
swallowing applies to `scan_plan`'s scan tasks:

```rust
let Ok(scanned) = res else { continue };
```

`src/repair/scan.rs:131-132` — a panicked or failed scan task makes the file
"unreached" and it is estimated in full, silently.

**Evidence.** Code reading; no test exercises an oversized/panicking manifest.
All existing tests pass because manifests in the mock are well behaved.

**Severity.** High: a silent exit-0 with an unrepaired file and an advanced
`config.ini` is exactly the launcher-desync scenario the post-phase ordering
exists to prevent.

**Recommended action.** Fix: await each file task; treat an `Err` result as a
file failure (count `files_failed`). Do **not** handle a task panic — let it
remain swallowed (matching the other projects' behavior). Use checked arithmetic
(`checked_add`, checked casts) in `try_reuse_slice` and `PlannedFile::layout`
for manifest-sourced offsets/sizes. Recommend a pinning test: a manifest with a
huge offset must fail the file (exit 3, `failed=1`), never silently succeed.

---

## F-2 (a) Bug — medium — No containment of manifest `rel` paths inside `game_dir`

**Behavior.** Manifest-relative paths are accepted verbatim:

- `src/repair/phases/plan.rs:187`: `let rel = f.file.replace('\\', "/");` — no
  rejection of `..` segments, absolute paths, or leading `/`.
- `src/repair/file/mod.rs:329`: `game_dir.join(util::normalize_rel(&file.rel))`
  — repair target.
- `src/repair/scan.rs:148` and `src/util.rs:277-279` — same join for scan and
  for reuse-slice reads (`temp.rs:217`).
- Purge: `plan_purge` walks `game_dir` with `follow_links(false)`, but
  `rel_path` (purge.rs:200) strips the prefix symlink-style, and
  `e.metadata()` (purge.rs:107) follows symlinks, so a symlink inside the
  install can be reported at the target's size while only the link is removed —
  the delete ceiling and the bytes actually freed disagree.

A malicious or corrupted manifest listing `../../escape/evil.exe` makes the
repair write (and the scan read) outside the install. Nothing in `build_plan`
or the writers rejects it.

**Evidence.** Code reading at the locations above; tests only ever use plain
`game/...` paths, so traversal is never asserted.

**Severity.** Medium: writes outside the install driven by server data; requires
a hostile/corrupt manifest, not a typo.

**Recommended action.** Fix: validate each `rel` at plan-build time — reject
`..` segments, absolute paths, and paths that escape the root — and fail the
plan with **exit code 2** (API is variable, not user error). Recommend a
pinning test: manifest with `../outside.dat` is refused, no write outside
`game_dir`.

---

## F-3 (b) Intentional, not covered — Partial failed fetch over-refunds the download ceiling

**Behavior.** `write_downloaded_chunk` reserves `compressed_size` against
`--max-download-bytes` before fetching (correct: bounds live traffic
precisely), but on ANY fetch failure it refunds the **whole** reservation:

```rust
let digest = match sophon::stream_chunk_to(...).await {
    Ok(d) => d,
    Err(e) => {
        ctx.summary.adjust_download_bytes(-(reserved as i64));
        return Err(e).with_context(|| format!("chunk {}", c.id));
    }
};
```

`src/repair/file/temp.rs:368-373`

The comment at lines 354-357 says "A failed fetch delivers nothing, so the
reservation goes straight back." That is false for a **partial** failure: the
stream may have delivered most of the body before the transport died, but the
live download counter is refunded in full, so `--max-download-bytes` can be
under-enforced by up to one partial chunk per failed fetch. The over-count
direction (manifest lies about `compressed_size`, claim smaller than reality)
also lets the live figure exceed the ceiling silently — docs/04 §B2 relies on
capture invariant A6 (`len(blob) == compressed_size`), which `girpr` trusts but
does not enforce against a mock or corrupted CDN.

**Evidence.** Code reading at temp.rs:342-377; the repo tests cover full-body
failure (`a_404_is_asked_once...`, `fail_fast...`, `download_bytes_counts...`)
but no case delivers a partial body and then fails.

**Severity.** Low: ceiling under-enforcement is bounded by one partial chunk per
failed fetch, and a failed chunk still fails the file attempt.

**Recommended action.** Fix or document: either charge the actual bytes
delivered before a mid-stream failure (hard to do exactly with a stream), or
correct the comment so the documented guarantee ("live counter = traffic
caused") is honest. Recommend a pinning test with a mock that streams a partial
body then drops the connection; assert the live figure behaves as documented
and the ceiling is not materially under-enforced.

---

## F-4 (b) Intentional, not covered — Temp-mode resume trusts a complete-length `_tmp`

**Behavior.** The temp path resumes by length (`temp.rs:198-204`): if
`cur_len >= end` the chunk is taken as done with no hash check of the existing
bytes. The rollback (`rollback_tmp`, temp.rs:161-173) only truncates after a
digest failure, and promotion is gated by final MD5 (`promote_tmp`, temp.rs:412
ff.), so a wrong `_tmp` cannot be *promoted* — but a `_tmp` that already has
the full length and wrong content fails at the final gate, is deleted, and the
file is retried from scratch (eventually exit 3 after retries). Safe, but the
"corrupt prefix of full length → MD5 fails → tmp removed → retry" path is not
directly asserted; the existing test (`a_corrupt_chunk_is_not_resumed_as_already_complete`,
`tests/repair_offline.rs:2813`) covers the mid-file rollback, not the
full-length case.

**Evidence.** Code reading; test coverage of the rollback exists but not of the
full-length corrupt prefix.

**Severity.** Low: no silent corruption, but wasted work and an opaque failure
mode.

**Recommended action.** Leave as intentional; recommend a pinning test: pre-seed
a full-length `_tmp` with wrong bytes, run `--repair-mode temp`, assert the run
does not promote it (MD5 gate) and either repairs on retry or fails with exit 3.

---

## F-5 (b) Intentional, docs inaccuracy — `--max-write-bytes` comment says `*_tmp`

**Behavior.** `cli.rs:89-95` documents `--max-write-bytes` as bytes "written
into `*_tmp`". For `--repair-mode inplace` (the default) and `discard`, the
bytes are written into the target file itself, not a `_tmp`; for `temp` they
land in `_tmp`. The README flag table (line 290) is more accurate ("written
into the install"). The code is correct (`reserve_write`/`note_write` charge
every write site: `write_at`, `copy_within`, `zero_fill`, the chunk sink, the
reuse-slice copy); only the flag help text is imprecise.

**Evidence.** `cli.rs:89-95` vs `src/repair/file/mod.rs:518-604` write sites.

**Severity.** Low.

**Recommended action.** Clarify the `cli.rs` doc comment to match the README:
"bytes written into the install" (mode-dependent file).

---

## F-6 (b) Coverage gap — `#[ignore]`d suites never run in the standard suite

**Behavior.** `tests/artifact_bridge.rs` (the cross-tool claim: patched bytes,
exact fetch set, byte total vs `patchmaps/<id>.json`) and `tests/peak_heap.rs`
(the streaming-path heap measurement) are `#[ignore]`d. `cargo nextest run`
compiles them but never runs them; the bridge additionally hard-fails only when
manually invoked with no bundle present (`tests/artifact_bridge.rs:48-52`).
Nothing in CI (as configured here) exercises real bundle bytes or the heap
number.

**Evidence.** `tests/artifact_bridge.rs:27-42`, `tests/peak_heap.rs:19-29`;
`cargo nextest run -p girpr` ran 270 tests and neither suite appeared.

**Severity.** Low: neither is a code defect; the bridge's "missing bundle is an
error, not a skip" design is good. But the strongest end-to-end claim and the
heap measurement are effectively opt-in.

**Recommended action.** Leave intentional; recommend documenting that the
bridge/peak_heap should be run on any repair-spec change (already in README)
and, if a bundle can be produced in CI, run them on a schedule. No code change.

---

## F-7 (b) Intentional, minor asymmetry — `--audio` under a scope is refused only when a write would occur

**Behavior.** `src/repair/metadata.rs:136-154`: an explicit `--audio` whose scan
file is outside the active scope is refused (exit 1) **unless** the run is
read-only (`--dry-run`/`--check-only`), because read-only modes never write the
scan file. Under `--dry-run` the REPORT still lists the requested audio as the
effective selection even though nothing is written. This follows from the
"dry-run writes nothing" contract, but the refusal condition and the report
differ.

**Evidence.** `src/repair/metadata.rs:136-154`; tested only for the mutating
case (`tests/repair_offline.rs:1271`, "explicit audio under a scope is refused").

**Severity.** Low.

**Recommended action.** Leave as intentional; clarify the docs that `--dry-run`
reports the requested audio without writing it. Optional pinning test: dry-run
+ explicit `--audio` + out-of-scope scan file → exit 0, scan file absent,
REPORT names the lang.

---

## F-8 (b) Intentional, operator-facing confusion — `--max-delete-bytes` post-pass vs. purge-before

**Behavior.** The delete ceiling is checked pre-flight on the union of both
purge passes (`limits.rs:122-127`) and again post-repair against the
post-repair re-plan (`post.rs:64-75`, `finish` at `mod.rs:514-525`). If
`--purge-before` already freed space, the post-pass ceiling can still hit;
`stopped=--max-delete-bytes` then means "the post pass deleted nothing," not
"the run deleted nothing," which can misread the SUMMARY.

**Evidence.** Code reading; tested only pre-flight refusal (`max_delete_bytes_refuses_and_deletes_nothing`,
`tests/repair_offline.rs:1491`) and out-of-scope exclusion.

**Severity.** Low.

**Recommended action.** Clarify `docs/04` §B4/README that the two passes are
checked independently and `stopped=` names the pass that bound.

---

## F-9 (b) Intentional, debatable classification — `--plan-out` write failure is exit 3

**Behavior.** `src/repair/mod.rs:331-333`: failure to write the `--plan-out`
document (a plan-phase artifact outside the install) is mapped to
`RunFailure::write` → exit 3 (write/verify class), the same class as install
write failures. It happens after the assert phase, so nothing in the install is
damaged.

**Evidence.** `src/repair/mod.rs:308-335`.

**Severity.** Low.

**Recommended action.** Decide: exit 3 is defensible (a write failed), but the
failure is outside the install. If kept, document it; no test covers
`--plan-out` write failure (unwritable parent). Recommend a pinning test if
the classification is kept.

---

## F-10 (b) Documented boundary — metadata/manifest traffic is not bounded by any ceiling

**Behavior.** Ceilings bound chunk downloads, writes, and deletes only; the
`getGameConfigs`/`getGameBranches`/`getBuild`/manifest fetches (~2–3 MB) happen
before the plan exists and are never refused. This is stated in docs/04 §2.2 and
the README flag table ("Does **not** bound metadata or manifest traffic"). Not
a bug, but consumers who read `--max-download-bytes` as a network cap will be
surprised on a slow/expensive link.

**Evidence.** docs/04 §2.2; README flag table; no ceiling is applied in
`src/repair/phases/manifest.rs` or `src/hyp.rs`.

**Severity.** Low (documented).

**Recommended action.** Leave as intentional; keep the docs prominent. No test
gap: refusals assert zero chunk hits (`max_download_bytes_refuses...`).

---

## Additional observations (not findings)

- **Exit-code remapping works.** `main.rs:79-89` remaps clap's `2` to `1` for
  all argument failures; `--help`/`--version` stay `0`. Covered by
  `tests/cli_contract.rs`. `into_ctx` failures (io-threads 0, bad audio, bad
  glob) are also exit 1 after tracing is initialized, so a usage error *does*
  leave a log file when `--log-file`/`GIRPR_LOG_FILE` names one — consistent
  with main.rs:14-30.
- **Log policy is consistent and tested.** `crates/log` (plan/level/filter)
  and its consumption in `src/logging.rs` agree on precedence
  (`--no-log-file` > `--log-file` > `GIRPR_LOG_FILE` > default), release-vs-debug
  default (`default_file_is_written` = `!debug_assertions`), separate console
  (`--log-level`, `RUST_LOG` adds) vs file (`--log-file-level`,
  `GIRPR_LOG_FILE_LEVEL`) levels, and infallible open failure. Covered by
  `crates/log` unit tests and `tests/cli_contract.rs`.
- **`crates/tempdir`**: unique pid/timestamp/seq names under `<temp>/girpr-test/`,
  `Drop` removal, `absent`/`at`/`subdir` variants; well tested, including the
  five-part name shape. No findings.
- **`--scan` exactness.** `tests/repair_offline.rs:869` (`the_scanned_estimate...`)
  pins that the scanned estimate equals the real download cost. Well covered.
- **Retry policy.** 404 asked once, 5xx/429/408 retried 5× linear; terminal
  retcodes `-202/-201/-1001` not retried. Covered (`hyp.rs:205`, `sophon.rs:553`,
  offline tests).
- **`--fail-fast`** caps attempts at 1 and aborts subsequent files; counters stay
  honest. Covered.
- **Local manifest degradation** is visible via `local_manifest=ok|unavailable`
  and covered both directions (`tests/repair_offline.rs:2975`, `repair_machinery.rs`).
- **Scope invariant** ("filtered-out path does not exist on both sides") is
  implemented in `build_plan`, `classify_purge_path`, `delete_deprecated`,
  `move_audio_cache`, and the config.ini/no-bump and audio-write guards; covered
  end to end.

---

## Summary of recommended actions

| # | Class | Severity | Action |
|---|-------|----------|--------|
| F-1 | (a) bug | high | Stop swallowing file-task `Err` results (count as `files_failed`); do not handle task panics; checked arithmetic on manifest offsets/sizes; pinning test for oversized manifest offset. |
| F-2 | (a) bug | medium | Reject `..`/absolute/out-of-root `rel` paths at plan time — fail the plan with exit code 2 (API is variable, not user error); pinning test. |
| F-3 | (b) intentional | low | Correct the partial-refund comment or charge actual delivered bytes; pinning test with a partial-body-then-drop mock. |
| F-4 | (b) intentional | low | Pinning test for full-length corrupt `_tmp` under `--repair-mode temp`. |
| F-5 | (b) docs | low | Fix `--max-write-bytes` help text in `cli.rs`. |
| F-6 | (b) coverage | low | Keep ignored; consider scheduled CI runs of bridge/peak_heap when a bundle is available. |
| F-7 | (b) docs | low | Clarify dry-run `--audio` reporting vs. writing. |
| F-8 | (b) docs | low | Clarify that `--max-delete-bytes` is checked per purge pass. |
| F-9 | (b) classification | low | Document or re-classify `--plan-out` write failure; optional pinning test. |

No code or tests were modified in this review. The report is ready for human
review; no PR or commit was created.