# T20 — Guardrail audit: hooks intact, no temp→`Z:` redirect

Priority P2 · all crates · checkpoint CP5

## Pointers (give these to the implementing session)

* Report priority row: `docs/08-test-runtime-optimization.md:L479` (§8, P2).
* Report non-recommendations: `docs/08-test-runtime-optimization.md:L488–L500`
  (§9), especially the "do not move temp to `Z:`" item.
* Report hook description: `docs/08-test-runtime-optimization.md:L72–L101` (§3)
  and the environment preamble at `L8–L13`.
* This plan: `OPTIMIZATIONS_PLAN/tasks/T20-guardrail-audit.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "`Z:` ramdisk".

## Goal

A final verification task, not an optimization: confirm that across all the work
above the existing test-only fsync/durability hooks are still present and
effective, and that nobody committed a change that redirects the test temp
directory to the `Z:` ramdisk as a "fix". The report explicitly says the hooks
are worth keeping (~169 ms/op saving) and that moving temp to `Z:` is out of
scope — `Z:` is a measurement instrument only.

## Implementation plan

Code anchors from the report (validate before editing):

* `.config/nextest-test-env` — must still contain:
  `GIRSYNC_TEST_DURABILITY=none`, `GIRSYNC_TEST_NO_FSYNC=1`,
  `GIRPR_TEST_NO_FSYNC=1`.
* `.config/nextest.toml` — setup scripts that copy the env file; keep them.
* `.gitattributes` — pins the env file to LF (so `DURABILITY` equals `"none"`).
* Search the tree for any test-env/temp-dir configuration that points at `Z:`.

Steps:

1. Verify the three hook lines are present and unchanged in
   `.config/nextest-test-env`, that the setup scripts still export them, and
   that the LF pin is intact.
2. Verify no committed change sets `TEMP`/`TMP`/`TMPDIR` (or a crate's temp-root
   helper) to a `Z:` path as an optimization. The `Z:` ramdisk may be used during
   this work only as scratch for timing comparisons.
3. If any regression is found, revert/fix it here (this task is a fix task).

## Acceptance criteria

* All three hooks remain present and effective; the setup scripts and LF pin are
  intact.
* No test temp directory is redirected to `Z:` anywhere in the committed tree.
* The normal `cargo nextest run --workspace` gate still passes.

## Guardrails

* This is an audit/fix task — if everything is already correct, make no code
  change and record the audit result as `completed` with a note.
* Validate against current code first. Do not add new hooks or change hook
  semantics; this task only restores them if earlier tasks broke them.
* If `Z:` is needed for a timing check and is not present, pause and return to
  the user. Report follow-up ideas without implementing them.
