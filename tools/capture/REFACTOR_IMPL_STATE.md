# Refactor implementation state

Checklist for the findings in `REFACTOR_FINDINGS.md`. Order follows the
document's suggested sequence: low risk first, silent-failure concurrency work
last. One session at a time; each task lands as its own commit.

Legend: `[ ]` pending · `[~]` in progress · `[x]` done, awaiting review · `[!]` skipped

## 1. Safe fixes, no output change

- [x] **F1** — `percent_decode` slices by byte offset and panics on a multi-byte character after `%`
- [x] **F15** — integration tests carry their own `percent_decode`; import the library one instead
- [x] **F4** — stdout lock panics on poisoning instead of recovering like every other lock
- [ ] **F14** — `Val::from(u64)`/`From<usize>` narrow with `as i64` and a justification that does not hold
- [ ] **F12** — `Rejections::snapshot` and `document` carry the same sort closure twice
- [ ] **F10** — A17 doc comment states "Every state is acceptable" twice
- [ ] **F11** — duplicated doc line in `manifest/dump.rs` test

## 2. Local dedup, no output change

- [ ] **F2** — `http::fetcher::attempt` carries two byte-identical `fail` closures
- [ ] **F5** — `run::verify` and `run::materialize` duplicate the verdict/exit-code tail
- [ ] **F6** — four `BlobProps` constructors repeat a 14-field literal

## 3. `cli` surface

- [ ] **F8** — `--output` is parsed after clap instead of by clap
- [ ] **F9** — non-Unicode flag table in `cli` tests is hand-maintained and can silently miss a new flag

## 4. `select` restructure

- [ ] **F7** — `notes`/`coverage` are parallel vectors zipped in phase 3; four `MatrixRow` literals repeat the same `None`s

## 5. Concurrency (land alone, verify with a bundle diff)

- [ ] **F3** — `Store::fill` re-implements `Fetcher::map_concurrently`'s ordered-join loop

## 6. Clone removal

- [ ] **F13** — avoidable clones of `Want`, of the whole request log, and of the file list

## Pending review

- F1 — `percent_decode` byte-oriented, regression test added — commit `d61798c`
- F15 — integration tests use the library `percent_decode` — commit `38853ed`
- F4 — stdout lock recovers from poisoning like the other locks — commit `ffdeba5`
