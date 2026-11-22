# Review checkpoints

A review-and-fix session reviews one batch at a time. Batches are sized by **amount of
work**, so a reviewer is never overwhelmed; a batch that turns out larger than expected
gets split or its own review. These groupings are a starting proposal — the orchestrator
adjusts them based on the actual state and may call a review at any time, especially to
prune follow-ups that are running away.

| Checkpoint | Tasks | Why grouped (work size / risk) |
|---|---|---|
| A | T01, T02 | Two foundational chunk leaves (§F4+§F8, §F5); establish the new-leaf doc-seam pattern first, verify digest normalization stays at the decode boundary |
| B | T03, T04, T05 | Three self-contained pure chunk folds/leaves (§F6, §F7, §F9); check published PLAN/SUMMARY figures and the `usable_len` asymmetry are preserved |
| C | T06 | API DTO revisit (§F10) — wire-preserving decoders and byte-exact capture bodies; one focused review |
| D | T07, T08 | Cycle-sensitive: lift Mode/resolve first (§F15), then chunk-verify (§F11, §F16) — verify the write-order correctness proof survives |
| E | T09, T10 | repair-plan value lifting (§F12, §F16) and path-scope semantics (§F13); both touch large cyclic files |
| F | T11, T12, T13 | Byte accounting (§F14) plus two low-confidence value crates (§F17, §F18); check no shared format/emitter was created |
| G | T14, T15, T16 | gircap path-shape (§F19) + A4-forbidden refusal-ledger assessment (§F20) + URL builder (§F21) |
| H | T17, T18, T19 | gircap document envelope (§F22), fs-names predicate (§F23), value ranges (§F24) — check error/exit strings and key sets |
| I | T20, T21, T22 | Verify walker (§F25), high-risk HTTP transport assessment (§F26), retry decision vocabulary (§F27); check independent re-derivation and jitter keepers |
| J | T23, T24, T25 | Two likely-skip assessments (§F28 engine-cache, §F29 engine trait) plus `path-identity` leaf (§F30); verify no forbidden cache/engine was introduced |
| K | T26, T27, T28 | Evidence policy (§F31), sync-test-support split (§F32), generic atomic publish (§F33) — verify env pinning, nextest isolation and one-syscall gaps |
| L | T29, T30, T31 | Report-plane generalization (§F34), fault seam (§F35), plan-action record (§F36) — check NDJSON bytes and pinned env spellings |
| M | T32, T33, T34 | KV backend (§F37), batched writer (§F38), restartable sink (§F39) — separate large sealed-handle/seam changes |
| N | T35, T36, T37 | Capture buffer (§F40), command-harness assessment (§F41), skip policy (§F42); check strict-vs-lossy UTF-8 and skip wording |
| O | T38, T39, T40 | Fixture support (§F43), test-support facade assessment (§F44), time/stamp leaf (§F45) — check rebuild domains and on-disk byte strings |
| P | T41, T42, T43 | Line sink (§F46), IO read buffer (§F47), primitives-consolidation assessment (§F48); check no serde leak into log and caller-owned buffer seam |
| Q | T44, T45, T46 | Loopback narrow extension (§F49), report-model leaf (§F50), CLI narrow extension (§F51) — check pinned flag help/exit codes |
| R | T47, T48 | Likely-skip `bounded_decode` assessment (§F52) and `girpr-fs` composition leaf (§F53); final, smaller batch |

## Rules

- Review the whole `pending_review` set at a checkpoint in **one** review session,
  passing the relevant report/task sections and the commit refs from `STATE.md`.
- The reviewer may fix or revert any task in the batch, set its final state, and record
  follow-ups; it does not implement follow-ups. It checks especially that no published
  byte/key order/flag condition drifted (§A5), gircap stayed independent and doc tables
  moved with any new leaf (§A4, §A14), no divergence was collapsed (§A9), no keeper from
  `DO-NOT-TOUCH.md` was pulled in, and no re-exporting facade was left behind (§A2).
- A validated `skipped` outcome for a low-confidence / forbidden candidate is correct and
  is not a failed review; do not pressure such a task into an implementation.
- After every task is settled, run a **follow-up planning review** first to decide how to
  batch follow-ups and which to drop, then alternate implement and review sessions over
  those batches.
- If follow-up count keeps growing, the orchestrator may call an extra review session
  purely to reduce/prune the list. The orchestrator itself does not research the code.
