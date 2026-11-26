# `REFACTOR_PLAN` — third-party delegation plan

This is the planning package for the audit recorded in
`docs/08-third-party-delegation-findings.md` (referred to everywhere below as
**the report**).

The report is a read-only classification of one tree. It contains assumptions,
findings and explicit non-findings — no task list, sequencing or implementation
steps (lines 3–9). This package adds only the implementation plans and the rules
for carrying them out. Every task points back to report sections instead of
copying them; the implementing session validates every premise against the
current code.

The goal of this round is to replace hand-rolled mechanics with maintained
libraries wherever a library genuinely owns the mechanism — HTTP servers, URL
and percent-encoding, CLI/process/log output parsing, bit/hash/map helpers,
protobuf and encoding primitives, path and time conversion, benchmark harnesses
and test fixtures — while keeping each program's policy, error vocabulary and
core data planes local. Findings are grouped into tasks by area, library and
risk; a task may need several commits.

> This directory and the report are **deleted when the work is finished**.
> Implementation code, tests, comments, manifests, doc tables and commit
> messages must therefore never refer to `REFACTOR_PLAN/` or to
> `docs/08-third-party-delegation-findings.md`. Every plan file points outward
> to the report only so an implementing session can find the goal; no shipped
> artifact depends on those files.

## Layout

```text
REFACTOR_PLAN/
  README.md                    this file — index, structure, how to read a task
  ORCHESTRATION.md             the orchestration plan (state machine, checkpoints)
  STATE.md                     the committed check-list state (single source of truth)
  tasks/
    T01-rayon-purge.md                     R1: purge parallel map
    T02-futures-repair-combinators.md      R5, R7: settle + ordered reorder buffer
    T03-petgraph-salvage-order.md          R6: salvage-order heuristic (high risk)
    T04-axum-test-origin.md                R2: root test HTTP origin
    T05-trycmd-cli-help.md                 R3, LEAF-2: CLI help parsing + ANSI stripper
    T06-assert-cmd-processes.md            R4: process-spawn/output tuples
    T07-structured-log-capture.md          R8, LEAF-10: JSON log capture + girpr-log docs
    T08-rust-ini.md                        APP-1: INI parser/merger/renderer
    T09-url-delegation.md                  X2: URL assembly + percent-encoding across products
    T10-small-app-wire-helpers.md          APP-3/4/9/10: serde-aux, quick-xml, base64, hex
    T11-fnv-hashing.md                     APP-7, CAP-5: FNV-1a hashing
    T12-prost-protobuf.md                  APP-8, LEAF-4: prost decoding + prost-build codegen
    T13-junction-crate.md                  APP-6, LEAF-3: junction creation (Windows)
    T14-wiremock-artifact-server.md        APP-11: test-fixtures artifact server
    T15-wiremock-scripted-server.md        LEAF-1: girpr-loopback-mock ScriptedServer
    T16-strum-enum-tables.md               LEAF-7, SYNC-2: enum↔string tables
    T17-async-trait-sleep.md               LEAF-6: boxed-future sleep hook
    T18-tempfile-scratch.md                LEAF-9, SYNC-4: scratch/case-tmp paths
    T19-relative-path-shape.md             LEAF-8: lexical path normalization
    T20-globset-filter.md                  LEAF-5: glob matching + ancestor walker
    T21-bitflags-algo-set.md               SYNC-1: AlgoSet bitfield
    T22-filetime-mtime.md                  SYNC-3: mtime conversion/stamping
    T23-path-clean-link-targets.md         SYNC-5: link-target normalization
    T24-cache-json-streaming.md            SYNC-6/7: NDJSON stream + direct hex
    T25-criterion-benchmark.md             SYNC-8: benchmark harness
    T26-ignore-walker.md                   SYNC-9: walk + include/exclude (low confidence)
    T27-axum-capture-server.md             CAP-1: capture test HTTP mock server
    T28-ordered-json-fixtures.md           CAP-2: ordered-JSON fixture value
    T29-futures-ordered-join.md            CAP-6: bounded ordered join
    T30-csv-tsv-writers.md                 CAP-7: two TSV writers
    T31-capture-misc.md                    CAP-8/9/10/11: markdown, semver, slug, differ
    T32-unicode-bom.md                     APP-12: duplicated BOM stripping (optional)
  guidance/
    CONVENTIONS.md               commit/state conventions, pointer rules, skip/follow-up rules
    DO-NOT-TOUCH.md              non-findings and pinned contracts; what is not a task
    REVIEW-CHECKPOINTS.md        how tasks are grouped into review batches
```

## How to read a task file

Every `tasks/Txx-*.md` has the same shape:

- **Goal / pointers** — the exact report sections (§ and line numbers) that are
  the detailed brief, plus the assumptions to validate first and any keep items.
  The report supplies the *goal*; the task file supplies only the *plan*.
- **Scope** — the files and crates named by the report. These are *hints*; the
  implementing session locates them in the current tree before editing.
- **Implementation plan** — a short outline of the intended change. It is not
  exhaustive and not carved in stone; the implementing session does the detailed
  research and validation. A task may be delivered in more than one commit.
- **Constraints / validation** — the caveats that must survive (policies, error
  vocabularies, deterministic outputs, test-only seams, shared leaves).
- **Done when** — the acceptance check, including what to do when validation
  fails.

The implementing session is expected to validate every premise against the
current code. Report items are static claims taken on one tree (§A1, lines
15–16; §A2, 17–18); line numbers and versions may have drifted. If a task is
stale, contradicted by the code, would change a keeper in `DO-NOT-TOUCH.md`,
would delete a test of intended behavior (§A5, lines 26–28), would enable a
feature with cross-cutting effects (§A7, lines 33–34), or cannot be bounded
safely, the session **stops** and the task is recorded as `skipped` with a
one-line reason. If a better delegation presents itself, the session may
improvise as long as the finding's goal — library owns the mechanics, policy
stays local — is met.

Recurring constraints drawn from the report that apply to nearly every task:

- **A1 (lines 15–16):** nothing was compiled or implemented in the report; call
  sites and line numbers must be re-validated against the current tree.
- **A4 (lines 22–23):** observable output, byte spelling, error text and golden
  snapshots may change; backwards compatibility is not required.
- **A5 (lines 26–28):** tests that pin replaced behavior may be removed, and
  tests of a hand-rolled implementation may go, but tests of the intended
  function must be adapted, not deleted. Regenerate golden files; keep the
  semantic assertions.
- **A7 (lines 33–34):** never enable `serde_json/preserve_order`; where order
  matters, use a local ordered representation.
- **A8 (lines 35–36):** project-local `girpr-*` leaves hold policy, not
  boilerplate; they are generally not candidates. See `DO-NOT-TOUCH.md`.
- **A12 (lines 48–50):** tests must assert the library's documented guarantees,
  not its internal formatting quirks.

## Task order

Task IDs define the recommended pick-up order used by the orchestrator (T01
first): root package, then `app/*`, then `crates/*`, then `tools/sync*`, then
`tools/capture*`. Within an area, small isolated changes come before large
test-server rewrites and low-confidence changes. This is a recommendation, not
a hard rule; `STATE.md` is authoritative for what remains.

The cross-cutting themes X1–X4 (report lines 56–134) are **not** standalone
tasks; they are distributed across the tasks above:

- **X1 (HTTP servers):** T04, T14, T15, T27.
- **X2 (URL/percent-encoding):** T09 — one session makes the `game_ids[]`
  spelling decision once for all products and records it in `STATE.md`.
- **X3 (parsing other tools' output):** T05, T07.
- **X4 (bit/hash/map mechanics):** T11, T16, T21.

## Lifecycle at a glance

1. Orchestrator reads `STATE.md`, picks the next `todo` task.
2. A single implementation session handles that one task and receives pointers
   (report sections + its task file), not a recitation of the plan.
3. In a **separate commit**, the orchestrator moves the task to `pending_review`.
4. When the accumulated `pending_review` set reaches a checkpoint sized by
   amount of work, one review-and-fix session reviews the batch, fixes or
   reverts, and the orchestrator updates `STATE.md`
   (`completed` / `reverted` / `skipped`) and records non-blocking follow-ups.
5. After all tasks are settled, a follow-up planning review first decides how to
   batch survivors and which to drop, then alternate implement and review
   sessions over those batches.
6. Final cleanup deletes this directory and the report; no reference may survive.

See `ORCHESTRATION.md` for the full state machine, `CONVENTIONS.md` for the exact
commit and state-update format, and `DO-NOT-TOUCH.md` before editing anything.
