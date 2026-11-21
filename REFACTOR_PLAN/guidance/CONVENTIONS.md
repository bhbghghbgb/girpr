# Conventions

## Pointers, not recitations

- To aim a session at a goal, pass **file paths + line ranges / section numbers**
  (e.g. `docs/09-crate-split-turnaround-research.md §F3 lines 140–152` and a
  `REFACTOR_PLAN/tasks/T01-*.md` path). Do not paste the report or a task file
  into the session prompt as prose; the session reads the files itself.
- Line numbers come from the report as committed alongside this package. A
  session that finds a stale pointer should trust the code and say so.
- Report sections: §A assumptions (A1–A14), §F findings (F1–F41), including the
  exclusions subsection (F40). A task file names the exact sections and lines
  it needs.

## Commits

- Implementation work for one task is committed by the implementing session.
- Every `STATE.md` change is its **own commit**, separate from implementation
  commits (this includes `todo → in_progress`, `→ pending_review`, review
  results and follow-up bookkeeping).
- Commit messages must not reference `REFACTOR_PLAN/`, the report file, session
  identifiers, tool names, dates, or machine-specific data. Describe the code
  change itself.

## New crates and crate boundaries

- Convention from §A4 (lines 40–46): shared code lives in `crates/`, is named
  `girpr-<policy>`, carries `[lib] name/path`, a one-line
  `description = "The shared …"`, `edition = "2024"`, `version = "0.1.0"`, the
  stable-toolchain comment, and `[lints] workspace = true`. Root lint policy:
  `unsafe_code = "deny"`, `clippy::all` / `unwrap_used` / `expect_used = "warn"`.
- **Payoff test (§A2, lines 27–33):** before accepting any split, confirm that
  at least one high-level consumer (a binary or integration test) can depend on
  the new crate directly and that the old facade is not still an ancestor of
  every target. A crate that is only re-exported by the facade is not a win.
- **gircap boundary (§A5, lines 48–53):** no dependency on the `girpr` crate, no
  invoking its binary, and only leaf crates holding no program logic. Adding a
  new gircap leaf requires updating the table at
  `tools/capture/docs/00-spec.md:42` and the matching manifest comment at
  `tools/capture/Cargo.toml:1-44` in the same work, plus the other doc seams in
  §F39: `README.md:44-45`, `README.md:390-422`, `Cargo.toml:17` (workspace
  member list). Do not leave a doc gap.
- **nextest filters (§A7, lines 63–70; §F31, lines 377–383):** a crate split or
  test relocation changes the binary name and/or fully-qualified test path
  pinned in `.config/nextest.toml`. Update those filters in the same work.
- **Test helpers (§A7):** shared test helpers must be a public library surface
  reached through thin per-consumer shims (one process per test, no doctests).
  A helper that makes one program's test depend on another's is forbidden
  (§A5, §A11).
- A crate's tests must not make one program's test depend on another's. Keep
  test-only inversions visible (§F3, §A11: `girpr-tempdir`'s non-dev deps on
  girsync and serde_json for `test_support`).

## `STATE.md` fields

- **Tasks**: `id`, `title`, `state`, `commit ref(s)`, `note`.
  States: `todo` → `in_progress` → `pending_review` → `completed` / `reverted` /
  `skipped`.
- `commit ref(s)` is only the pointer handed to a review session so it knows
  what to look at. It is not a chronological change log. Do not add dates,
  session/tool IDs, machine data, or a running narrative.
- `note` carries the one-line reason for `skipped`/`reverted`, or a decision
  that later tasks need (e.g. "split did not shrink the rebuild set, §A2").
  Keep it short and point to report sections where useful.
- **Reviews**: `id`, task IDs in the batch, `state` (`pending`/`done`), note.
- **Follow-ups**: `id`, description, `state` (`open`/`batched`/`done`/`dropped`),
  note. Follow-ups are non-blocking suggestions; the reviewer records them and
  never implements them.
- **Persistent context**: any validation result or decision that affects later
  tasks (e.g. "§A2 failed for X — facade still re-exports both halves"). This is
  the only place to stash knowledge gained during orchestration. No dates, no
  session/tool IDs, no change history.

## Skip / improvise / follow-up

- A stopped task becomes `skipped` with the reason. Skipped tasks do not go to
  review. They may be reopened later as follow-ups if a reviewer says so.
- An implementing session may deviate from the written plan when it finds a
  better optimization; record the deviation in the task's `note`. The goal is
  faster turnaround, not literal compliance with a static plan.
- Only a review session creates or removes follow-ups, and only the orchestrator
  writes them into `STATE.md`. Reviewers may prune follow-ups to avoid run-away.
- A task whose split does not actually remove a rebuild edge (§A2) is skipped,
  not reported as a cosmetic success.

## No lingering references

Before the final cleanup, confirm no source, test, comment, manifest, doc table
or commit message in version control points at this directory or at
`docs/09-crate-split-turnaround-research.md`. Then delete `REFACTOR_PLAN/` and
that report file.
