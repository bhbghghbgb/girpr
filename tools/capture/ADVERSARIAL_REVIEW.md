# ADVERSARIAL REVIEW — `gircap` (`tools/capture`)

**Baseline:** HEAD `64dc69d` (`revert: drop the adversarial review series`), tree `66985753edf618610a3df629bacdf2b6b70609fa`, identical to `7f2d69c` (`git diff --stat 7f2d69 HEAD` is empty); reviewed 2026-10-27 against HEAD, not the reverted `f9352f7` series.
**Scope:** `tools/capture/src/**`, `tools/capture/tests/**`, `tools/capture/Cargo.toml`, `tools/capture/docs/00-spec.md`, `docs/01-artifact-schema.md`, `docs/03-design-notes.md`, `docs/04-bundle-differ.md`; source-verified by `git show`, `git grep`/`rg` and file reads — no build, test, binary or network run.
**History:** the uncompacted first-pass audit (F1–F12 essays, corrections to the reverted series, landing order) is in git: `git show 65eaa38:tools/capture/ADVERSARIAL_REVIEW.md`.
**Open items:** [OPEN_FOR_MAINTAINERS.md](OPEN_FOR_MAINTAINERS.md).

## Pass log

> Detailed evidence, failure scenarios and per-pass verification are in the git
> history: `git log --oneline main..HEAD`, `git show <commit>`. The commit hashes
> in the tables are the index. No item in any pass was rejected as invalid or out
> of scope.

### First pass — F1–F12 (all fixed)

All twelve findings were fixed on `fix-gaps/gircap`; the commits are in the table.

| ID | Finding | Status | Commit |
|---|---|---|---|
| F1 | Forced `--files` admissions bypass the cumulative budget. | fixed | `39304be`, `abe7bdc`, `998649c` |
| F2 | A budget-refused forced path `break`s and silently drops later paths. | fixed | `39304be` |
| F3 | A forced path that answers no matrix class is silently dropped. | fixed | `83eac6d`, `d8514c7` |
| F4 | A failed chunk fetch loses its index, is double-counted and hides its cause. | fixed | `f31850b`, `583aa79` |
| F5 | Non-2xx bodies are drained without the response ceiling. | fixed | `71a1321`, `0259220` |
| F6 | Tag validation accepts Windows separators and path prefixes. | fixed | `efc5322`, `de85df2` |
| F7 | `--no-chunk-blobs` still enforces the chunk-byte ceiling. | fixed | `801d91d` |
| F8 | Non-Unicode `--out`/`--log-file` panics in `main`. | fixed | `df366ee` |
| F9 | Duplicate `--files` entries produce duplicate ids and rows. | fixed | `39304be` |
| F10 | CLI help contradicts the code and the spec. | fixed | `4dd1c05` |
| F11 | A failed fetch can orphan `.tmp` files; the comment overstates cleanup. | fixed | `882a53b`, `7c64557` |
| F12 | The closeout skip-set test checks only 8 of 11 ids. | fixed | `4dd1c05`, `d538d4d` |

**Verified correct (do not re-fix):**

- `too_big` is a `Shape`, not a retried `Transport`: the declared-length and running-total checks return `too_big` (`src/http/fetcher.rs:129-152`), which builds `ApiError::shape` (`:168-182`), and retry treats `Shape` as non-retryable (`:603-609`); F5 was only the error-body exception.
- `--per-class 0` is floored, not rejected: `.max(1)` at `src/config/mod.rs:336`, re-applied at `src/select/decide.rs:54`; tests at `config/mod.rs:686-696` and `decide.rs:221`.
- `--require-class` defaults to nothing: `require::check` returns `Ok(())` for an empty list (`src/select/require.rs:84-86`), matching `docs/00-spec.md:730-739`; F10 was only the help-text drift.
- A9 is byte-compared internally, digest-recorded in the artifact, and intentionally uncovered: `assemble/mod.rs:856-881` compares `patch_bytes == new.bytes`, the A9 row (`:395-401`) compares `p.md5` and `n.md5`, and the comment at `:872-875` records that replacing the comparison with `true` passes the suite (mutation-tested twice).
- The crate's dependency independence holds: `Cargo.toml:24-58` has only `girpr-log` and `girpr-tempdir` path dependencies; `std::process::Command` appears in tests (e.g. `tests/case_coverage.rs:62`, `:102`, `:145`) but not in `src/`, and none of those invocations is `girpr`.
- The manifest-id prefix is recorded, never asserted: every construction site sets `id_prefix_verified: false` (e.g. `src/manifest/mod.rs:323`, `:435`) and `Verified::all_green` ignores it (`:94-98`), per spec §3.7.
- A missing `--files` path is recorded, not fatal: `forced_skipped` at `src/select/mod.rs:499-505`; `tests/matrix.rs:695-707` asserts requested 1 / skipped 1; F2 and F3 were the other "requested but invisible" branches.
- Phase 2 want admission is smallest-first and charges only admitted costs: `budget::admit_in_order` (`src/select/budget.rs:114-128`) and `src/select/mod.rs:648-652`; F1/F2 were the forced-phase exceptions.
- `--files` path validation is strict: `validate_paths` rejects empty, backslash, absolute, colon, `..` components and empty components (`src/config/mod.rs:457-490`); F6 was the missing tag counterpart.
- `--no-chunk-blobs` stores nothing: `fill_or_plan` returns the plan-only index without calling `store.fill` (`src/blobs/fill.rs:48-57`); F7 was only the ceiling check before it.
- A12 is checked on the plan, before any blob is fetched: `blobs::plan` rejects a retention plan whose `patch_fetch` is not a subset of `materialize_new` (`src/blobs/plan.rs:86-103`).

### Second pass — residuals against F1–F12

Re-derived at `b726054`; all items accepted, and F1, F2, F7, F8, F9 and F10 needed no change.

| ID | Finding | Status | Commit |
|---|---|---|---|
| F6 residual | API-resolved tags reach `pair_dir` unvalidated (blocking). | fixed | `de85df2` |
| F6 reserved | Windows-reserved `* ? " < > \|` accepted in tags. | fixed | `de85df2` |
| F6 dots | A leading or trailing `.` collides on Windows. | fixed | `de85df2` |
| F4 order | `fill.rs` says errors are in `wants` order. | fixed (doc) | `583aa79` |
| F4 panic | A panicking chunk's `JoinError` payload is discarded. | fixed | `583aa79` |
| F5 log | A mid-read error-body failure is silent. | fixed | `0259220` |
| F1 budget | `Budget::with_spent` is dead and unclamped. | fixed | `abe7bdc` |
| F1 matrix | `matrix.json` reports `bytes_admitted`, not the charged ledger. | fixed | `998649c` |
| F3 test | Missing end-to-end test for a nonzero `forced_files_unscored`. | fixed (test) | `d8514c7` |
| F12 test | The expectation is derived from the constant under test. | fixed (test) | `d538d4d` |
| F11 doc | The doc overstates the cleanup guarantee. | fixed (doc) | `7c64557` |

### Third pass — G1–G3

Re-derived at `577c399`; all three confirmed valid.

| ID | Finding | Status | Commit |
|---|---|---|---|
| G1 | Document-supplied paths reach filesystem joins unvalidated (blocking). | fixed | `88d5413` |
| G2 | `tmp/` staging and universal-atomicity claims. | fixed | `b1b035b` |
| G3 | The "three causes" of a null `stored_path` omit `--no-chunk-blobs`. | fixed | `b1b035b` |

Two test-harness defects were fixed alongside: the `hyp` fixture lacked the now-required `audio_pkg_scan_dir` (`6003958`), and the mock's armed `503` sent `keep-alive` then closed (`66bc6f0`).

### Fourth pass — H1–H4

Re-derived at `1cbf2f1`; all four confirmed valid; H1 was reproduced end to end before the fix.

| ID | Finding | Status | Commit |
|---|---|---|---|
| H1 | `verify`/`materialize` still join document paths unvalidated (**high**). | fixed | `0a21ebe` |
| H2 | `--audio` becomes `matching_field` and reaches manifest filenames raw. | fixed | `0b55a40` |
| H3 | The path rule accepts names Windows folds together. | fixed | `c9caa02`, `b6aeb6d` |
| H4 | The `--no-chunk-blobs` schema row contradicts `unmeasured`. | fixed (doc) | `c9bf161` |

### Fifth pass — I1–I5

Re-derived at `9b60455`; all five confirmed valid; I1 and I2 were reproduced at the real seam before the fix.

| ID | Finding | Status | Commit |
|---|---|---|---|
| I1 | Two distinct forced paths of one primary class share a `capture_id`. | fixed | `afc08bd` |
| I2 | A failed `2xx` body is recorded with `bytes_in: 0`. | fixed | `f631c5c` |
| I3 | Duplicate patchmap ids are silently collapsed. | fixed | `34de040` |
| I4 | `list` silently skips unreadable directories and failed entries. | fixed | `5d24494` |
| I5 | `dump::write` validates the field but not the tag. | fixed | `f20ee4c` |

### Sixth pass — J1–J3

Re-derived at `ecc81c5`; all three confirmed valid; J1 was reproduced with a share-locked reference manifest (exit `0` before, usage error after).

| ID | Finding | Status | Commit |
|---|---|---|---|
| J1 | `list` treats a `MANIFEST.json` it cannot read as "not a bundle", so a real bundle vanishes with exit `0`. | fixed | `0706984`, `43d5977`, `3f042a1` |
| J2 | The `list` doc promises "deepest first"; the code sorts by relative-path string. | fixed (doc) | `28c3c18`, `d74e40b` |
| J3 | The `list` command doc calls a refused bundle "a skip" and omits I4's fail-fast usage error. | fixed (doc) | `73d0cb5` |

### Seventh pass — K1–K2

Re-derived at `8d25b06`; both confirmed valid and both reproduced outside the repository before the fix. K1's `Layout::of([(i64::MAX, 1)], 1)` aborted at `ranges.rs:48`, a manifest with `compressed_size: -1` produced `download_bytes: -1`, and three maximum `projected_download` wants aborted. K2's `Store::fill` left an unframed 58-byte XML body on disk while the row said `bytes: 0`, `stored_path: null` and the error list said "not a zstd frame".

| ID | Finding | Status | Commit |
|---|---|---|---|
| K1 | Arithmetic on untrusted offsets, lengths and costs is unchecked across the range algebra, plan totals, index decode, manifest decode, A20 and the diff totals. | fixed | `06bd550`, `e371ee9`, `37b7b2a`, `fc71387`, `ab560d8` |
| K2 | An undecodable fetched body is published on disk but recorded as an unfilled fetch, losing the evidence the spec says to keep. | fixed | `f7a8da2`, `41fdef9` |

K1 follows `decode_counts`: *measurements* saturate, while address-like sums use `checked_add` and return a typed error naming the chunk and values. `Range::at`/`len`/`to` are total; `Layout::of` records an overflowing end or total as a violation; `materialize::rebuild` checks offsets, lengths and `declared_size`; `ChunkIndex::validate_arithmetic` runs in `Bundle::open`; `SophonChunkFile::validate_arithmetic` runs at manifest decode and again in `assemble`; `totals_for` rejects a negative `compressed_size` and checks the sum; A20 rejects a negative cost and folds with `checked_add`; `projected_download` and `diff.rs` saturate. Every fixed site has a regression test, including a crafted index refused at `Bundle::open`.

K2 takes the spec-supported row: an unframed body is a successful fetch with `stored_path`, `bytes`, both compressed digests, `decode_error`, and `verified` carrying the two decode-independent answers and two `false` decode-dependent keys. Because the fetch succeeded, the store stops rather than asking a second source and renaming a good body over the evidence; a real I/O error after the rename removes the published blob so disk and index agree. Both docs now state this. A loopback `Store::fill` test drives the real store path.

### Eighth pass — L1–L2

Re-derived at `b9c1682`; both confirmed valid and both reproduced before the fix. L1 failed as `--old-tag "CON" must be refused`; L2's `select` fixture panicked in debug at `accum.rs:206:1` (`attempt to add with overflow`) and the release budget test reported `18446744073709551612` against the ledger's `u64::MAX`.

| ID | Finding | Status | Commit |
|---|---|---|---|
| L1 | A tag may name a Windows DOS device, so `manifests/<tag>` fails late or names a directory Windows cannot open. | fixed | `63ba129`, `f049c12` |
| L2 | `bytes_admitted` and `total_admitted` use plain `.sum()` while the budget saturates. | fixed | `06c650b` |

L1 makes `paths::is_windows_device_name` `pub(crate)` and calls it from `tag_shape_problem`, so the flag path, the API-resolved tags and the manifest writer share one rule; the extension is ignored, and `7.1.0`/`CONSOLE`/`COM0`/`LPT10` stay legal. L2 folds both totals with `saturating_add`, so the summary and `diff/matrix.json` report the same saturated number; a `select` fixture with two `u64::MAX - 1` costs pins it in debug and release.

### Ninth pass — M1–M2

Re-derived at `6561044`; both confirmed valid. M1 was reproduced before the fix:
its new `Checksums::findings` test panicked in debug at `accum.rs:206:1`
(`attempt to add with overflow`), and the release run reported a wrapped total.
M2's panic path was then exercised through a real `map_concurrently` call.

| ID | Finding | Status | Commit |
|---|---|---|---|
| M1 | `Checksums::findings` summed untrusted `entries[].bytes` with a plain `.sum()`, so a crafted `verify/checksums.json` panicked `gircap verify` in debug and wrapped in release, where the wrapped figure could equal the crafted `total_bytes` and suppress the disagreement. | fixed | `88407c0` |
| M2 | `map_concurrently` discarded the `JoinError` payload and reported every non-returning task as `"a request task panicked"`, losing the cause at all five metadata call sites and conflating cancellation with panic. | fixed | `47b0dc2` |

M1 is the L2 rule at the one total folded from a document rather than from the
tool's own measurement. The fold now uses `checked_add`, keeps the K1/L2
saturating figure, and turns an overflow into its own finding *before* the
equality test, so a crafted `u64::MAX` cannot certify an overflowing sum; an
exact `u64::MAX` sum stays legal and `u64::MAX - 1` is still a disagreement.
The test covers the review's two-`u64::MAX` wrapping pair, two
`u64::MAX - 1` entries, the saturated and exact boundaries, and one byte below
the boundary, in debug and release.

M2 uses `join_next_with_id` and records each spawn's task id against its item
index, so a panic keeps both index and cause: the slot becomes
`item {i} did not return: the request task panicked: <payload>`, and the log
line carries the same cause. The `JoinError` rule moved to `src/tasks.rs` with
a label, shared by `map_concurrently` ("request") and `Store::fill` ("chunk
fetch"); a cancellation is named as a cancellation. Pinned by a real
`map_concurrently` panic test and by helper tests for payload recovery and
cancellation.

### Tenth pass — N1

Re-derived at `385c4af`; N1 confirmed valid. Both direct `spawn_blocking`
joins formatted the `JoinError` with `Display`, dropping a panic's payload and
naming a cancellation a panic, and the measurement arm skipped
`discard_published`, so a published blob could outlive a `stored_path: null`
row.

| ID | Finding | Status | Commit |
|---|---|---|---|
| N1 | The two direct `spawn_blocking` joins discard a panic's payload, call a cancellation a panic, and leave an unmeasurable published blob without a `stored_path` row. | fixed | `1300931`, `a84d3c5` |

N1 moved both joins onto `tasks::join_blocking`, the blocking-pool application
of `join_failure`. The measurement keeps `<chunk id>: ` and now calls
`discard_published` on both failure arms, so disk and index agree; the
request-log write keeps `write <bundle>: ` and the writer's message. Each seam
has a panicking-closure test (`src/tasks.rs:81`, `src/blobs/mod.rs:1476`,
`src/run/mod.rs:914`); no portable input makes the real decoder or writer
panic, so that instruction remains unexercised.

### Eleventh pass — O1–O4

Re-derived at `bcb4c38`; all four confirmed and reproduced before the fix. O1
panicked with `capacity overflow` on a `Content-Length` of
`(isize::MAX as u64) + 1` and asked the allocator for 10 GiB below that; O2
returned a green A17 for a five-row and for a lone-unknown-class matrix; O3
printed `OverBudget`, not `over_budget`; O4's ~8 KiB zstd frame expanded to
268 435 456 bytes under `zstd::decode_all`.

| ID | Finding | Status | Commit |
|---|---|---|---|
| O1 | A success body sizes its buffer from an untrusted `Content-Length` before the response ceiling rejects it, so a header can abort or panic instead of producing the typed refusal. | fixed | `bb43a8b` |
| O2 | A17 checks each present row's state but not completeness against `Class::ALL`, so `verify` certifies a matrix with a missing or unknown class. | fixed | `daca14c` |
| O3 | The `--require-class` failure text uses the `Debug` spelling (`OverBudget`) and its doc example names a variant that does not exist. | fixed | `066a3e9` |
| O4 | Manifest and bundle zstd output is unbounded; the declared uncompressed size is compared only after `decode_all` materialises the frame. | fixed | `d17fb18` |

O1 adds `body_reservation`, which clamps the declared length to the response
ceiling and to a 64 KiB starting size; the typed `too_big` refusal still comes
from `drain_capped`. Its loopback test covers both the review's 10 GiB header and
the `isize::MAX + 1` value that used to panic before any read.

O2 makes `Manifest::a17` compare the matrix with `Class::ALL`: every class
exactly once, no unknown class, no duplicate, and one of the four documented
states. `verify` reaches the same function and reports the result as a failed
A17 row and `agrees: false`; `Bundle::open` remains a structural reader.

O3 renders the row through `MatrixStatus`'s existing `Display` (which delegates
to `id`) and corrects the example to `reuse_moved: over_budget (...)`.

O4 derives the decode limit from the declared or expected size, capped by
`MAX_DECOMPRESSED_BYTES = 256 MiB`, and reads through `take(limit + 1)` so an
overrun is a typed refusal before the rest is materialised. The initial
capacity is clamped to 64 KiB, never the untrusted declaration. The manifest
path computes the declared sizes first; the bundle path derives the limit from
the index's `uncompressed_size`.

`177f304` is a test-fixture follow-up: the closeout suite's seeded selection now
carries one `unavailable` row per class, because strict A17 correctly rejects
the empty matrix the old fixture used.

### Decisions that still constrain current behaviour

- **F1/F2/F9.** Forced paths share one cumulative budget; a refusal is recorded and the loop continues; duplicate `--files` paths are a usage error.
- **F1 matrix.** `matrix.json` reports the charged ledger, with a debug assertion tying it to `bytes_admitted`.
- **F5.** An error body is drained through a counting cap, but the status is always preserved, partial bytes are recorded, and a 404 is never retried.
- **F6/G1.** One shared validator covers every document-supplied path: no `\`, absolute path, `:`, `..`, empty component or control character on any host; tags also refuse Windows-reserved characters and leading/trailing dots. A chunk id must match `<16 hex>_<32 hex>` (case-insensitive), not merely be one component.
- **L1.** A tag also refuses DOS device names through the same `paths::is_windows_device_name` helper the path rule uses, extension ignored (`CON`, `NUL.txt`, `COM1`, `CONIN$`), on the flag path, the API-resolved tags and the manifest writer.
- **L2.** `Selection::bytes_admitted` and `budget::total_admitted` saturate with the charged ledger, so a crafted maximum cost cannot panic in debug or wrap in release.
- **M1.** The checksums total is folded from an untrusted document: it saturates for reporting, and an overflow is its own finding before the equality test, so `u64::MAX` is agreement only when the entries really sum to it.
- **M2.** A failing task keeps its item index and its `JoinError` payload; cancellation is named as cancellation, not as a panic; both `JoinSet` loops use one helper (`src/tasks.rs`).
- **N1.** A blocking task that does not return goes through `tasks::join_blocking`, so its payload survives; a panicking measurement removes the already-published blob rather than leaving it without an index row.
- **O1.** A success body reserves `min(declared, ceiling, 64 KiB)` bytes up front; the declared header is never an allocation size, and the typed `too_big` refusal still comes from `drain_capped`'s checks.
- **O2.** A17 is equality with `Class::ALL`: exactly six classes, each once, each in `captured`/`covered_by`/`over_budget`/`unavailable`. An unknown, missing or duplicated class fails A17; `verify` surfaces it as a failed A17 row and `agrees: false`, while `Bundle::open` stays a structural reader.
- **O3.** Requirement failures use the artifact spelling from `MatrixStatus`'s `Display`/`id`, not its `Debug` variant name.
- **O4.** A zstd decode is limited by the declared/expected uncompressed size, capped absolutely at 256 MiB, and reads `limit + 1` bytes to detect an overrun. The initial capacity is at most 64 KiB; an absent declaration gets the ceiling, and a false or absent declaration cannot make the output unbounded.
- **F3.** A classless forced path is reported in `forced_unscored`/`forced_files_unscored`, not overloaded onto `forced_skipped`.
- **F4.** The fetch reason goes in `decode_error`; a dedicated `fetch_error` remains open.
- **F7.** A metadata-only run does not enforce a chunk wire-byte ceiling it pays nothing towards.
- **F8.** Recorded argv is lossy, not byte-exact; replay would need a raw-bytes companion.
- **F11.** A handled fetch error removes the temp file best-effort after the handle is dropped; a panic unwinds past cleanup.
- **F12.** The closeout test pins an independent eleven-id literal and asserts it equals the constant.
- **H1.** `Bundle::open` validates document paths once; the older sink checks remain as defence in depth.
- **H2/H3.** `--audio` is one file-name component; names Windows folds together (trailing dot/space) and DOS device names are refused on every host.
- **I1.** Forced admissions are numbered per primary class; phase 1's `taken` still starts at zero so a budget-refused class stays over-budget, not absent; the matrix resolves a class to its first admission.
- **I3.** A patchmap is keyed by its declared `capture_id`; its filename is not authoritative.
- **I4/J1.** `list` fails fast on an unreadable directory, entry or manifest; a non-bundle is stepped over and a refused bundle is named and counted.
- **I5.** The manifest writer repeats the CLI's tag validation by design.
- **J2.** `list` orders by relative path, not deepest-first; deepest-first would be a new contract.
- **K1.** Measurement totals saturate; address-like sums and plan costs use `checked_add` with an error naming the offending chunk and values. No negative `offset`, `compressed_size` or `uncompressed_size` is accepted at a decode boundary, and no `uncompressed_size` above `i64::MAX` survives index validation.
- **K2.** An unframed fetched body is a successful fetch and an `undecodable` row, not a failed fetch; it is not retried from another source, because replacing it would destroy the evidence. A post-rename I/O error removes the published blob.

### Verification (latest: eleventh pass, `177f304`)

| Command | Result |
|---|---|
| `cargo build -p gircap` | finished, no warnings (0.48s cached; the fix commits built in 9.01s or less) |
| `cargo test -p gircap` | **711 passed, 0 failed, 0 ignored** (566 lib + 145 integration/doc) |
| `cargo clippy -p gircap --all-targets` | finished, no diagnostics (33.11s) |
| `cargo fmt -p gircap --check` | clean, no output |

Integration total 3 + 8 + 32 + 2 + 30 + 10 + 29 + 16 + 15; no test is
`#[cfg(unix)]`, so none was skipped and the `#[cfg(windows)]` tests ran. The
eleventh pass adds nine library tests (O1 one, O2 five, O3 one, O4 two) and one
`tests/offline_bundle.rs` test, and the closeout fixture change is covered by
the existing closeout suite.
