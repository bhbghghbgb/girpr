# Open for maintainers — `gircap` (`tools/capture`)

Deliberate trade-offs, unverified areas and product decisions on `fix-gaps/gircap`, not open defects.
Context, pass log and verification: [ADVERSARIAL_REVIEW.md](ADVERSARIAL_REVIEW.md).

## Suggested decision order

1. **Contract first.** Decide F4 (`fetch_error`), F8 replay (raw argv companion) and O2 (`Bundle::open` structural). These decide whether a bundle gains a field, a document or a new acceptance rule, and therefore whether any bundle must be re-captured.
2. **Gate semantics.** Decide F7/F1 (wire ceiling and metadata-only), F6/G1, L1, H3 and I5 (what the tool refuses). These change what a capture accepts, so they also gate a re-capture.
3. **Failure, cleanup and arithmetic.** Decide F11, G2, K2, M1, L2 and K1. These change what a failed or hostile bundle looks like, not the happy-path document set.
4. **Resource bounds.** Decide O1, O4 and F5. These change memory ceilings and error-body accounting only.
5. **Evidence only.** F8 clap typing, K1 A20, I4/J1, K1/K2 live inputs and H4 need a crafted bundle or a live capture before a decision is possible. Do not decide them from the current mock/reference corpus.

Rationale: 1–2 can stale existing bundles; 3–4 cannot; 5 is sampling, and one capture or one crafted bundle can settle several rows at once.

## 1. Artifact contract and replay

### F4 — a dedicated `fetch_error` field

**Current.** A failed fetch writes `stored_path: null`, `verified: null`, and the failure text in `decode_error` (`src/blobs/mod.rs:420-438`). `BlobProps::unmeasured` is used for `--no-chunk-blobs`, and `unfilled` overwrites `decode_error` with the fetch reason for a fetch that was attempted. `decode_error` is documented as the field that answers "why is this blob unusable", including a fetch that never produced a body (`docs/01-artifact-schema.md:614-644`; `src/blobs/props.rs:377-401, 432-439`).

**History.** `f31850b` kept a failed chunk's slot and its reason instead of losing both; `583aa79` added the panic payload; `f7a8da2` made an undecodable body a kept row rather than a failed fetch. The first pass recorded the reason in a generic row; the field name has not changed.

**Dependents.** `verify` and `selfcheck` read `decode_error` as the unusable reason; `is_usable` treats any non-null value as unusable (`props.rs:426-439`). `tests/schema.rs:300-324` pins the current blob-entry keys. No code reads a `fetch_error` field.

**Options.** (a) Keep `decode_error` — no schema change, one field means one question, but a fetch failure and a decode failure share a name. (b) Add an optional `fetch_error` field — additive under the README's rule (`README.md:101-109`), old readers ignore it, but writers/docs/tests must agree and a reader now has two fields to classify. (c) Bump `schema_version` to make it required — clean separation, but every existing bundle becomes stale and the `girpr` bridge needs the new version. Failure mode for (b): a reader that keys only on `decode_error` sees a null reason for a failed fetch unless the writer keeps writing both; (c) breaks every bundle in `artifacts/`.

**Code/tests/docs.** `src/blobs/props.rs` `BlobProps`, `src/blobs/mod.rs:420-438`, `docs/01-artifact-schema.md` §12, `tests/schema.rs:300-324`. No code change under (a).

**Recommendation.** Keep (a). `stored_path` and `verified` already distinguish "not attempted" from "attempted and failed" from "decoded and failed"; a new field is justified only by a consumer that cannot make that distinction.

**Verification limits.** No external consumer was found that reads `chunks/index.json`; the bridge reads the matrix, patchmaps, blobs and `expected/` (`docs/06-artifact-bridge-spec.md`). Evidence to settle: a consumer that filters failed fetches without decoding `decode_error`.

**Cross-project impact.** Bundle contract only. An optional field leaves `schema_version` at 2 and does not affect `girpr`'s bridge; a version bump would affect `docs/06-artifact-bridge-spec.md` and `tests/artifact_bridge*`.

### F8 — replay of the recorded argv

**Current.** `capture.tool_argv` is recorded from `args_os()` through `to_string_lossy` (`src/cli/mod.rs:398-416`, written at `src/emit/index.rs:130`). A non-Unicode argument becomes U+FFFD in the record. The differ treats `tool_argv` as a volatile field, not a replay input (`docs/04-bundle-differ.md:64, 299`).

**History.** `df366ee` replaced `std::env::args()` after parsing; that call panicked on non-Unicode arguments, exited `101` and printed no `SUMMARY`. The lossy record is the fix, not an oversight.

**Dependents.** `tests/schema.rs:246` requires `tool_argv`; `tests/offline_bundle.rs:355` constructs it; `tests/cli_contract.rs:175` exercises a non-Unicode `--out`. No replay command exists.

**Options.** (a) Keep lossy argv — every run records something, no schema change; a replay of a non-Unicode path would target the wrong file. (b) Add an optional raw companion (array of byte arrays or base64) — replay becomes exact; additive, but JSON grows and every writer/test must populate it. (c) Store raw bytes instead of strings — exact, but breaks every existing reader and `schema_version`. Failure mode for (a): a future `replay` command silently uses a different path; (b) without a replay command is unused surface.

**Code/tests/docs.** `src/cli/mod.rs:409-416`, `src/emit/index.rs:130`, `docs/01-artifact-schema.md:101-107`, `tests/schema.rs:246`.

**Recommendation.** Keep (a) until a replay command exists. If replay is added, add (b) as an optional field; do not change the meaning of `tool_argv`.

**Verification limits.** No non-Unicode argument other than `--out` is exercised end to end; which clap argument types accept non-Unicode values is F8 clap typing below.

**Cross-project impact.** None unless a new field is added; the bridge does not read `capture.tool_argv`.

### O2 — completeness outside `verify`

**Current.** `Bundle::open` is a structural reader; it accepts a matrix with a missing or unknown class. A17 requires equality with all six classes and is checked only by `verify` (`src/emit/index.rs:1395-1438`; `src/verify/mod.rs:603-619`). The test `daca14c` is the regression. The bridge already makes `gircap verify --artifact <dir>` its precondition (`README.md:147-162`; `docs/06-artifact-bridge-spec.md:198-203`).

**History.** Before `daca14c`, A17 checked only the rows that were present, so a five-row matrix or a single `bogus` row could certify itself complete.

**Dependents.** `verify` reports A17 as a row; `list` and `Bundle::open` do not. Any consumer that reads `MANIFEST.json` directly (without `verify`) owns its own completeness check. The bridge does run `verify`.

**Options.** (a) Keep `Bundle::open` structural — offline diagnostics can open a bundle that `verify` then names as incomplete; consumers must run `verify`. (b) Enforce A17 in `Bundle::open` — one acceptance rule, but a bundle that is useful evidence becomes unreadable and `verify` cannot report the detailed row. (c) Add a strict open used by consumers — explicit, but a second entry point and every caller must choose. Failure mode for (a): a direct reader trusts an incomplete matrix; for (b): a diagnostic command cannot open the bundle it is meant to explain.

**Code/tests/docs.** No code change under (a). `src/emit/index.rs:1395`, `src/verify/mod.rs:603`, `README.md:147-162`, `docs/06-artifact-bridge-spec.md:198-203`.

**Recommendation.** Keep (a); the documented consumer contract is `verify` first. If a direct reader appears, point it at `verify` rather than weakening the structural reader.

**Verification limits.** No consumer other than the bridge was inspected.

**Cross-project impact.** `girpr` bridge: keep `gircap verify` as the precondition; a direct `Bundle::open` consumer would need its own A17 check. No `girpr` dependency or invocation is introduced.

### H4 — a real `--no-chunk-blobs` bundle

**Current.** `--no-chunk-blobs` rows come from `BlobProps::unmeasured` (`src/blobs/props.rs:377-401`): `stored_path`, `verified` and `decode_error` are null, `bytes` is 0, compressed digests are empty strings, and `md5_decompressed`/`uncompressed_size` are the manifest's declared values. The wire-byte ceiling is skipped (`src/blobs/plan.rs:122-140`); A6–A12, A19 and A21–A23 are skipped (`src/emit/closeout.rs:175-184`; schema §12/§16). No real `--no-chunk-blobs` bundle has been inspected; the doc matches the constructor and the tests.

**History.** `c9bf161` corrected the docs to say what an unmeasured row carries; `f7a8da2` is the different case of an attempted fetch that arrived but did not decode.

**Dependents.** `verify` reports `verified: null` as "not measured"; a reader must treat the row as a declared gap. The bridge is not a consumer: it needs chunk blobs and `expected/` bytes.

**Options.** (a) Keep — a metadata-only capture is a declared, self-describing gap. (b) Remove the flag — one bundle shape, but loses the cheap metadata-only capture. (c) Capture one real bundle and keep it as a fixture — proves the doc and the verify path. Failure mode for (a): a consumer mistakes a null `verified` for a clean measurement; the schema already says otherwise.

**Code/tests/docs.** No code change under (a). `src/blobs/props.rs:377`, `src/emit/closeout.rs:175-184`, `docs/01-artifact-schema.md` §12/§16.

**Recommendation.** Keep (a), and run (c) once before relying on the shape in a release. A metadata-only bundle is not a bridge fixture.

**Verification limits.** No on-disk metadata-only bundle, and no `verify` run over one.

**Cross-project impact.** None; the bridge requires chunk blobs, so a `--no-chunk-blobs` bundle is outside its contract.

## 2. Gates and validation

### F7 — the metadata-only skip of the wire-byte ceiling

**Current.** `--max-download-bytes` is documented as a ceiling on the summed cost of admitted files (`src/cli/mod.rs:157-159`) and is charged against `total_cost` in admission. A second, independent check bounds distinct chunk `compressed_size` on the wire (`src/blobs/plan.rs:37-54`). Under `--no-chunk-blobs` the second check is skipped because the run fetches no chunk bytes; A12 still runs (`plan.rs:122-140`). The boundary is `>`, so a store exactly on the ceiling is affordable.

**History.** `801d91d` introduced the skip and states `projected <= bytes_admitted <= cap`, so the wire check is a guard against a future charging change, not a limit a current run can hit.

**Dependents.** The plan document records `projected`; the log announces it; admission uses the same flag with a different total. Tests: `a_metadata_only_plan_is_not_refused_by_the_wire_ceiling` (`plan.rs:209`), `a_metadata_only_plan_still_lists_every_want` (`:247`), `a_store_over_the_ceiling_is_refused_with_both_numbers` (`:266`), `a_store_exactly_on_the_ceiling_is_affordable` (`:285`).

**Options.** (a) Keep the skip — the flag is admission cost, and a metadata-only run sends no chunk bytes. (b) Revert and enforce — treats the flag as an input-independent policy check, but refuses a run for bytes it will not fetch and requires changing the help and spec. (c) Split into `--max-wire-bytes` — each flag has one meaning, but adds a flag and a second budget surface. Failure mode for (b): an operator who sized `--max-download-bytes` for admission cannot run metadata-only; for (c): two ceilings to document and test.

**Code/tests/docs.** `src/blobs/plan.rs:122-140`, `src/cli/mod.rs:157-159`, `docs/00-spec.md` §5, the four `plan.rs` tests above.

**Recommendation.** Keep (a). If maintainers intended an input-independent policy, (b) is coherent only with a help/spec change that says the flag bounds the hypothetical store; the commit message and spec currently say otherwise.

**Verification limits.** The reference bundle has `limit: null`, so no real capture exercised a ceiling near `projected`.

**Cross-project impact.** None.

### F1 — the projected check on a real corpus

**Current.** Admission charges `total_cost`; the wire check sums distinct chunk `compressed_size` over the admitted captures. The distinct-chunk store is a subset of the charges, so `projected <= bytes_admitted` by construction; the check cannot fire on a normal run unless charging changes. The plan still computes `projected` and reports it (`src/blobs/plan.rs:112-140`).

**History.** `801d91d` records the ordering explicitly. The first pass treated "does the projected check fire on a real corpus?" as unverified; the ordering settles it: it is a guard, not a reachable limit.

**Dependents.** `tests/measured_fixture.rs:713, 768, 1240` drive selection/ceiling boundaries; `tests/matrix.rs:232` checks admission ordering and totals. No test drives a hand-edited manifest through the check.

**Options.** (a) Keep the guard — cheap, independent of budget arithmetic, and names both numbers if a future change breaks the ordering. (b) Remove it — one less redundant check, but loses the second, wire-based ceiling and its operator message. (c) Turn it into a debug assertion — no operator-facing refusal, but a release build would silently proceed. Failure mode for (b): a future charging change over-admits and nothing catches it; for (c): a release run spends bandwidth the guard would have refused.

**Code/tests/docs.** `src/blobs/plan.rs:37-54, 112-140`; tests `plan.rs:266, 285` and `tests/measured_fixture.rs:713, 768, 1240`.

**Recommendation.** Keep (a); it is explicitly a guard against a future charging change. No code change.

**Verification limits.** No capture was run with a ceiling between `projected` and `bytes_admitted`; admission would refuse first.

**Cross-project impact.** None.

### F6/G1 — live hostile inputs

**Current.** One shared shape rule refuses empty/control/backslash/absolute/drive/`..`/`.`/trailing-dot-or-space/device/empty components (`src/paths.rs:34-89`); the tag rule adds Windows-invalid characters, length, leading/trailing dot and device names (`src/config/mod.rs:581-627`); the writer re-checks field and tag before the first `create_dir_all` (`src/manifest/dump.rs:80-99`); chunk ids must be `<16 hex>_<32 hex>` (`src/paths.rs:142-155`). Local sinks and rejection paths are tested.

**History.** `efc5322` closed separators/prefixes in tags; `b6aeb6d` device names as paths; `c9caa02` Windows-folded names; `0b55a40` audio fields; `88d5413`/`0a21ebe` the verify side.

**Dependents.** `--files`, manifest paths, `chunks/index.json` `stored_path`/`referenced_by`, `patchmaps/*.json` paths, audio codes, tags and chunk URLs all use these rules. A live API that echoes a hostile tag, serves a hostile document/audio path, or uses an unusual-but-legitimate chunk-id spelling is unverified.

**Options.** (a) Keep strict — the rule only rejects shapes that cannot name a distinct file in a bundle. (b) Relax chunk-id shape to the CDN's actual spelling — would accept a live input, but also admits URL metacharacters and `..` into a file name. (c) Log rejected values with their source — makes a live refusal diagnosable, but adds log volume. Failure mode for (a): a legitimate live tag/path is refused and the capture cannot proceed; for (b): a hostile value becomes a path or URL segment.

**Code/tests/docs.** `src/paths.rs:34-159`, `src/config/mod.rs:452-470, 581-627`, `src/manifest/dump.rs:80-99`, schema §12, `docs/00-spec.md` §7.

**Recommendation.** Keep (a). Evidence to settle: a live tag, manifest path or chunk id that the rule rejects and the CDN accepts. Until then, refusing is the safe side.

**Verification limits.** No live manifest/tag/audio path outside the rule was sampled; only local sinks and unit tests cover the rejection.

**Cross-project impact.** None.

### L1 — device names end to end and across hosts

**Current.** A device-name tag is refused at argument resolution before any request (`src/config/mod.rs:618-622`); a device-name path is refused by the shared rule (`src/paths.rs:103-112`); `verify` refuses a bundle document that names one (`src/verify/mod.rs:305-337`). A16's missing-file consequence is reasoned, not executed. No capture was run with a device-name tag, and a Linux-produced `manifests/CON/` bundle was not replayed through `verify` on Windows.

**History.** `63ba129` added device names to the tag rule; `b6aeb6d` to paths; A16 already treats a listed-but-missing file as a finding.

**Dependents.** Tags become `manifests/<tag>`; paths become blob/capture names; a bundle can move between hosts, so a rule enforced only on Windows would not protect a Linux reader.

**Options.** (a) Keep — a bundle that names a device is not portable. (b) Relax on non-Windows — accepts a local capture but produces a bundle Windows cannot read. (c) Add a cross-host replay test — synthetic `CON` directory in a bundle, run `verify` on Windows. Failure mode for (b): a bundle is written on Linux and silently unreadable on Windows; for (a): a legitimate file literally named `CON` cannot be captured.

**Code/tests/docs.** No code change under (a). `src/config/mod.rs:618`, `src/paths.rs:103`, `src/verify/mod.rs:305`; test `src/manifest/dump.rs:418`; add (c) under `tests/` if bundles are shared.

**Recommendation.** Keep (a); add (c) if bundles cross hosts. Evidence to settle: a Linux-produced bundle with a device-named component.

**Verification limits.** No device-name capture and no cross-host replay; A16's consequence is reasoned.

**Cross-project impact.** None.

### H3 — live manifests with Windows-folded names

**Current.** A component ending in `.` or a space is refused (`src/paths.rs:68-76`); an audio field must be one file-name component (`src/paths.rs:122-132`; `src/config/mod.rs:452-470`). Whether a live manifest contains names differing only by a trailing dot or space was not sampled.

**History.** `c9caa02` added the folding rule; `0b55a40` extended the field rule to audio.

**Dependents.** Two manifest entries that fold onto one file would make the last write win and the bundle list bytes it does not hold. The rule runs on every manifest path, not only on `--files`.

**Options.** (a) Keep — refuse the ambiguous document. (b) Normalise and deduplicate — captures more, but invents a mapping the CDN did not declare. (c) Report all offenders then refuse — better operator message, more code. Failure mode for (a): a legitimate live name is refused; for (b): two distinct upstream files collapse silently.

**Code/tests/docs.** `src/paths.rs:68-76, 122-132`; `src/config/mod.rs:452-470`; tests in `src/paths.rs`.

**Recommendation.** Keep (a). Evidence to settle: a live manifest with such a name.

**Verification limits.** No live manifest sample; only synthetic unit tests.

**Cross-project impact.** None.

### I5 — the CLI precondition versus the writer's own check

**Current.** The CLI validates resolved tags (`src/config/mod.rs:523-539`) and the writer re-checks field and tag before writing (`src/manifest/dump.rs:80-99`). A hostile tag reaches the writer only through a hand-built `CapturedManifest`; the writer's refusal is unit-tested (`src/manifest/dump.rs:418`). The CLI's earlier validation is not re-driven through that path.

**History.** `f20ee4c` made the dump refuse a traversal tag; `de85df2` validated API-resolved tags before they name a directory; `efc5322` widened the tag rule.

**Dependents.** The CLI check avoids network work for an invalid tag; the writer check protects in-crate callers that bypass the CLI. Both call the same `tag_shape_problem`/`field_component_problem`, so they cannot disagree about the rule.

**Options.** (a) Keep both — defense in depth, one duplicate call. (b) Remove the CLI check — one place to change, but an invalid tag reaches the network before failing. (c) Add a property test over all `CapturedManifest` constructors — proves the writer path, more test surface. Failure mode for (b): wasted work and a late error; for (c): no production change.

**Code/tests/docs.** No code change. `src/config/mod.rs:523`, `src/manifest/dump.rs:80-99, 418`.

**Recommendation.** Keep (a); the duplication is two calls to one rule, not two rules.

**Verification limits.** No end-to-end run with a hand-built hostile manifest; the writer test drives the refusal directly.

**Cross-project impact.** None.

## 3. Failure, cleanup and arithmetic

### F11 — older temp-file orphans

**Current.** Handled fetch/flush/rename errors drop the handle and call `discard_tmp` (`src/blobs/mod.rs:466-474, 492-517`); a post-rename measurement failure calls `discard_published` (`:549-568, 589-608`). A panic unwinds past both. Orphans from older binaries are not swept, and the historical accumulation rate is unmeasured. The tmp name carries the slot, so a leftover is reused only if the same chunk lands in the same slot in the same directory.

**History.** `882a53b` removed a failed chunk's temp file instead of orphaning it; `1300931` added the published-blob discard.

**Dependents.** `chunks/blobs/.*.tmp` files are ordinary regular files to the close-out walk: an orphan is listed in `MANIFEST.json → files[]` and `verify/checksums.json` and passes A16, so it silently inflates the bundle instead of becoming a finding. It also consumes disk. No reader distinguishes it from an intended file.

**Options.** (a) Keep — no cross-process delete risk. (b) Sweep `.*.tmp` at store start — clears old runs, but can delete a concurrent run's live temp files. (c) Sweep only files older than a threshold — safer for concurrency, needs an mtime/clock heuristic. Failure mode for (b): two captures sharing `--out` corrupt each other; for (c): a long run's own temp files could be swept if the threshold is short.

**Code/tests/docs.** `src/blobs/mod.rs:577-608`; add a test that a failing fetch leaves no `.tmp` (already covered by the fetch-error tests) and, under (b)/(c), a sweep test.

**Recommendation.** Keep (a) unless captures share an `--out`; the current design already avoids reuse by slot. Evidence: measured orphan count after a panicking run, or a shared-output workflow.

**Verification limits.** No panic-orphan accumulation was measured; cleanup is tested for handled errors only.

**Cross-project impact.** None.

### G2 — a crash tear in metadata

**Current.** Metadata documents are written non-atomically by design; only chunk blobs go through temp+rename (`src/emit/mod.rs:285-333`). The close-out writes documents, then `checksums.json` (`src/emit/closeout.rs:160, 205-215`), then rewrites `selfcheck.json`/`README.md` and `checksums.json` (`:217-240`). A torn document fails `read_json`; a stale checksum list fails A16. A tear that `checksums.json` misses was not stress-tested.

**History.** `111f69c` made the two-pass close-out a fixpoint; the second pass is unconditional and re-writes exactly the two documents A16 moves.

**Dependents.** `verify`/`Bundle::open` read these documents; A16 is the independent digest half. A bundle left torn by a crash is not a fixture, but `verify` should name it rather than pass.

**Options.** (a) Keep — a torn bundle fails verification; do not add an atomic-write layer for small documents. (b) Write every metadata document via temp+rename — stronger crash story, but more code and Windows rename/replace behaviour for files a reader may hold open. (c) Add a `.complete` marker last — clear signal, but a new document becomes required and stales every existing bundle (`README.md:101-109`). Failure mode for (a): a torn checksum list is an unreadable bundle rather than a named finding; for (c): every bundle in `artifacts/` becomes stale.

**Code/tests/docs.** No code change under (a). `src/emit/mod.rs:285-333`, `src/emit/closeout.rs:128-240`; add a crash-injection test if (b) is chosen.

**Recommendation.** Keep (a); do not add (c), because it is a new required document by the README's own compatibility rule.

**Verification limits.** No crash harness; the tear scenario is reasoned from the write order.

**Cross-project impact.** None.

### K2 — the removal arm after a failed measurement

**Current.** An undecodable body is kept: bytes are on disk before measurement, `decode_error` carries the reason, and the row points at the bytes (`src/blobs/mod.rs:461-464`; schema §12). A post-rename measurement I/O error, panic or cancellation removes the published blob (`measure_published`, `src/blobs/mod.rs:549-568`; `discard_published`, `:589-608`) and, unless a later source fills the slot, the row becomes `stored_path: null` with the reason. No test forces the I/O failure; the undecodable path is the tested post-rename path.

**History.** `f7a8da2` kept an undecodable body as evidence; `1300931` added the published-blob discard for a measurement that did not return.

**Dependents.** `is_usable` requires `stored_path`, `verified` all-green and no `decode_error` (`src/blobs/props.rs:426-439`). The asymmetry is deliberate: a body that exists but is not a frame is evidence; bytes whose measurement failed cannot be certified by the index.

**Options.** (a) Keep the asymmetry — disk and index agree, and undecodable evidence survives. (b) Keep the blob on a measurement I/O error and record the reason — preserves bytes, but the index cannot certify their length/digest. (c) Remove on undecodable too — one rule, but loses the evidence the tool exists to keep. Failure mode for (b): a reader assembles from bytes whose declared length was never verified; for (c): a mis-served chunk disappears from the fixture.

**Code/tests/docs.** `src/blobs/mod.rs:549-568, 589-608`; add a test seam that injects an I/O error into `measure_file` under (a)'s verification.

**Recommendation.** Keep (a); the two cases answer different questions. A test for the removal arm is worthwhile only if the measurement path changes.

**Verification limits.** The removal arm is exercised by the panic test at the same join (`src/tasks.rs:81`) but not by an I/O error.

**Cross-project impact.** None.

### M1 — checksum totals through the command

**Current.** `Checksums::findings` folds entry sizes with `checked_add`, saturates at `u64::MAX` and records that the true sum does not fit (`src/emit/index.rs:437-464`). The test `a_crafted_checksums_total_cannot_overflow_or_certify_an_impossible_sum` (`:722`) drives `findings` directly, not a written bundle through `gircap verify`; `Bundle::open → reverify → findings` is an unchanged direct call.

**History.** `88407c0` saturated the untrusted checksums total; the earlier plain `.sum()` panicked in debug and wrapped in release, where the wrapped figure could equal a crafted `total_bytes`.

**Dependents.** A16 and `verify`'s checksum row call `findings`; a crafted `total_bytes: u64::MAX` must not compare equal to a saturated sum.

**Options.** (a) Keep the unit seam — the integration adds only JSON plumbing. (b) Add a crafted-bundle test — proves the read path, but a fixture must be assembled by hand. (c) Change the total to a checked type — stronger, but a schema change. Failure mode for (a): a future change to `Bundle::open` breaks the wiring without the unit test noticing.

**Code/tests/docs.** No code change. `src/emit/index.rs:409-481, 722`.

**Recommendation.** Keep (a); add (b) only when the bundle-root read path changes.

**Verification limits.** No hand-edited `checksums.json` was run through `gircap verify`.

**Cross-project impact.** None.

### L2 — an extreme cost through the command

**Current.** Admission and totals saturate rather than overflow; the near-`u64::MAX` cost is driven through `select` with a `PathScore` fixture (`tests/matrix.rs:232`), not a hand-edited manifest read from disk. No live manifest declaring extreme sizes was sampled.

**History.** `fc71387` extended the saturating arithmetic to the verify/index path; the select path already had the same rule.

**Dependents.** `bytes_admitted`, `total_cost`, budget comparisons and the summary totals. A negative `compressed_size` is clamped by the same validation.

**Options.** (a) Keep — the arithmetic is the same function. (b) Add a manifest fixture with near-`u64::MAX` sizes and run selection offline — proves the read/parse path. (c) Reject extreme declarations earlier — clearer error, but needs a documented bound. Failure mode for (a): a parser-level overflow reaches selection unnoticed; (c) refuses a legitimate large corpus.

**Code/tests/docs.** `src/select/budget.rs`, `tests/matrix.rs:232`. No code change.

**Recommendation.** Keep (a); add (b) if manifest number handling changes.

**Verification limits.** No live or on-disk manifest with extreme sizes.

**Cross-project impact.** None.

### K1 — A20 through the command

**Current.** A20's negative and overflowing cases are unit-tested against a crafted `PatchMap`; no full bundle with a hand-edited patchmap was run through `gircap verify`. `fc71387` added the index and A20 arithmetic checks.

**History.** `fc71387` is the fix; the unit test is the seam.

**Dependents.** `verify` re-derives A20 from `patchmaps/*.json`; a negative `old.size`/`new.size` or a sum that overflows must be a finding, not a wrap.

**Options.** (a) Keep the unit seam. (b) Add a crafted bundle fixture. (c) Validate patchmap numbers at write time too — defense in depth, but duplicates A20. Failure mode for (a): the bundle read path could pass a value the unit test does not construct.

**Code/tests/docs.** `src/verify/mod.rs` A20 path; no code change.

**Recommendation.** Keep (a); add (b) when the patchmap schema next changes.

**Verification limits.** No hand-edited patchmap through `verify`.

**Cross-project impact.** None.

## 4. Concurrency and resource bounds

### M2 — cancellation text

**Current.** `join_failure` reports a cancelled task as "cancelled", not as a panic, and preserves a panic payload (`src/tasks.rs:20-31`); the helper is tested (`:68-78`). `map_concurrently` never aborts a spawned task, so the slot text for a cancelled metadata job was not driven end to end.

**History.** `47b0dc2` kept the cause and index of a failing request task; `583aa79` kept the panic reason.

**Dependents.** `Fetcher::map_concurrently` and `Store::fill` use the same rule; the summary counts a slot once.

**Options.** (a) Keep — cancellation is only reachable on runtime shutdown, where a distinct text is already best-effort. (b) Abort a task on a timeout and test the resulting text — makes cancellation a real path, but changes request semantics. (c) Treat cancellation as a panic — simpler, but wrong. Failure mode for (b): a timed-out request is recorded as cancelled and not retried; for (c): an operator sees a panic that never happened.

**Code/tests/docs.** `src/tasks.rs:20-78`, `src/http/mod.rs:825-853`; no code change under (a).

**Recommendation.** Keep (a); cancellation is a shutdown fact, not a request outcome.

**Verification limits.** Cancellation is exercised on the helper, not through `map_concurrently`.

**Cross-project impact.** None.

### N1 — panic seams and the published-blob discard

**Current.** Both direct `spawn_blocking` joins share `join_blocking` (`src/tasks.rs:40-49`); the panic mapping and discard are driven with a panicking closure at the same seam production uses (`src/tasks.rs:81`). No portable input makes `BlobProps::measure_file` or `emit::write_requests_log` panic; cancellation at those joins was not driven. A measure task that panics removes the already-published blob and the row says `stored_path: null`; that loss is deliberate and differs from K2's kept evidence.

**History.** `1300931` made the measure task keep the cause and discard an unmeasured blob; `47b0dc2` did the same for request tasks.

**Dependents.** `Store::measure_published` and the request-log write; the row/summary reason is the payload, not `Display`.

**Options.** (a) Keep — payload-preserving and disk/index-consistent. (b) Test through `measure_published` with an injected panicking closure — proves the discard wiring, no production change. (c) Keep the blob after a panic — preserves bytes but leaves an index row that cannot certify them. Failure mode for (c): the directory holds bytes the index says are absent.

**Code/tests/docs.** `src/tasks.rs:40-49`, `src/blobs/mod.rs:539-568`; add (b) if the wiring changes.

**Recommendation.** Keep (a). The asymmetry with K2 is the point: an undecodable body is evidence; an unmeasured one is not certifiable.

**Verification limits.** The real measurement/log-write paths have no portable panic input; cancellation at those joins is undriven.

**Cross-project impact.** None.

### O1 — the reservation through a real socket

**Current.** `body_reservation` clamps an untrusted declared length to the ceiling and to 64 KiB; the buffer grows as bytes arrive (`src/http/fetcher.rs:114-130`). The regression test uses the loopback harness with a declared length far above the ceiling (`:1222`); the pre-fix failure was reproduced with the same reqwest version and a raw listener, but a live CDN header was not sampled.

**History.** `bb43a8b` stopped sizing a body buffer from an untrusted `Content-Length`; a declared length at or above `isize::MAX` previously panicked with capacity overflow before the drain could refuse it.

**Dependents.** Every buffered success body; the file path and `observe` share the same drain. The typed refusal still comes from `drain_capped`.

**Options.** (a) Keep — a bounded initial reservation is enough and the running total is the real ceiling. (b) Reserve nothing — simpler, more reallocations on large legitimate bodies. (c) Trust the header up to the cap — fewer reallocations, but commits up to 64 MiB per request from an unauthenticated header. Failure mode for (c): many concurrent responses reserve the cap at once; for (a): negligible extra copies.

**Code/tests/docs.** `src/http/fetcher.rs:114-130, 160-189`; test `:1222`.

**Recommendation.** Keep (a).

**Verification limits.** No live CDN header sample; the loopback regression is the evidence.

**Cross-project impact.** None.

### O4 — the absolute decompressed ceiling

**Current.** `MAX_DECOMPRESSED_BYTES` is 256 MiB, not configurable (`src/hash/mod.rs:338`). `decompression_limit` takes the declaration capped by it (`:347-351`); both decode paths read `limit + 1` and refuse one byte past (`:372-432`). The reference manifest decodes to ~16.8 MiB and its largest chunk to 3.7 MiB; no measured input approaches the limit. A crafted bomb blob was not run through `gircap verify` end to end.

**History.** `d17fb18` replaced unbounded `zstd::decode_all` with a bounded stream.

**Dependents.** Blob measurement, `verify`'s decode, any future reader of a manifest or chunk. A legitimate larger manifest/chunk would now be refused.

**Options.** (a) Keep the hard limit — a bomb cannot reach the allocator. (b) Make it a flag — an operator can raise it, but a mis-set flag reintroduces the bomb. (c) Raise the default — more headroom with no evidence one is needed. Failure mode for (b): a hostile bundle plus a large flag aborts the process; for (a): a legitimate >256 MiB decode is refused with a clear error.

**Code/tests/docs.** `src/hash/mod.rs:338-432`; unit tests at the helpers both paths call. No code change under (a).

**Recommendation.** Keep (a). The reference corpus is at least an order of magnitude below the limit; add (b) only if a real input needs it.

**Verification limits.** No crafted bomb through `verify`; the limit is unit-tested at the helper.

**Cross-project impact.** None.

### F5 — production error bodies

**Current.** A non-2xx body is counted only, never buffered; the same declared/running-total checks apply, and no outcome is turned into a different error (`src/http/fetcher.rs:191-258`). The default cap is 64 MiB (`src/http/transport.rs:136`). Real 404 bodies in the reference bundle are ~300–400 B XML; no retryable 429/503 production body was sampled, so the practical effect of the cap is unverified.

**History.** `71a1321` capped the error-body drain without losing the status; a lying `503` stays a `Status`, not a `Shape` or `Transport`, so it is not retried.

**Dependents.** `bytes_in` pricing and the recorded status; a cap that stopped early undercounts the body but preserves the status.

**Options.** (a) Keep — a stop, not a buffer, and 64 MiB is far above observed error bodies. (b) Lower the error-body cap — less time/bandwidth on a hostile origin, but may undercount `bytes_in`. (c) Add a separate `--max-error-body-bytes` — one knob per question, more surface. Failure mode for (b): a legitimate large 5xx body is counted short; for (c): two caps to document.

**Code/tests/docs.** `src/http/fetcher.rs:191-258`, `src/http/transport.rs:136`; tests `:1448` and the error-body accounting tests.

**Recommendation.** Keep (a); the cap bounds time and bandwidth without changing the status.

**Verification limits.** No production 429/503 body size sample.

**Cross-project impact.** None.

## 5. Evidence only

### F8 clap typing — which argument types accept non-Unicode

**Current.** Only `--out` is exercised end to end with a non-Unicode value (`tests/cli_contract.rs:175`). Which non-`PathBuf` clap argument types accept non-Unicode values is unverified; `recorded_argv` is lossy for all of them.

**History.** `df366ee` fixed the panic after parse; the test covers the realistic carrier (`PathBuf` flags and `argv[0]`).

**Dependents.** Only the lossy conversion in `recorded_argv`; clap itself already parses through `args_os` (`src/cli/mod.rs:390-396`). `--out`, `--log-file` and `argv[0]` are the realistic non-Unicode carriers.

**Options.** (a) Leave as evidence — the record is lossy but present. (b) Add a table test over each argument type with an `OsString` — proves the clap surface. (c) Change every argument to `OsString`/`PathBuf` — larger CLI change. Failure mode for (b): none production; for (c): every parser and help string changes.

**Code/tests/docs.** `tests/cli_contract.rs:175`, `src/cli/mod.rs:409-416`. No code change.

**Recommendation.** Add (b) only if non-Unicode flag values become a supported workflow.

**Verification limits.** As stated.

**Cross-project impact.** None.

### I4/J1 — real Windows ACLs and the stat/read race

**Current.** The list walk fails on an unreadable directory, entry or `MANIFEST.json` instead of returning a shorter inventory (`src/verify/mod.rs:1147-1153`); the readers are injected so tests can produce the error (`:1385, 1416, 1454`). The J1 share lock was reproduced end to end; the stat/read race was reasoned, not timed. No real Windows ACL-denied directory or `MANIFEST.json` was set up.

**History.** `5d24494` and `0706984` made the walk fail rather than shorten; `43d5977` and `3f042a1` covered the manifest probe.

**Dependents.** `gircap list` and any audit harness that treats the inventory as complete; the bridge does not walk the artifact root. A read failure must be an error, not a shorter list.

**Options.** (a) Keep the injected tests — they cover the control flow without OS privileges. (b) Add a Windows ACL test with `icacls` — proves the real read failure, but needs a privileged/CI-specific setup. (c) Retry on transient errors — hides a real denial. Failure mode for (b): a test that cannot run everywhere; for (c): an incomplete inventory looks complete.

**Code/tests/docs.** No code change. `src/verify/mod.rs:1147-1153, 1385-1470`.

**Recommendation.** Keep (a); add (b) only if CI has a Windows ACL lane.

**Verification limits.** The ACL arms are injected, not real; the race is reasoned.

**Cross-project impact.** None.

### K1/K2 live inputs

**Current.** A live manifest declaring extreme or negative numbers is unmeasured (K1); a live CDN serving a non-frame chunk is unmeasured (K2). The arithmetic saturates (`src/emit/index.rs:437-464`, `src/verify` A20), and an undecodable body is kept with `decode_error` (`src/blobs/mod.rs:461-464`).

**History.** `fc71387` covers K1's arithmetic; `f7a8da2` covers K2's kept evidence.

**Dependents.** K1 feeds admission, `projected` and the summary totals; K2 feeds `decode_error`, `is_usable` and any consumer of the chunk blob.

**Options.** (a) Keep — both paths have a defined answer. (b) Sample a non-frame chunk and a manifest with extreme numbers once, then record the result. (c) Add a hostile-input capture mode — new surface, no current need. Failure mode for (b): none production; for (c): more code to maintain.

**Code/tests/docs.** No code change. `src/blobs/mod.rs:461-464`, `src/emit/index.rs:437-464`.

**Recommendation.** Keep (a); run (b) once if a live anomaly is reported.

**Verification limits.** Both live inputs are unsampled.

**Cross-project impact.** None.

## Shared / cross-project impacts

- **`crates/log`, `crates/tempdir`.** No decision here changes either crate or how `gircap` uses them.
- **`.config/nextest*`.** No new test target or runner setting is required by any item; the test-only items (F8 clap typing, M1, L2, K1 A20, I4/J1) reuse existing targets.
- **Workspace `Cargo.toml` / `Cargo.lock`.** No dependency change: `gircap` keeps only the `girpr-log` path dependency and the `girpr-tempdir` dev-dependency, and adds no `girpr` dependency or invocation.
- **`girpr` artifact bridge.** Only F4 and O2 touch the contract: F4 is a bundle field (additive unless `schema_version` is bumped), O2 is the existing rule that the bridge runs `gircap verify` first (`docs/06-artifact-bridge-spec.md:198-203`; `tests/artifact_bridge*`). No item requires a bridge change if those two keep their current form.
- **`schema_version` / artifact-schema versioning.** F4 and F8 replay are the only possible schema changes. Under the README rule (`README.md:101-109`), a new optional field is additive at version 2; a new document or a required field stales every existing bundle. Prefer additive, and re-capture only if a required document or a version bump is chosen.
- **No-`girpr`-dependency rule.** Every option above stays inside `tools/capture`; none needs the `girpr` crate, its binary, or a new invocation path.
