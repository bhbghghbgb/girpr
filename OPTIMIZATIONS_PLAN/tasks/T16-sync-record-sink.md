# T16 — Sync injectable record sink + `run_captured`

Priority P2 · `tools/sync` (girsync) / production code · checkpoint CP7 ·
report finding E1 · **highest-value item in the report**

## Pointers (give these to the implementing session)

* Report finding E0 (rationale): `docs/09-test-scale-reduction-research.md:L611–L620`.
* Report finding E1: `docs/09-test-scale-reduction-research.md:L635–L671`.
* Report process surface table: `docs/09-test-scale-reduction-research.md:L622–L633`.
* Report synthesis: `docs/09-test-scale-reduction-research.md:L790–L795`
  (pattern 1), `L818–L820` (Wave 3), `L858–L862` (headline).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T16-sync-record-sink.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Eliminate ~85 of ~100 process spawns in `tools/sync` by capturing the record
stream in-process. This is the single highest-value change in the report.

`Report` is the sole writer of stdout in the crate (one `println!` at
`src/report/mod.rs:140`); `run()` returns only `i32` and discards records
(`src/commands/mod.rs:139–199`). Process tests exist to capture that stream, the
`anyhow` error string logged by `main`, or the tracing subscriber — all reachable
in-process with contained changes.

## Implementation plan

Code anchors (validate before editing): `src/report/mod.rs:90–154`,
`src/config/common.rs:40–68` (`LogCtx`, `report()` :66),
`src/commands/mod.rs:139–199`, `src/main.rs:34–42`,
`src/commands/update.rs:143–149`, `src/report/record.rs:39,217–219`; test helpers
`run_binary`/`run_binary_raw` (`tests/sync.rs:67,102`), `sync_bin`/`sync_bin_raw`
(`tests/symlink.rs:35,53`), `bin` (`tests/why/flags.rs:13`), `sync_plan::run`
(`tests/sync_plan/main.rs:73`).

Steps (large task — multiple commits expected):

1. `Report { format, sink }` with
   `enum ReportSink { Stdout, Capture(Arc<Mutex<Vec<Record>>>) }`; `emit` pushes
   into the mutex when capturing, otherwise prints as today. The `Arc<Mutex<_>>`
   is **required** — `COPY` records are emitted from rayon workers under a
   separate `emit.lock()` (`apply/phases.rs:230–237`); a thread-local would drop
   them. `Record` is already `Clone`.
2. Add `report_sink` to `LogCtx` (default `Stdout`); change `update.rs:149` from
   `.emit(log.output)` to `log.report().emit(...)` so `update` is captured too.
   All four commands already take `&LogCtx` and call `log.report()` — no command
   signatures change.
3. Add `pub struct RunOutcome { code, records, error }` and
   `pub fn run_captured(cli, plan) -> Result<RunOutcome>`, dispatching the same
   four commands with a fresh capture sink and mirroring `main`'s Err→3 mapping
   with `format!("{:#}", e)` recorded as `error`. Parse-error tests can use
   `Cli::try_parse_from`.
4. Test side: replace the helpers with a thin wrapper
   `Cli::try_parse_from → run_captured → records.map(json)`.

## Acceptance criteria

* Captured values are the printed `Record`s (one definition, both renderings);
  exit code from the same `report_diff`/`report_and_finish`; error string the one
  `main` logs. Vocabulary, ordering, `why` tags, summaries and failure messages
  survive verbatim.
* Keep ~3 smoke process tests (`compare`, `sync`, `update`) plus the `output.rs`
  log tests as the process guard. `CARGO_BIN_EXE_girsync` shrinks to a handful of
  smoke tests.
* ~85 spawns eliminated (sync.rs 30, sync_plan 24, 7 of 18 in output.rs, symlink
  11, why/flags 11, stat_trust 4, rw_threads 2, compare/compare_self/hash_mode 3);
  test count flat or ~10 lower where a process test duplicates a library test.
* Affected suites pass.

## Guardrails

* Removing `Copy` from `Report` needs an audit of its three by-ref
  `let report = log.report()` sites. Production contention on the `Arc<Mutex>`
  is absent.
* The grouped sync suites (lazy/hash_mode/stat_trust/why/rw_threads/sync_plan)
  test genuinely different mechanisms — convert their harnesses, not their
  cases (report constraints L839–L841).
* This task does **not** include T17 (E2 logging) or T18 (E5 env hooks); those are
  separate tasks that build on it. Report follow-up ideas without implementing
  them.
