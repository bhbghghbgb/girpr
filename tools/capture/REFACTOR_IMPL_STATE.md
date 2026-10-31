# Refactor implementation state

Checklist for the findings in `REFACTOR_FINDINGS.md`. Order follows the
document's suggested sequence: low risk first, silent-failure concurrency work
last. One session at a time; each task lands as its own commit.

Legend: `[ ]` pending · `[~]` in progress · `[x]` done, awaiting review · `[!]` skipped

## 1. Safe fixes, no output change

- [x] **F1** — `percent_decode` slices by byte offset and panics on a multi-byte character after `%`
- [x] **F15** — integration tests carry their own `percent_decode`; import the library one instead
- [x] **F4** — stdout lock panics on poisoning instead of recovering like every other lock
- [x] **F14** — `Val::from(u64)`/`From<usize>` narrow with `as i64` and a justification that does not hold
- [x] **F12** — `Rejections::snapshot` and `document` carry the same sort closure twice
- [x] **F10** — A17 doc comment states "Every state is acceptable" twice
- [x] **F11** — duplicated doc line in `manifest/dump.rs` test

## 2. Local dedup, no output change

- [x] **F2** — `http::fetcher::attempt` carries two byte-identical `fail` closures
- [x] **F5** — `run::verify` and `run::materialize` duplicate the verdict/exit-code tail
- [x] **F6** — four `BlobProps` constructors repeat a 14-field literal

## 3. `cli` surface

- [x] **F8** — `--output` is parsed after clap instead of by clap
- [x] **F9** — non-Unicode flag table in `cli` tests is hand-maintained and can silently miss a new flag

## 4. `select` restructure

- [x] **F7** — `notes`/`coverage` are parallel vectors zipped in phase 3; four `MatrixRow` literals repeat the same `None`s

## 5. Concurrency (land alone, verify with a bundle diff)

- [x] **F3** — `Store::fill` re-implements `Fetcher::map_concurrently`'s ordered-join loop

## 6. Clone removal

- [ ] **F13** — avoidable clones of `Want`, of the whole request log, and of the file list

## Pending review

- F8 — `--output` is a clap `ValueEnum`; `resolve_common` infallible — commit `ec343e9`
- F9 — every value flag must have a non-Unicode case — commit `783c6f7`
- F7 — one `PendingRow` per class, `MatrixRow::new`, per-class candidate lists — commit `a8cfd2d`
- F3 — one `join_ordered` in `tasks.rs`; bundle diff 72/72 identical — commit `84322da`

## Follow-ups from review (non-blocking)

- [ ] **R1** — `Val::from(u64::MAX) == Val::Int(i64::MAX)`, pinning the F14 clamp as the F1 test pins the decoder
- [ ] **R2** — keep the "the summary percent-encodes whitespace and `%`" rationale as a comment at the first `percent_decode` call site in the integration tests
- **Not actioned:** F5's `Verdict` holds an owned `Option<String>`; one headline per run, not a hot path, so borrowing it buys nothing today.
