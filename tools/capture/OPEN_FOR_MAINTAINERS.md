# Open for maintainers — `gircap` (`tools/capture`)

Settled bundle data-preservation decisions on `fix-gaps/gircap`, left here to be researched and implemented. Items that concluded "keep the current behaviour" have been removed; earlier revisions are in git history. Context and pass log: [ADVERSARIAL_REVIEW.md](ADVERSARIAL_REVIEW.md).

**Standing decision (2026-10-29): record more, not less.** The bundle exists to preserve the API's intricacies — every observation, evaluation and failure that could help someone reconstruct the API — even when no current consumer reads it. When an option leaves more of what was seen in the bundle, prefer it. Backwards compatibility is not a requirement: a `schema_version` bump and a re-capture are acceptable costs. The safety rules that keep writes inside the bundle and allocations bounded still apply.

## 1. Blob and fetch evidence

### F4 — a dedicated `fetch_error` field

**Current.** A failed fetch writes `stored_path: null`, `verified: null`, and the failure text in `decode_error` (`src/blobs/mod.rs:420-438`). `BlobProps::unmeasured` is used for `--no-chunk-blobs`, and `unfilled` overwrites `decode_error` with the fetch reason for a fetch that was attempted. `decode_error` is documented as the field that answers "why is this blob unusable", including a fetch that never produced a body (`docs/01-artifact-schema.md:614-644`; `src/blobs/props.rs:377-401, 432-439`).

**History.** `f31850b` kept a failed chunk's slot and its reason instead of losing both; `583aa79` added the panic payload; `f7a8da2` made an undecodable body a kept row rather than a failed fetch. The first pass recorded the reason in a generic row; the field name has not changed.

**Dependents.** `verify` and `selfcheck` read `decode_error` as the unusable reason; `is_usable` treats any non-null value as unusable (`props.rs:426-439`). `tests/schema.rs:300-324` pins the current blob-entry keys. No code reads a `fetch_error` field.

**Decision.** Add a structured `fetch_error` object to every blob row — source tag, requested URL, status or transport failure, attempts, and the detail — and make it required in the version-3 schema bump. `decode_error` stays for bodies that arrived but did not decode. Every writer, schema test and selfcheck row must agree.

**Code/tests/docs.** `src/blobs/props.rs` `BlobProps` (new field), `src/blobs/mod.rs:420-438` (`unfilled` populates it), schema §12, `tests/schema.rs:300-324`. Bump `src/SCHEMA_VERSION` and re-capture `artifacts/`.

**Verification limits.** No external consumer was found that reads `chunks/index.json`; the bridge reads the matrix, patchmaps, blobs and `expected/` (`docs/06-artifact-bridge-spec.md`). The evidence needed here is not a consumer but a failing live request whose status/attempt structure is currently collapsed into one string.

**Cross-project impact.** Bundle contract only. A schema bump reaches `girpr`'s bridge reader (`docs/06-artifact-bridge-spec.md`, `tests/artifact_bridge*`), which must accept the new version; no `girpr` dependency or invocation is introduced.

### F11 — failed-fetch partial bytes and older temp-file orphans

**Current.** Handled fetch/flush/rename errors drop the handle and call `discard_tmp` (`src/blobs/mod.rs:466-474, 492-517`); a post-rename measurement failure calls `discard_published` (`:549-568, 589-608`) — the path K2/N1 retains. A panic unwinds past both temp cleanup and the close-out. Orphans from older binaries are not swept, and the historical accumulation rate is unmeasured. The tmp name carries the slot, so a leftover is reused only if the same chunk lands in the same slot in the same directory.

**History.** `882a53b` removed a failed chunk's temp file instead of orphaning it; `1300931` added the published-blob discard.

**Dependents.** `chunks/blobs/.*.tmp` files are ordinary regular files to the close-out walk: an old orphan is listed in `MANIFEST.json → files[]` and `verify/checksums.json` and passes A16, so it silently inflates the bundle instead of becoming a finding. Under the decision below, a failed fetch's bytes become an indexed partial row instead, and `is_usable`/A6 must reject it. No reader distinguishes an old orphan from an intended file.

**Decision.** Publish the partial bytes of a failed fetch as an indexed, unusable row: record `fetch_error`, `partial: true` and what arrived, and remove only a zero-byte temp. Keep the cap. A partial body must never look like a complete chunk, so `is_usable` and A6 must reject it. Old unindexed `.*.tmp` files from earlier binaries still need a manual sweep.

**Code/tests/docs.** `src/blobs/mod.rs:492-517` (`stream_from` currently drops the temp on fetch failure), `src/blobs/props.rs` (`partial`/`fetch_error`), schema §12, `src/SCHEMA_VERSION`. Add a test with a truncated response that asserts the partial row and bytes survive, and one with a connection-refused fetch that leaves nothing.

**Verification limits.** No panic-orphan accumulation was measured; cleanup is tested for handled errors only, and no truncated-response fixture is retained today.

**Cross-project impact.** New fields and a schema bump; the bridge must treat `partial`/`fetch_error` rows as unusable, not as fetchable chunks. No dependency change.

### K2 — the removal arm after a failed measurement

**Current.** An undecodable body is kept: bytes are on disk before measurement, `decode_error` carries the reason, and the row points at the bytes (`src/blobs/mod.rs:461-464`; schema §12). A post-rename measurement I/O error, panic or cancellation removes the published blob (`measure_published`, `src/blobs/mod.rs:549-568`; `discard_published`, `:589-608`) and, unless a later source fills the slot, the row becomes `stored_path: null` with the reason. No test forces the I/O failure; the undecodable path is the tested post-rename path.

**History.** `f7a8da2` kept an undecodable body as evidence; `1300931` added the published-blob discard for a measurement that did not return.

**Dependents.** `is_usable` requires `stored_path`, `verified` all-green and no `decode_error` (`src/blobs/props.rs:426-439`); under the decision below it must also require `measurement_error` to be null. An undecodable body is evidence; a body whose measurement failed is still evidence, just not a certifiable one.

**Decision.** Keep every fetched blob, including one whose post-rename measurement failed: leave `stored_path` set, keep the raw size and compressed digests taken during the write, and record `measurement_error` with the I/O/panic/cancel reason. `is_usable` stays false and must require `measurement_error` to be null. `discard_published` becomes unnecessary for this arm.

**Code/tests/docs.** `src/blobs/mod.rs:549-568` (`measure_published` stops discarding), `:589-608` (`discard_published` becomes unnecessary for this arm), `src/blobs/props.rs` (`measurement_error`, `is_usable`), schema §12, `src/SCHEMA_VERSION`. Add a test that injects an I/O error and asserts both the row and the blob survive.

**Verification limits.** The removal arm is exercised by the panic test at the same join (`src/tasks.rs:81`) but not by an I/O error.

**Cross-project impact.** New bundle field and schema bump; the bridge may see a blob row with `stored_path` set and `verified: null`, so it must rely on `is_usable`/`measurement_error`, not `stored_path` alone.

### N1 — panic seams and the published-blob discard

**Current.** Both direct `spawn_blocking` joins share `join_blocking` (`src/tasks.rs:40-49`); the panic mapping and discard are driven with a panicking closure at the same seam production uses (`src/tasks.rs:81`). No portable input makes `BlobProps::measure_file` or `emit::write_requests_log` panic; cancellation at those joins was not driven. A measure task that panics removes the already-published blob and the row says `stored_path: null`.

**History.** `1300931` made the measure task keep the cause and discard an unmeasured blob; `47b0dc2` did the same for request tasks.

**Dependents.** `Store::measure_published` and the request-log write; the row/summary reason is the payload, not `Display`. The row also carries `measurement_error`, and `is_usable` must read it.

**Decision.** On a panic or cancellation at the measure join, keep the published blob, record the payload in `measurement_error`, and leave `stored_path` set. `is_usable` must reject the row on `measurement_error`, not on `stored_path`, so a reader can tell "measurement failed" from "not attempted".

**Code/tests/docs.** `src/tasks.rs:40-49`, `src/blobs/mod.rs:539-568`, `src/blobs/props.rs` (`measurement_error`), schema §12; add a test through `measure_published` with an injected panicking closure that asserts the blob survives and the row says why.

**Verification limits.** The real measurement/log-write paths have no portable panic input; cancellation at those joins is undriven.

**Cross-project impact.** Same new field and schema bump as K2; no bridge dependency.

### F5 — production error bodies

**Current.** A non-2xx body is counted only, never buffered; the same declared/running-total checks apply, and no outcome is turned into a different error (`src/http/fetcher.rs:191-258`). The default cap is 64 MiB (`src/http/transport.rs:136`). Real 404 bodies in the reference bundle are ~300–400 B XML; no retryable 429/503 production body was sampled, so the practical effect of the cap is unverified.

**History.** `71a1321` capped the error-body drain without losing the status; a lying `503` stays a `Status`, not a `Shape` or `Transport`, so it is not retried.

**Dependents.** `bytes_in` pricing and the recorded status; a cap that stopped early undercounts the body but preserves the status.

**Decision.** Record incidental non-2xx response bodies as bundle content: status, content-type, URL and the bytes received, bounded by the same cap (escaped in a JSON document, or per-response files with an index). The cap stays a stop, and a per-run total budget remains necessary. `api/error_shapes.json` is the precedent for byte-exact error bodies.

**Code/tests/docs.** `src/http/fetcher.rs:191-258` (`drain_error_body` collects the body instead of only counting it), `src/run/mod.rs`/`src/emit` (write the document), `docs/01-artifact-schema.md`, `src/SCHEMA_VERSION`, tests around `:1448` and the error-body accounting tests.

**Verification limits.** No production 429/503 body size sample, so the practical shape of those bodies remains unknown; the mechanism can be tested against mock bodies.

**Cross-project impact.** New document(s) and a schema bump; the bridge does not read them, so no dependency or invocation change.

### O4 — the absolute decompressed ceiling

**Current.** `MAX_DECOMPRESSED_BYTES` is 256 MiB, not configurable (`src/hash/mod.rs:338`). `decompression_limit` takes the declaration capped by it (`:347-351`); both decode paths read `limit + 1` and refuse one byte past (`:372-432`). The reference manifest decodes to ~16.8 MiB and its largest chunk to 3.7 MiB; no measured input approaches the limit. A crafted bomb blob was not run through `gircap verify` end to end.

**History.** `d17fb18` replaced unbounded `zstd::decode_all` with a bounded stream.

**Dependents.** Blob measurement, `verify`'s decode, any future reader of a manifest or chunk. A legitimate larger manifest/chunk would now be refused.

**Decision.** Replace the hard-coded 256 MiB ceiling with `--max-decompressed-bytes` (the default may stay 256 MiB) and record the effective value in the bundle, so a decode is reproducible. Keep the running-total guard.

**Code/tests/docs.** `src/hash/mod.rs:338-432` (the constant becomes a parameter), `src/cli/mod.rs` (new flag), `src/config/mod.rs` (validation), schema (record the effective limit), `src/SCHEMA_VERSION`. Tests at the helpers both paths call.

**Verification limits.** No crafted bomb through `verify`; the reference manifest is far below 256 MiB. No legitimate input above the current limit was sampled.

**Cross-project impact.** New flag and a bundle field/schema bump; no dependency or bridge path change.

## 2. Run provenance and bundle shapes

### F8 — replay of the recorded argv

**Current.** `capture.tool_argv` is recorded from `args_os()` through `to_string_lossy` (`src/cli/mod.rs:398-416`, written at `src/emit/index.rs:130`). A non-Unicode argument becomes U+FFFD in the record. The differ treats `tool_argv` as a volatile field, not a replay input (`docs/04-bundle-differ.md:64, 299`).

**History.** `df366ee` replaced `std::env::args()` after parsing; that call panicked on non-Unicode arguments, exited `101` and printed no `SUMMARY`. The lossy record is the fix, not an oversight.

**Dependents.** `tests/schema.rs:246` requires `tool_argv`; `tests/offline_bundle.rs:355` constructs it; `tests/cli_contract.rs:175` exercises a non-Unicode `--out`. No replay command exists.

**Decision.** Keep `tool_argv` as the human-readable rendering and add `tool_argv_raw` carrying the exact `OsString` bytes per argument (base64 or an array of byte arrays), required in the version-3 schema. Replay need not exist yet; the lossy form cannot be reconstructed later.

**Code/tests/docs.** `src/cli/mod.rs:409-416` (return both forms), `src/emit/index.rs:130`, `docs/01-artifact-schema.md:101-107`, `tests/schema.rs:246`, `tests/cli_contract.rs:175`. Update `src/SCHEMA_VERSION` and re-capture.

**Verification limits.** No non-Unicode argument other than `--out` is exercised end to end; which clap argument types accept non-Unicode values is F8 clap typing below.

**Cross-project impact.** New bundle field in the version-3 schema; the bridge does not read `capture.tool_argv`, so no consumer change beyond accepting the version.

### F8 clap typing — which argument types accept non-Unicode

**Current.** Only `--out` is exercised end to end with a non-Unicode value (`tests/cli_contract.rs:175`). Which non-`PathBuf` clap argument types accept non-Unicode values is unverified; `recorded_argv` is lossy for all of them.

**History.** `df366ee` fixed the panic after parse; the test covers the realistic carrier (`PathBuf` flags and `argv[0]`).

**Dependents.** Only the lossy conversion in `recorded_argv`; clap itself already parses through `args_os` (`src/cli/mod.rs:390-396`). `--out`, `--log-file` and `argv[0]` are the realistic non-Unicode carriers.

**Decision.** Add a table test over each argument type with an `OsString`, alongside the F8 replay field, so the lossy conversion is pinned across the whole clap surface and not only `--out`. No production CLI change.

**Code/tests/docs.** `tests/cli_contract.rs:175`, `src/cli/mod.rs:409-416`.

**Verification limits.** As stated.

**Cross-project impact.** None.

### H4 — a real `--no-chunk-blobs` bundle

**Current.** `--no-chunk-blobs` rows come from `BlobProps::unmeasured` (`src/blobs/props.rs:377-401`): `stored_path`, `verified` and `decode_error` are null, `bytes` is 0, compressed digests are empty strings, and `md5_decompressed`/`uncompressed_size` are the manifest's declared values. The wire-byte ceiling is skipped (`src/blobs/plan.rs:122-140`); A6–A12, A19 and A21–A23 are skipped (`src/emit/closeout.rs:175-184`; schema §12/§16). No real `--no-chunk-blobs` bundle has been inspected; the doc matches the constructor and the tests.

**History.** `c9bf161` corrected the docs to say what an unmeasured row carries; `f7a8da2` is the different case of an attempted fetch that arrived but did not decode.

**Dependents.** `verify` reports `verified: null` as "not measured"; a reader must treat the row as a declared gap. The bridge is not a consumer: it needs chunk blobs and `expected/` bytes.

**Decision.** Capture one real `--no-chunk-blobs` bundle and keep it as a second bundle shape/fixture. It proves what `BlobProps::unmeasured` actually writes and is not a bridge fixture.

**Code/tests/docs.** `src/blobs/props.rs:377`, `src/emit/closeout.rs:175-184`, `docs/01-artifact-schema.md` §12/§16.

**Verification limits.** No on-disk metadata-only bundle, and no `verify` run over one.

**Cross-project impact.** None; the bridge requires chunk blobs, so a `--no-chunk-blobs` bundle is outside its contract.

## 3. Refused and anomalous input

### F6/G1 — live hostile inputs

**Current.** One shared shape rule refuses empty/control/backslash/absolute/drive/`..`/`.`/trailing-dot-or-space/device/empty components (`src/paths.rs:34-89`); the tag rule adds Windows-invalid characters, length, leading/trailing dot and device names (`src/config/mod.rs:581-627`); the writer re-checks field and tag before the first `create_dir_all` (`src/manifest/dump.rs:80-99`); chunk ids must be `<16 hex>_<32 hex>` (`src/paths.rs:142-155`). Local sinks and rejection paths are tested.

**History.** `efc5322` closed separators/prefixes in tags; `b6aeb6d` device names as paths; `c9caa02` Windows-folded names; `0b55a40` audio fields; `88d5413`/`0a21ebe` the verify side.

**Dependents.** `--files`, manifest paths, `chunks/index.json` `stored_path`/`referenced_by`, `patchmaps/*.json` paths, audio codes, tags and chunk URLs all use these rules. A live API that echoes a hostile tag, serves a hostile document/audio path, or uses an unusual-but-legitimate chunk-id spelling is unverified.

**Decision.** Keep the strict write rules; add `diagnostics/rejections.json` recording every refused value, its source and the rule it failed. Never write the hostile value, but keep it in the run's diagnostics and in the bundle whenever a bundle is still written. Untrusted strings need escaping and a cap.

**Code/tests/docs.** `src/paths.rs:34-159`, `src/config/mod.rs:452-470, 581-627`, `src/manifest/dump.rs:80-99`, schema §12, `docs/00-spec.md` §7. Add the `diagnostics/rejections.json` writer and schema entry; call sites already have the value and the problem string.

**Verification limits.** No live manifest/tag/audio path outside the rule was sampled; only local sinks and unit tests cover the rejection.

**Cross-project impact.** A new document and a schema bump; the bridge ignores it, and no `girpr` dependency is introduced.

### L1 — device names end to end and across hosts

**Current.** A device-name tag is refused at argument resolution before any request (`src/config/mod.rs:618-622`); a device-name path is refused by the shared rule (`src/paths.rs:103-112`); `verify` refuses a bundle document that names one (`src/verify/mod.rs:305-337`). A16's missing-file consequence is reasoned, not executed. No capture was run with a device-name tag, and a Linux-produced `manifests/CON/` bundle was not replayed through `verify` on Windows.

**History.** `63ba129` added device names to the tag rule; `b6aeb6d` to paths; A16 already treats a listed-but-missing file as a finding.

**Dependents.** Tags become `manifests/<tag>`; paths become blob/capture names; a bundle can move between hosts, so a rule enforced only on Windows would not protect a Linux reader.

**Decision.** Keep refusing a device-name value that appears at argument resolution (there is no bundle yet). If a live manifest or API tag actually contains a device name, preserve the original value in the documents under an escaped on-disk name and record the mapping, rather than discarding the observation. A cross-host replay test is needed only if bundles cross hosts.

**Code/tests/docs.** `src/config/mod.rs:618`, `src/paths.rs:103`, `src/verify/mod.rs:305`; test `src/manifest/dump.rs:418`. Under the mapping, `archive_rel`/`tag_dir`/blob `stored_path` and the schema need it; the replay test goes under `tests/`.

**Verification limits.** No device-name capture and no cross-host replay; A16's consequence is reasoned.

**Cross-project impact.** A bundle path-mapping document and a schema bump; the bridge must follow the mapping; no dependency change.

### H3 — live manifests with Windows-folded names

**Current.** A component ending in `.` or a space is refused (`src/paths.rs:68-76`); an audio field must be one file-name component (`src/paths.rs:122-132`; `src/config/mod.rs:452-470`). Whether a live manifest contains names differing only by a trailing dot or space was not sampled.

**History.** `c9caa02` added the folding rule; `0b55a40` extended the field rule to audio.

**Dependents.** Two manifest entries that fold onto one file would make the last write win and the bundle list bytes it does not hold. The rule runs on every manifest path, not only on `--files`.

**Decision.** Keep refusing the ambiguous write; collect every offending name and record it with its field and rule in the rejection document, then refuse. The name is evidence about the API even though it cannot be written as a path.

**Code/tests/docs.** `src/paths.rs:68-76, 122-132`; `src/config/mod.rs:452-470`; tests in `src/paths.rs`. Use the rejection writer from F6/G1.

**Verification limits.** No live manifest sample; only synthetic unit tests.

**Cross-project impact.** The diagnostics document and schema bump above; no bridge or dependency change.

### K1/K2 live inputs

**Current.** A live manifest declaring extreme or negative numbers is unmeasured (K1); a live CDN serving a non-frame chunk is unmeasured (K2). The arithmetic saturates (`src/emit/index.rs:437-464`, `src/verify` A20), and an undecodable body is kept with `decode_error` (`src/blobs/mod.rs:461-464`).

**History.** `fc71387` covers K1's arithmetic; `f7a8da2` covers K2's kept evidence.

**Dependents.** K1 feeds admission, `projected` and the summary totals; K2 feeds `decode_error`, `is_usable` and any consumer of the chunk blob.

**Decision.** Sample a non-frame chunk and a manifest with extreme or negative numbers once against a controlled origin, and keep the result as a fixture. This is exactly the API intricacy the bundle preserves, even if only one sample is obtained.

**Code/tests/docs.** `src/blobs/mod.rs:461-464`, `src/emit/index.rs:437-464`.

**Verification limits.** Both live inputs are unsampled.

**Cross-project impact.** None.

## Shared / cross-project impacts

- **`crates/log`, `crates/tempdir`.** No decision here changes either crate or how `gircap` uses them.
- **`.config/nextest*`.** No new runner setting is required. The data additions add tests to existing targets; F8 clap typing adds cases, not a new binary.
- **Workspace `Cargo.toml` / `Cargo.lock`.** No dependency change: `gircap` keeps only the `girpr-log` path dependency and the `girpr-tempdir` dev-dependency, and adds no `girpr` dependency or invocation.
- **`girpr` artifact bridge.** F4, F8 replay, F5, F11, K2/N1, O4, the F6/G1 rejection document and L1's mapping all add bundle fields or documents. The bridge must accept the new `schema_version`; it does not need to read the new data, and it keeps `gircap verify` as its precondition (`docs/06-artifact-bridge-spec.md:198-203`; `tests/artifact_bridge*`).
- **`schema_version` / artifact-schema versioning.** Make the data additions required under one bump to `3` rather than scattering optional fields across version 2. Backwards compatibility is explicitly not required, so an existing bundle is re-captured when it must be verified or consumed; a reader seeing version 2 refuses it and says so, as it already does for version 1.
- **No-`girpr`-dependency rule.** Every option above stays inside `tools/capture`; none needs the `girpr` crate, its binary, or a new invocation path.
