# Refactor implementation state

Checklist for the findings in `REFACTOR_FINDINGS.md`. Order follows the
document's suggested sequence: low risk first, silent-failure concurrency work
last. One session at a time; each task lands as its own commit.

Legend: `[ ]` pending · `[~]` in progress · `[x]` done, reviewed and accepted · `[!]` skipped

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

- [x] **F13** — avoidable clones of `Want`, of the whole request log, and of the file list

## Pending review

_None._ F8, F9, F7, F3 and F13 were reviewed and accepted.

**F8** (`ec343e9`) — accepted. The flag is a clap `ValueEnum`, `resolve_common`
is infallible, and `every_argument_error_is_one` still pins exit `1`. The
failure moved to clap's `InvalidValue`; `cli::tests` asserts the kind and both
possible values. One consequence the finding did not name: `--output yaml` used
to take the old `resolve_common` arm and print a stdout `SUMMARY`; it now
prints none, because it is an argument-level failure like every other bad flag.
That is the uniform rule, and `a_usage_error_prints_a_summary_on_stdout_too`
still covers the plan-level usage errors that do print one.

**F9** (`783c6f7`) — accepted, and the gate was mutated to prove it: adding a
`--zzz-probe` value flag to `list` fails
`every_value_flag_has_a_non_unicode_case` with `list --zzz-probe has no
non-Unicode case`. The row table still hand-lists values (the test needs a
concrete non-Unicode value per flag); coverage is now derived from
`Cli::command()`.

**F7** (`a8cfd2d`) — accepted. `by_class` is one walk of the pool and `taken`
applies the same exclusion the old `find` predicate did (`admitted` plus every
earlier pick, in pool order); `pending` makes "one row per class" structural
instead of two index-aligned vectors, and `MatrixRow::new` removes the repeated
`None` fields. `select/tests.rs` pins the matrix row-for-row and is unchanged.

**F3** (`84322da`) — accepted after this review fixed one blocking defect. The
shared loop is behaviour-preserving: `map_concurrently`'s panic text and slot
are unchanged (`a_panicking_request_task_keeps_its_index_and_cause`), and
`fill` now records its errors in `wants` order (nothing consumed the old
completion order; `run` reads only the length) while a panicking chunk task
gains its payload in both the error and the index row. The blocker was
documentation, not behaviour: the new doc on the public
`Fetcher::map_concurrently` linked the private `crate::tasks::join_ordered`, so
the rustdoc build with `-D warnings` was red. Fixed here by naming the loop in plain code
(`http/fetcher.rs:449`); rustdoc is green.

**F13** (`80b4837`) — accepted. `Vec<Want> -> Arc<[Want]>` is one move, so
`fill` stops cloning a whole `Want` per task; `attach_seq` gets the same "last
matching row" it read from `records().rev()`, from a filtered read instead of a
full clone; and `write_manifest_files` returns one list that `MANIFEST.json` and
the checksums seed both borrow. No recorded value moves.

## Follow-ups from review (non-blocking)

- [x] **R1** — `Val::from(u64::MAX) == Val::Int(i64::MAX)`, pinning the F14 clamp as the F1 test pins the decoder — commit `95df907` — validated: with the conversion put back to `as i64` the test fails at `src/report/tests.rs:326`; it passes on HEAD.
- [x] **R2** — keep the "the summary percent-encodes whitespace and `%`" rationale as a comment at the first `percent_decode` call site in the integration tests — commit `a30b4b7` — validated: both `Run::summary` parsers carry it, and the mock server's independent decoder in `tests/common` is untouched.
- [ ] **R3** — pin `RequestLog::records_matching`'s `seq` order. Deleting its `sort_by_key` passes every test today: the `attach_seq` fixtures are pushed in `seq` order, so filter-then-`pop` cannot see a missing sort. Push two records out of order (as `http::tests` already does for `records()`) and assert the matches come back ascending. (F13)
- [ ] **R4** — `Fetcher::map_concurrently` now clones each item twice: once when `&[T]` becomes `Arc<[T]>`, and once more for `f` inside each task. `T: Clone` is already required, so `items.to_vec().into()` would halve it (or the loop could hand `f` a reference). Inputs are ≤ seven metadata requests today, so this is not blocking; it is the one place where F13's clone-removal commit adds a clone. (F13)
- [ ] **R5** — `Store::fill`'s `JoinOutcome::Lost` arm is untested. `join_ordered` proves the loop maps a panic to its slot, but the `fill` mapping — the generic sentence on `why: None`, the payload-bearing `FetchError` and `{chunk id}: ...` detail on `why: Some` — is only exercised by reading. Extract the outcome-to-entry mapping so a synthetic `Lost` can drive both arms. (F3)
- **Not actioned:** F5's `Verdict` holds an owned `Option<String>`; one headline per run, not a hot path, so borrowing it buys nothing today.
- **Not actioned:** `OutputFormat::FromStr` has no production caller now that clap owns `--output`; its only use is its own test, and its error text is a second spelling of the same possible values. Keep it as public API or delete it with the test. (F8)
- **Not actioned:** F9's coverage set is keyed by flag *name*, not `(subcommand, flag)`. That is correct while the repeated flags are the `global` ones; two subcommands defining the same long name with different types would under-cover again. (F9)
- **History:** `ec343e9`, `783c6f7`, `a8cfd2d`, `84322da` and `80b4837` are all ancestors of `67de40c`, in that order, and each touches only its own module plus the tests that pin it. Gates at `67de40c` after a forced rebuild: `cargo nextest run -p gircap` 760 passed / 3 skipped, `cargo clippy -p gircap --all-targets -- -D warnings`, `cargo fmt --check` and `cargo doc -p gircap --no-deps` with `RUSTDOCFLAGS=-D warnings` all clean. The only non-state change in this review commit is the F3 doc link.
- **Cargo freshness caveat:** the first run on this checkout reported `report::tests::a_count_too_large_for_i64_clamps_instead_of_wrapping_negative` failing with `Int(-1)` against a source tree that already clamps — the test binary predated the source and cargo's mtime fingerprint called it fresh. Touching `report/mod.rs` recompiled it and the suite went 760/760. Touch the file (or `cargo clean -p gircap`) before trusting a gate result on this checkout.
