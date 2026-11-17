# 08 — Test-suite reduction audit

**Status:** analysis only — no source files were changed.
**Date:** 2026-11-18
**Method:** five parallel read-only audits covering every crate in the workspace:
root product integration tests, root product unit tests, `tools/sync`, `tools/capture`,
and the small crates (`crates/log`, `crates/tempdir`, `tools/sled2redb`) plus a
workspace-wide cross-cutting pass. Each audit read the production code behind the
tests it assessed. Measured total: **2,260–2,268 test attributes**
(`#[test]` / `#[tokio::test]` / `#[rstest]`), plus 12 `#[ignore]`d tests.

## Headline verdict

The suite is unusually well curated and heavily **mutation-driven**: most tests
carry a comment naming the exact defect they pin, and the obvious redundancy has
already been removed in several places (e.g. `tools/sync/tests/sync.rs:131-138`
records a past merge). There is **no large block of useless tests** to delete.

Reductions that are defensible fall into three buckets:

1. **Merge/parametrize** many small single-assertion tests of one pure function
   into one table-driven test — **no coverage lost**, lowest risk. This is the
   bulk of the opportunity.
2. **Remove** a small number of tests that assert nothing new (duplicated helper
   self-tests, no-op / tautological tests, a weaker test subsumed by a strict pin).
3. **Reduce scale** by replacing process/HTTP/e2e setup with direct function calls
   — but in this workspace this almost never lowers the *test count*, because the
   process-level tests assert on channels (JSON record stream, stderr, exit-code
   remapping, clap errors) that exist only in the binary. It saves runtime, not test
   functions.

**Conservative estimate: ~70–90 removable tests (~3–4 % of 2,260), all low-to-medium
risk.** Up to ~110 if the optional/aggressive merges below are also taken. No finding
justifies a large-scale deletion. If no changes are made, that is a valid outcome —
the remaining tests each carry distinct, documented coverage.

## Summary by area

| Area | Low-risk merges / removes | Higher-risk / optional | Notes |
|---|---|---|---|
| Root product integration (`tests/`) | ~30–35 | — | Almost entirely MERGE, concentrated in `cli_contract.rs` and `run_lock.rs` |
| Root product unit (`src/**`) | ~13–16 | ~4 (`purge.rs`) | Duplicated fixture-helper meta-tests are the main win |
| `tools/sync` | ~5–10 | ~5 (`migration_lock.rs`) | 633 tests; already has explicit merge notes |
| `tools/capture` | ~6–7 | ~8–10 (measured-fixture) | ~870 tests; mutation-driven; mostly KEEP |
| Small crates + cross-cutting | ~19–22 | — | Cross-binary log-policy duplication is the main win |
| **Total** | **~73–90** | **up to ~110** | Overlap between areas noted below |

## Detailed findings

### A. Log-file CLI policy re-tested in all three binaries (~12–14 tests, medium risk)

One shared policy in `crates/log` is asserted at three layers, three times over.
The per-binary CLI tests repeat the same five cases:

- `tests/cli_contract.rs` — `the_default_log_file_follows_the_build_it_was_built_from` (1070),
  `no_log_file_turns_the_file_off_even_when_the_environment_names_one` (1104),
  `no_log_file_and_log_file_together_are_a_usage_error` (1133),
  `the_environment_names_the_log_file_when_the_flag_does_not` (1162),
  `the_environment_sets_the_log_files_level` (1265).
- `tools/capture/tests/cli_contract.rs` — same five at 529, 565, 586, 610, 698,
  plus `the_file_is_louder_than_the_console_by_default` (635) and
  `the_log_file_level_flag_narrows_the_file` (665).
- `tools/sync/tests/output.rs` — same five at 632, 670, 697, 728, 548,
  plus the same two at 483, 517.

Plus per-binary unit duplicates `the_default_file_is_named_for_the_project` in
`src/logging.rs:359`, `tools/capture/src/logging/mod.rs:257`,
`tools/sync/src/logging.rs:233`. The policy's shape is already covered by ~23 unit
tests in `crates/log/src/{plan,filter,level}.rs` (the CLI comments say so themselves).

**Action:** keep girpr's CLI set as the end-to-end representative; reduce gircap and
girsync to one smoke case each, moving the env-prefix check into their `logging` unit
tests. *Note:* girpr's own log-file tests are also assessed in §B — count them once.

### B. Root product integration tests (`tests/`) — ~30–35 tests

All reductions here are MERGE (no coverage lost) except one pure REMOVE. The
`repair_*` integration tests already call the library via `common::run`
(`tests/common/mod.rs:131`) and only use an in-process `MockServer`; genuine HTTP
behavior cannot be shortened further. Process-spawning tests
(`cli_contract.rs`, `process.rs`, `repair_plan.rs`, `repair_post.rs`) generally batch
several modes into one function, so converting them to library calls would save
runtime but not count.

1. **`tests/cli_contract.rs`** (49 tests) — three clusters test the file's *own*
   parsing/assertion helpers, not product code:
   - Help-entry / option-body / indent self-tests — 12 tests
     (201, 213, 228, 260, 289, 308, 340, 360, 381, 399, 568, 589) → collapse into
     **1–2** table-driven tests (LF/CRLF, short alias, boundary, panic text, indent
     arithmetic). **~10–11 saved, low risk.**
   - Real-help layout (clap dependency behavior) — 3 tests (476, 511, 540) → **1**
     parametrized test over the indent columns. **2 saved, low risk.**
   - `text_records` parser self-tests — 4 tests (1698, 1719, 1742, 1763) → **1** with
     the `#[should_panic]` case via `catch_unwind`. **3 saved, low risk.**
   - Pure REMOVE: `the_golden_rejects_a_reorder_a_duplicate_a_misplaced_progress_and_a_rename`
     (2130) tests the test-only `check_golden` helper on synthetic values; the real
     goldens (1936–2065) exercise it. **1 saved, low–medium risk.**
   - Log-file process tests — 11 tests (981–1331) with identical shape, varying only
     flags/env → **4–5** table-driven process tests (keep them as process tests).
     **~6 saved, medium risk.**
2. **`tests/run_lock.rs`** (15 tests) — the whole file guards the test-support helper
   in `tests/common/run_lock.rs`, not product lock behavior (which is covered in
   `repair_offline.rs:348/408/441`). Consolidate the case clusters: `game_path_arg`
   (108, 131, 156, 175, 203, 238) → 1; mode/args mirrors (63, 86) → 1;
   `lock_key_path` (356, 364) → 1; cleanup (376, 390, 403) → 1; CLI bad-game-path
   (319, 346) → 1. **15 → ~5, ~9–10 saved, medium risk** (keep one end-to-end cleanup case).
3. **`tests/repair_plan.rs`** — three audio `--print-plan` process tests (117, 170,
   225) differ only by `--include`; add a scope column to a case table. **2 saved,
   low–medium risk.**
4. Smaller optional merges: `repair_offline.rs` branch-failure pairs (514/560,
   662/699), `repair_post.rs` non-UTF-8 switch pair (485/497), `repair_scope.rs`
   include-nothing pair (184/414). **~5 saved, low–medium risk.**
   - **Do not merge** `repair_post.rs:512/596` (library-vs-binary variants pin
     different layers) or the `repair_purge.rs:421/454`, `repair_scope.rs:212/342`,
     `repair_plan.rs:477/503` pairs (complementary, not duplicate).

### C. Root product unit tests (`src/**`) — ~17–20 tests

1. **Duplicated fixture-helper meta-tests (~5–7, LOW risk).** These test the
   test-support code that decides whether to skip a case when a host cannot create a
   symlink/junction. The same assertions appear in three files; the canonical owner
   is `src/util/junction_support.rs`:
   - `src/util/path.rs:1045, 1077, 1085`
   - `src/util/ini.rs:1230, 1291, 1300` — the last two are byte-identical to
     `path.rs:1077/1085`.
   Remove the 6 copies in `path.rs`/`ini.rs`; keep `junction_support.rs`. Also
   `src/util/ini.rs:1370` (`link_guard_retries_a_failed_explicit_removal_on_drop`)
   tests only a test-local `LinkGuard` helper — removable.
2. **`src/main.rs` worker-thread arithmetic (3, LOW).** `a_single_job_keeps_runtime_headroom_on_one_core`
   (166), `host_parallelism_wins_when_it_exceeds_the_file_window` (171),
   `a_huge_file_window_saturates_instead_of_overflowing` (176) → one parametrized
   test over the pure `worker_threads(jobs, window)`.
3. **`src/repair/reuse.rs` salvage_order (~5, LOW–MEDIUM).** Seven 2–4-line asserts
   (224, 232, 240, 250, 258, 264, 294) → one table-driven test. Keep the four
   distinct cases at 272, 283, 306, 324.
4. **`src/util/ini.rs:1075/1091` NUL game-dir inspection (1, LOW).** Both call
   `game_dir_refusal` with a NUL path; combine.
5. **`src/repair/purge.rs` classifier consolidation (~3–4, MEDIUM).** Four early
   tests (667, 732, 751, 1204) re-assert cases in the parametrized
   `cached_keys_classify_every_keep_rule` (930). Verify parity before deleting
   (the early ones use `classify_purge_path`, 930 uses `classify_purge_path_visible`).
   Keep `the_keep_set_is_config_and_the_lock` (1191) and `a_hidden_path_is_kept…` (782).
6. **Kept intentionally:** the RFC-1321 / scan tests in `src/util/md5.rs` (the two
   throughput tests at 736/791 are `#[ignore]`d benchmarks), the table-driven bad-path
   tests in `path.rs`/`ini.rs`, and the canonical symlink/junction "fail closed" tests.
   Unit-vs-integration overlap (e.g. `src/cli.rs:478` vs `tests/cli_contract.rs:838`;
   `src/repair/phases/plan.rs:906` vs `tests/repair_offline.rs:166`) is **layered**,
   not duplicate — no removal recommended.

### D. `tools/sync` — ~5–10 tests (633 total)

1. `tests/output.rs:438/483` — two tests build the same fixture and run the same
   binary; fold the console-level assertion into the record-log assertion. **1 saved, LOW.**
2. `src/hash/tests.rs:900, 916, 930, 942` — four short dedup/order asserts → one
   table-driven `dedup_keeps_first_seen_order` (keep a comment per regression row).
   **up to 3 saved, LOW–MEDIUM.**
3. `tests/output.rs:517/548` — flag vs environment log-file level differ only in
   how the value is set → one parametrized test. **1 saved, LOW.**
4. `tests/migration_lock.rs` (11 tests) — 4–5 tests overlap dedicated suites
   (443≈`sync.rs:140`, 376≈`sync.rs:1986`, 241≈`helpers.rs:459`/`lazy/compare.rs:182`,
   480/279≈generic convergence). Its **stated purpose is a backend-agnostic behavior
   lock** (asserts cache-row semantics without pinning format), which is real. Trim only
   the clear duplicates; keep the row/content assertions. **~5 saved, MEDIUM**; deleting
   the whole target is HIGH risk.
5. **No safe REDUCE_SCALE cuts:** every `CARGO_BIN_EXE_girsync` test asserts on the
   JSON record stream / stderr / clap errors that exist only in the process; the
   library returns only an exit code.
6. **Kept:** `src/cache/open.rs` (42), `src/hash/tests.rs` (41),
   `src/commands/sync/plan/tests.rs` (41), `src/cache/db/cachedb.rs` (20) — each pins
   a distinct documented race/guard/contract. The `lazy`/`stat_trust`/`hash_mode`/
   `rw_threads`/`sync_plan`/`why` suites vary a different mechanism over a similar
   fixture and assert different properties; the hypothesis that they repeat scenarios
   through different layers is **not borne out**. No no-assertion tests found.

### E. `tools/capture` — ~6–7 tests safe, up to ~10 optional (~870 total)

1. **REMOVE no-op:** `src/hash/tests.rs:578` `the_measured_archive_digest_is_reproducible`
   only re-asserts `md5_hex(b"").len() == 32` (also at 546/472) and the zstd magic
   (also at 623/634); its doc comment claims to pin a 9,007,187-byte digest that never
   appears. **1 saved, LOW.**
2. **REMOVE redundant process test:** `tests/cli_contract.rs:214`
   `a_bad_log_level_is_refused_by_the_parser` — its three assertions are covered by
   `every_argument_error_is_one` (142), `src/cli/mod.rs:1002`, and the stdout contract
   (197/229). **1 saved, MEDIUM.**
3. **FOLD assertion-free test:** `src/emit/closeout.rs:1000` `a_clean_close_out_returns_ok`
   calls `seed()/run()/remove_dir_all()` with no assertion; neighbouring tests (931/956/1008)
   call the same happy path and assert more. **1 saved, MEDIUM.**
4. **MERGE small same-function pairs (5, LOW–MEDIUM):**
   `src/report/tests.rs` 321/331, 142/152, 589/605; `src/config/tests.rs` 411/483;
   `src/manifest/proto.rs` 225/243 (the latter's "no allocation" claim is not measured —
   drop the misleading doc, keep the equality assertion).
5. **OPTIONAL measured-fixture merges (up to ~8–10, MEDIUM; recommend KEEP):**
   `tests/measured_fixture.rs` (29 tests) duplicates logic names with unit tests, e.g.
   `per_class_two_under_a_tight_ceiling_tags_the_second_over_budget` (714) vs
   `src/select/tests.rs:689`, `the_patch_flow_byte_equals_the_direct_new_assembly…` (1463)
   vs `src/assemble/tests.rs:441`, `a_moved_chunk_is_reused_and_counted_as_moved` (379)
   vs `src/manifest/diff.rs:1166`. But the file deliberately freezes real measured
   constants as ground truth (failures are "information, not a bug"); its added value is
   those constants, not the logic. Merge only if an explicit size target exists.
6. **Kept:** `src/http/tests.rs` (58 — every retry/ceiling/truncation case pins a
   distinct mutation; comments at 1176/1260/1325 record surviving mutants),
   `src/verify/tests.rs` (48), the `emit/index.rs` a16 family (898–1160),
   `tests/cli_contract.rs` remaining (exit-code remapping lives in `main.rs`, so
   library tests cannot observe it), and the bundle/schema/offline-bundle suites.

### F. Small crates + sled2redb — ~8–10 tests

1. `tools/sled2redb/tests/convert.rs:677` `the_help_exits_zero_and_documents_the_flags`
   is subsumed by the strict pin `the_help_output_is_pinned` (805). **1 saved, LOW.**
2. Same file — usage-error cases `a_usage_error_exits_three` (613),
   `a_missing_positional_exits_three` (653),
   `an_unknown_output_format_exits_three_and_names_the_accepted_values` (668)
   (+ optionally `a_version_request_is_refused…` 633) → one parametrized test.
   **2–3 saved, LOW–MEDIUM.**
3. `tools/sled2redb/tests/convert.rs:201` temp-name test is strictly weaker than
   `tools/sled2redb/src/convert.rs:410` (which checks retry spellings n=1..3 and
   `is_cache_rel`). **1 saved, LOW.**
4. `crates/log/src/plan.rs:428` `a_debug_build_and_a_release_build_name_the_default_the_same_way`
   restates `default_file_is_written() == !cfg!(debug_assertions)`; the next test (446)
   covers the policy through `LogPlan::resolve`. **1 saved, LOW.**
5. `crates/tempdir/src/lib.rs` — two overlapping pairs: name-shape tests 266/334 and
   root-placement tests 291/320. Fold the weaker into the stronger. **2 saved, LOW.**

## Cross-cutting / hygiene (no or minimal test-count effect)

- **Byte-identical helper duplication:** `tools/sync/tests/common/mod.rs` and
  `tools/sled2redb/tests/common/mod.rs` are **verbatim copies** (702 lines, 27,348 bytes,
  SHA-256 prefix `D3394032…`, including comments). sled2redb uses only ~7 helpers yet
  compiles the whole module under `#![allow(dead_code)]`. The existing `girpr-tempdir`
  crate is the natural home for the shared file/link/cache fixture helpers. Extracting
  them saves maintenance, **not test count**. The root (1,479 lines) and capture (1,811
  lines) `tests/common/mod.rs` are domain-specific and should stay separate.
  **Risk: LOW–MEDIUM** (test-only refactor).
- **12 `#[ignore]`d tests — recommend KEEP.** Six generated by `capture_test!` in
  `tests/artifact_bridge.rs:860-910`, three in `tools/capture/tests/bundle_shapes.rs`
  (213/371/465), two throughput benchmarks in `src/util/md5.rs:736/791`, and
  `tests/peak_heap.rs:277`. They are skipped by default (so removal would not reduce
  the running count) and are the only pre-release repair verification / cost measurements.
  *(Stale doc note: `tools/sync/DESIGN-NOTES.md:907` references an ignored test in
  `tests/sync.rs` that does not exist — documentation cleanup only.)*
- **No assertion-free tests found.** A body-window scan of all 2,262 test functions
  flagged 41 candidates; each uses `#[should_panic]`, a helper assertion
  (`assert_backoff`, `debug_assert()`), or `.expect(...)` as its verification. No reductions.
- **`tools/zz_probe.rs` / `tools/zz_probe_arm.rs`** contain `#[test]` attributes but are
  **not Cargo test targets** — `tools/probe.ps1:68` copies them into `tests/zz_probe.rs`
  only for an external cross-commit probe. Exclude from test-count metrics; leave in
  place (deleting them would break that workflow).
- **`tools/sled2redb/tests/convert.rs:515`** performs two full binary conversions after
  six in-process clap parses (the JSON-record test at 457 already runs conversions).
  Dropping the redundant conversions saves runtime, not test count. **LOW risk.**

## Recommended action plan

1. **Take the zero-coverage-loss MERGEs first** (§B.1–3, §C.1–4, §D.1–2, §E.4,
   §F.2/4/5, §A): ~50–60 tests, low risk, no coverage lost. These are case tables
   over pure functions or test-local helpers.
2. **Then the unambiguous REMOVEs:** `cli_contract.rs:2130`, `src/hash/tests.rs:578`,
   `crates/log/src/plan.rs:428`, the six duplicated fixture-helper tests in
   `src/util/{path,ini}.rs`, and the sled2redb help test — ~10 tests, low risk.
3. **Consider, with verification:** the `migration_lock.rs` trim, `purge.rs` classifier
   consolidation, `run_lock.rs` rewrite, log-file process-test tables, and the
   cross-binary log-policy reduction — ~30 tests, medium risk.
4. **Leave alone:** the `repair_*` integration invariants, `tools/capture` mutation-driven
   HTTP/verify/emit suites, `tools/sync` race/guard suites, the 12 ignored tests, and
   everything in §E.6/§D.6/§C.6. These carry distinct documented coverage.

Given the low absolute reduction (~3–4 %) and the mutation-detection value of the
existing tests, **doing nothing is a defensible outcome**. If a size target exists,
the merges in steps 1–2 are the safe way to reach it.
