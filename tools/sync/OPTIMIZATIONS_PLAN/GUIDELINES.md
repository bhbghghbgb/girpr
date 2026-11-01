# Guidelines for implementing and reviewing sessions

These rules apply to every session the orchestrator spawns. The orchestrator
passes this file (or the relevant excerpt) with the task file.

## Scope

- Implement **only** the task you were given. Do not pick up the next task, do
  not edit `STATE.md`, and do not start a follow-up.
- Read the task file and only the report line ranges the task names. The report
  is a survey, not a second plan; do not implement adjacent entries because
  they look related.
- If the task is blocked, stop and report exactly what is missing. Do not
  invent a behaviour change to get unstuck.
- Do not delete or overwrite unfamiliar work. Check `git status` and
  `git log` before editing.

## Invariants

`tools/sync/DESIGN-NOTES.md` is the contract. The following are load-bearing
and must survive every task:

- Two independent reads in the copy verify, and a propagated `sync_all`.
- The `src → dst` lock order, the `RwLimiter`, and the three-gate shape.
- The pre-drop of cache rows before filesystem changes.
- The plan-before-rename order, and the planner's `Lowercase` pairing staying
  different from the diff's `Exact` pairing.
- The open cache algorithm vocabulary (`DigestSet::foreign`) versus the closed
  request vocabulary (`HashAlg`).
- `plan_pairs`' coverage check and `pair_verdict`'s `unverifiable` clause.
- The `--hash-any-of` tie-break following flag order.
- The diff's always-on classification.

If a task appears to require breaking one of these, stop and report it rather
than working around it.

## Commits

- One commit per logical step, as the task's **Commit plan** says. A task may
  have more than one commit; do not squash unless the task says so.
- Commit messages are self-contained and use the crate's existing style, for
  example `fix(sync): look digests up by name`, `perf(sync): …`,
  `refactor(sync): …`, `docs(sync): …`, `test(sync): …`.
- **Never mention** `OPTIMIZATIONS_REPORT.md`, `OPTIMIZATIONS_PLAN`, a task id,
  a review batch, or the orchestrator in a commit message, code comment, doc,
  test name, or assertion message. The plan is deleted; the code must stand
  alone.
- Do not amend, rebase, force-push, or rewrite commits that already exist.
  Fixes are new commits. A deliberate rollback is `git revert`, not a reset,
  unless the orchestrator explicitly says the history is private and a reset
  is allowed.

## Verification

Run at least:

```text
cargo fmt --all -- --check
cargo test -p girsync
```

For a task that changes the converter, also run the feature-gated command the
task names. For a performance task, the benchmark harness is not a gate; do
not add timing assertions to CI.

If the full suite is too slow on this host, run the directly affected test
targets first, then the full suite once before declaring the task done. State
in the final report which commands you actually ran.

## Behaviour changes

A task that changes observable behaviour must update `README.md` in the same
task, unless the task explicitly hands documentation to T16. The report and
the design notes are not user documentation; `README.md` is.

## Reporting back

The implementing session returns:

- the task id;
- the commit hashes it created (or `none`);
- the commands it ran and their result;
- any deviation from the task file, with the reason;
- anything it noticed but deliberately did not change (candidate follow-ups).

Do not write a long narrative. The orchestrator records only the hashes,
status, and follow-up candidates.

## Reviewer rules

- Review the batch against the task files and the report sections named there.
  Do not review the whole crate.
- Check the invariants above first, then acceptance criteria, then tests.
- You may **fix** a bad task or **revert** it. Fixes are new commits. Reverts
  preserve history; if a later task in the batch depends on the reverted one,
  revert or fix the dependent commits too, and say so in the result.
- Do **not** implement non-blocking follow-ups. List them; the orchestrator
  records them in `STATE.md`.
- Do not edit `STATE.md`. Return a structured result the orchestrator can
  apply.
- Do not add references to the report or plan, even in a review comment
  committed to the tree.
