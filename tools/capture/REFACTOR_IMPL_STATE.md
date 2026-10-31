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

- [ ] **F8** — `--output` is parsed after clap instead of by clap
- [ ] **F9** — non-Unicode flag table in `cli` tests is hand-maintained and can silently miss a new flag

## 4. `select` restructure

- [ ] **F7** — `notes`/`coverage` are parallel vectors zipped in phase 3; four `MatrixRow` literals repeat the same `None`s

## 5. Concurrency (land alone, verify with a bundle diff)

- [ ] **F3** — `Store::fill` re-implements `Fetcher::map_concurrently`'s ordered-join loop

## 6. Clone removal

- [ ] **F13** — avoidable clones of `Want`, of the whole request log, and of the file list

## Pending review

None. F1, F15, F4, F14, F12, F10, F11, F2, F5 and F6 were reviewed and
accepted: each diff matches its finding, the field values and recorded
verdicts are unchanged, and `cargo nextest run -p gircap` (756 passed),
`cargo clippy -p gircap --all-targets -- -D warnings`, `cargo fmt --check`
and the rustdoc build are clean.

Non-blocking suggestions from that review, none of which holds up a task:

- **F14** — the clamp has no direct test. A `Val::from(u64::MAX) ==
  Val::Int(i64::MAX)` assertion would pin it as cheaply as the F1 regression
  test pins the decoder.
- **F15** — the deleted helper carried the one-line rationale "The summary
  percent-encodes whitespace and `%`, so decode it back"; keep that context as
  a comment at the import or first call site if a reader would otherwise have
  to find `percent_encode` to understand why.
- **F5** — `Verdict` holds an owned `Option<String>` and clones it into the
  summary. Fine for one headline per run; an `Option<&str>` borrowed from the
  report would remove the copy if this ever sits on a hotter path.
