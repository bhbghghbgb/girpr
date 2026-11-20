# Shared-crate refactor research: assumptions and findings

Static research over the workspace at `62a51126`. No source was changed, no build or
test was run, and no line was executed. Every item below is a claim to be validated
during implementation; line numbers are from the current tree and may drift.

Items are labelled `A` (assumptions), `F` (findings) and `N` (negative findings:
duplicate-looking code that is *not* an extraction candidate). "Candidate boundary"
describes where the duplication sits; it is not an implementation ordering.

## Assumptions

**A1 — Established crate convention.** Shared code lives in `crates/`, is named
`girpr-<policy>`, carries a one-line `description = "The shared … policy …"`, opts
into `[lints] workspace = true`, and holds policy/mechanics rather than program
logic. Precedents: `crates/log`, `crates/filter`, `crates/tempdir` and the extraction
commits `51b3f4d2` (log), `17dfc989` (tempdir), `aaa7659a` (filter), `382f992f`
(timestamp), `f9b6914e` (test fixtures), `4319a10f` (sled2redb). Source:
`crates/*/Cargo.toml`, `tools/capture/docs/00-spec.md`, `tools/sled2redb/README.md`.

**A2 — gircap independence is a hard boundary.** `tools/capture/docs/00-spec.md:34-48`
(§2 "Independence (non-negotiable)") forbids a dependency on `girpr`, forbids invoking
its binary, and permits only leaf policy crates that hold "no logic of any program".
`README.md:389-391`, `tools/sync/README.md:8-10`, `tools/capture/README.md:3-4` and
`docs/06-artifact-bridge-spec.md:707-709` repeat it. The spec's table currently names
**two** shared leaf crates; adding any third leaf dep requires updating that table.
The spec itself notes "None of the above is mechanically enforced".

**A3 — Byte contracts are load-bearing.** Capture bundles must stay byte-identical
(`tools/capture/docs/03-design-notes.md:92-103,134-150,308-312`,
`tools/capture/docs/04-bundle-differ.md:33-35`; `bundle_diff.rs` compares bytes after
normalising only known-nondeterministic fields). Product `SUMMARY` emits each key only
when its flag is active (`README.md:122-135`). Sync caches and unflagged/dry-run runs
have their own byte-identity gates (`tools/sync/README.md:691,748,1255-1259`). An
extraction that changes key order, casing, spacing or flag conditions fails these.

**A4 — Lint policy is narrower than it is often described.** Root `Cargo.toml:26-38`:
`unsafe_code = "deny"`, `clippy::all = "warn"`, `unwrap_used = "warn"`,
`expect_used = "warn"` — the unwrap/expect pair is *warned*, not denied, and CI may add
`-D warnings` (`README.md:475-491`). Dev tools keep only `unsafe_code = "deny"` and
`clippy::all = "warn"` (`tools/sync/Cargo.toml:8-17`, `tools/capture/Cargo.toml:17-26`,
`tools/sled2redb/Cargo.toml:8-15`). The single deliberate `unsafe` is the Windows handle
call in `tools/sync/src/hardlink.rs` (`README.md:499-501`).

**A5 — Test mechanics constrain helper extraction.** nextest uses one process per test
and does not run doctests (`README.md:503-517`), so shared test helpers must be public
or re-exported; this is the stated reason one `logs_of` copy exists
(`tests/common/mod.rs:839-842`). The precedent is a thin `tests/common/mod.rs` shim over
one implementation (`crates/tempdir/src/test_support.rs`; `tools/sync/tests/common/mod.rs:1-14`,
`tools/sled2redb/tests/common/mod.rs:1-13`).

**A6 — Divergent policies are intentional, not bugs to fix during extraction.**
Product retry uses ±50 % jitter (`src/retry.rs:41-49`); capture deliberately uses
`randomization_factor = 0.0` for byte-stable artifacts (`tools/capture/Cargo.toml:41-46`).
Product `game_version` is last-match/no-section (`src/util/ini.rs:10-28`); capture is
first-match/`[General]`-only via `ini_core` (`tools/capture/src/emit/gamedir.rs:75-112`,
`tools/capture/Cargo.toml:88-92`). Exit tables, report formats and flag sets are
deliberately independent (`tools/capture/src/exit/mod.rs:3-18`). `DO-NOT-TOUCH` guidance
in the deleted planning tree (recoverable at `bec516b6^:REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`)
says a session finding a "keep" item should be changed must stop and raise a follow-up.

**A7 — There is no single "digest" semantics.** girpr parses a strict 16-byte `Md5`
newtype where empty means "unknown, not a match" (`src/util/md5.rs:20-24,39-64`);
girsync's `DigestSet` stores raw bytes plus verbatim foreign names for forward
compatibility (`tools/sync/src/hash/digestset.rs:51-117`, `cache/wire.rs:18-20`);
gircap publishes lowercase hex strings (`tools/capture/src/hash/mod.rs:31-43`). A shared
type must not collapse these.

**A8 — Report planes cannot be merged verbatim.** JSON key order differs three ways:
girpr event-first insertion order (`src/report/record.rs:177-217`), girsync alphabetical
(`tools/sync/src/report/record.rs:239-264`), gircap event-first with a real `event` field
(`tools/capture/src/report/mod.rs:337-367`). `preserve_order` is rejected as a
"workspace-wide feature-unification hazard" (`tools/capture/docs/03-design-notes.md:66`).
Booleans are forbidden in girpr's data plane (flags render as `1`/`0`,
`src/report/record.rs:60-65`) but present in capture; capture percent-encodes text values,
the others do not.

**A9 — Exit codes are separate contracts.** girpr `RunExit` 0–5 (`src/config.rs:132-164`),
gircap `ExitCode` 0–4 (`tools/capture/src/exit/mod.rs:25-101`), girsync/sled2redb literals
with different meanings (`tools/sync/README.md:1481-1486`); capture documents the 1/2
overlap as coincidence. Only the *mechanism* for generating a table is shareable.

**A10 — The device-name divergence is a defect candidate, not yet a verdict.** The
product predicate includes `CLOCK$` and trims trailing dot/space before comparing
(`src/util/path.rs:303-321`); the capture predicate omits `CLOCK$` and does not trim
(`tools/capture/src/paths.rs:152-161`). Whether this is a bug or an intentional
bundle-format allowance must be confirmed before changing it: altering the predicate
can move bundle bytes.

**A11 — Not every existing "leaf" is program-independent.** `crates/tempdir/Cargo.toml:20-24`
declares a non-dev dependency on `girsync` and `serde_json` for `test_support`
(`crates/tempdir/src/lib.rs:70-81`). The deleted report flagged this as open assumption
§A10/§I14. A refactor that relies on "leaf with no program dependency" must account for
this test-only inversion.

**A12 — Prior duplication catalogue exists outside the tree.** `docs/08-hand-rolled-code-reduction-findings.md`
(858 lines) and `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` were deleted in `bec516b6` but are
recoverable from git. They explicitly catalogue JSON rendering, retry, hex, INI, HTTP
mocks, directory walkers, canonicalization, glob filtering, tracing-appender, tempdir,
timestamps and URL/status helpers, and list twelve "hand-rolled code that should stay"
items. Conventions from the deleted tree require no lingering references to those files.

## Findings

### Baseline: what is already shared

**F1 — Sharing today is policy-only.** `girpr-log` owns `Level`, `parse_level`,
`filter`/`console_filter`, `LogOptions`/`EnvOptions`/`LogPlan` and `resolve`, plus
`default_file_is_written`, `default_log_path` and `timestamp`
(`crates/log/src/{level.rs:20-82,filter.rs:20-66,plan.rs:102-366,lib.rs:69-72}`). It
explicitly does *not* own the subscriber implementation or the file format
(`crates/log/src/lib.rs:19-26`, `plan.rs:87-93`). `girpr-filter` owns glob rules;
`girpr-tempdir` owns scratch directories. All duplication reported below is *below* the
policy line, in implementation and mechanics. Confidence: high.

### Digest, hex and streaming hash

**F2 — Hex digest spelling is repeated at eight-plus sites, all over `hex = 0.4.3`.**
`src/util/md5.rs:39-74` (`parse_hex`, `to_hex`, `Display`), `tools/capture/src/hash/mod.rs:30-36,223-228,500,571`,
`tools/sled2redb/src/convert.rs:97-99`, `tests/common/digest.rs:5-8`,
`tools/sled2redb/tests/convert.rs:31`. Parsing policies differ (strict 32-digit vs
decode-with-context), but the primitive is the same. Candidate boundary: an encode/decode
layer in a leaf `girpr-digest`. Confidence: high; low value per site.

**F3 — "Hash bytes as they pass through" sinks are structurally identical.**
`src/util/md5.rs:365,397-402` (reader → MD5, 512 KiB buffer, seek to start),
`src/sophon/stream.rs:189-245` (`HashingWriter<W>`, MD5 + byte ceiling, hashes only
accepted bytes), `tools/capture/src/hash/mod.rs:72-216` (`DualDigest` MD5+SHA-256 `Write`
sink, `HashingFile` with mandatory `restart` on retry), `tools/capture/src/emit/index.rs:225-253`
(`digest_file` reader-side with 64 KiB cap), `tools/sync/src/hash/stream.rs:26-81`
(optional MD5/SHA by `AlgoSet`, raw-byte result). Candidate boundary: a shared
`Write`/reader sink with caller-supplied buffer size, algorithm set and reset contract.
The ordering policies (limit-before-forward, write-before-hash, restart-on-retry) stay
with each owner — a naive shared writer would produce plausible-but-wrong digests
(`tools/capture/src/hash/mod.rs:173-185`, `tools/capture/src/http/fetcher.rs:737-748`).
Confidence: high.

**F4 — The `Md5` newtype has a cross-crate mirror that had to be fixed by hand.**
Capture lowercases digest strings at the decode boundary specifically "to match girpr"
(`tools/capture/src/manifest/proto.rs:127-183`), and compares with
`eq_ignore_ascii_case` (`tools/capture/src/manifest/mod.rs:466-473`). Candidate boundary:
shared `Md5([u8;16])`/`Sha256([u8;32])` newtypes with `EMPTY`, `parse_hex`, `to_hex`,
`matches_hex`, `Display`; girpr keeps `Scan`/`read_range`, capture keeps its string
publish shape, girsync keeps `DigestSet`/`AlgoSet` (N9). Confidence: high.

**F5 — Digest test oracles are duplicated.** `EMPTY_MD5` (`src/util/md5.rs:15`) is
spelled literally in `tools/capture/src/hash/tests.rs:472`, `tools/sled2redb/tests/convert.rs:180`
and product fixtures; RFC 1321 vectors in `src/util/md5.rs:474-501` overlap the published
vectors in `tools/capture/src/hash/tests.rs:526-545`. `md5hex` exists in
`tests/common/digest.rs` and `gircap::hash::md5_hex`, with a third spelling in
`tools/sync/tests/helpers.rs:16,178`. Confidence: high.

### Windows path names, path keys, canonicalization

**F6 — Windows device-name predicate is duplicated and has drifted.** Exact evidence:
product `src/util/path.rs:303-321` reserves `CON PRN AUX NUL CONIN$ CONOUT$ CLOCK$ COM1-9
LPT1-9` and trims trailing ` ` / `.` before an ASCII-case-insensitive compare;
capture `tools/capture/src/paths.rs:152-161` reserves the same set **minus `CLOCK$`** and
uses `to_ascii_uppercase()` on the raw stem. Capture's own module doc says the rule is
kept in one place so it cannot drift (`tools/capture/src/paths.rs:3-10`).
`contained_rel` applies the product rule to every manifest path, so a bundle component
named `CLOCK$` could be accepted by capture and refused by girpr. Candidate boundary:
a `girpr-fs-names` leaf of pure predicates. See A10. Confidence: high on the textual
difference; the behavioural consequence is an assumption to validate.

**F7 — Trailing-dot/space and ASCII-fold rules are duplicated with deliberate host
policy.** `has_windows_trailing_dot_or_space` / `any_component_has_windows_trailing_dot_or_space`
(`src/util/path.rs:272-296`) vs `path_rule` (`tools/capture/src/paths.rs:67-138`); product
normalises `\` and `.` components (`src/util/path.rs:89-134`), capture refuses both.
`path_key` folds case only on Windows in girpr (`src/util/path.rs:222-261`) but
unconditionally in girsync (`tools/sync/src/util.rs:428-430,453-504`) because case-insensitive
pairing is a user option there. `slash_rel` preserves `\` on POSIX in girsync
(`tools/sync/src/util.rs:77-89`) but product rewrites it (`src/util/path.rs:175-220`).
Candidate boundary: shared predicates plus an explicit host-policy parameter; the two
`path_key` behaviours must not be merged. Confidence: high.

**F8 — Canonicalize + `\\?\` stripping is repeated.** girsync calls `dunce` in
`plan/safety.rs:76,355`, `commands/sync/mod.rs:667`, `apply/copy.rs:131`,
`apply/phases.rs:715`, `plan/link.rs:260`; capture in `verify/list.rs:111,234` and
`verify/bundle.rs:422-427`; product has a fail-closed longest-existing-prefix routine in
`src/util/path.rs:450-477`. The deleted report's §A7 lists this as acknowledged
duplication. Candidate boundary: a `canonical_no_verbatim` / `canonical_existing_prefix`
pair in a path leaf. Confidence: high.

**F9 — File identity and hard-link counts are two near-copies.**
`tools/sync/src/hardlink.rs:62-155` (`link_count`, `identity_from_path`, `info`, `identity_of`,
`FileId = same_file::Handle`, with the one deliberate `unsafe` Windows handle call) and
`src/repair/lock.rs:311-365` (`link_count`, `same_file` comparing `dev`/`ino` or volume
serial/file index with `FILE_FLAG_OPEN_REPARSE_POINT`, fail-closed `Option<bool>`).
`tools/sync/src/cache/backup/publish.rs:338,626-662` is a consumer of the girsync side.
Both derive the same platform facts; the representations and error types differ.
Candidate boundary: an fs-identity leaf exposing both from-path and from-handle variants.
Confidence: high.

**F10 — Atomic temp-then-publish exists in four shapes.** `src/util/atomic.rs:30-167`
(`write_atomic_with`: `NamedTempFile::new_in`, `BufWriter`, optional `sync_all`,
`persist`, best-effort parent sync), `src/repair/phases/post.rs:477-546`
(`check_publish_target` + `copy_publish_with`: a second `NamedTempFile`+`persist`, with an
injected copy closure, no parent creation, no sync, `io::Result`), `tools/sled2redb/src/convert.rs:276-329`
(`Builder::tempfile_in` + `persist`/`persist_noclobber`), `tools/sync/src/cache/backup/publish.rs:50,135-145,190-248,301-445,509-517,884-917`
(working name + sync + hard-link claim, falling back to rename). Capture's named
`.slot.tmp` blobs (`tools/capture/src/blobs/mod.rs:603-610,653-716`) and deliberately
non-atomic small docs (`tools/capture/src/emit/mod.rs:356-419`) are not candidates.
Candidate boundary: an fs-publish leaf with explicit `durable`, parent-creation,
replace-vs-noclobber and error-type parameters. Confidence: high.

**F11 — Link/junction/symlink detection and creation are duplicated three ways.**
Detection: `tools/sync/src/link.rs:53-89` (`try_is_link`, `is_link_metadata`) vs
`crates/tempdir/src/test_support.rs:227-250` (`is_link`, whose doc calls itself "the
test-side mirror of `girsync::link::try_is_link`"). Creation: product
`src/util/junction_support.rs:78-100,116-163,191-238` and `src/util/symlink_support.rs:35-52,169-220`,
`crates/tempdir/src/test_support.rs:108-194`, and an inline copy in
`tools/capture/src/verify/tests.rs:8-12`. Product deliberately keeps these
unit-test-only (`src/util/mod.rs:25-36`) and refuses `%` in junction paths because `cmd`
expands it (`src/util/junction_support.rs:110-163`) — extraction to tempdir's test support
would reverse that decision. Confidence: high.

**F12 — File locks share only a narrow primitive.** `src/repair/lock.rs:45-219,236-392`
(keyed `{temp}/girpr-{hash}.lock`, pid/body, `create_new` retries, symlink/hard-link
refusal, keeps leftover file) vs `tools/sync/src/cache/lock.rs:29-73` (sidecar
`<cache>-lock`, empty, shared/exclusive, degrade-to-no-lock). Capture fixture locking is a
third dev-only user of `fs2` (`tools/capture/tests/common/mod.rs:272-368`). Only the
`try_lock` guard plus error classification is common; policy divergence makes a crate low
value. Confidence: high.

**F13 — Directory walkers are not a single extraction, but their helpers are.**
`tools/sync/src/scan.rs:21,87-254` (follow links, prune caches, heartbeat),
`src/repair/purge.rs:157-204` (no follow, count errors), `src/repair/phases/post.rs:629-645`
(file-only), `tools/capture/src/verify/list.rs:103-234` (follow, sorted, canonical visited set).
Their policies are incompatible; the reusable pieces (`mtime_ns_of`, `is_cache_rel`,
`slash_rel`, `try_is_link`) are the subjects of F7/F9/F11. Confidence: high.

**F14 — `rw` concurrency control is deliberately not shared.** `tools/sync/src/rw/{gate.rs,limits.rs}`
(tokio semaphore + rayon, documented src→dst ordering, reverse-order permit drop) has no
product counterpart: `src/repair/file` bounds files with chunks sequential per file.
`tools/sync/src/rw/mod.rs:22-29` and `src/repair/scope/limits.rs:169-170` say so.
See N6. Confidence: high.

### Retry and backoff

**F15 — Two attempt loops plus two zero-wait test modes.** `src/retry.rs:28-208`
(`BackoffPolicy::real` 1 s/×2/±50 %/30 s, `none`, `retry`/`retry_state`, manual attempt
cap) vs `tools/capture/src/http/transport.rs:10-150` (`RetryPolicy` 500 ms/×2/no
jitter/10 s, `Sleep`/`RealSleep`/`ZeroSleep`/`RecordingSleep`) and the hand-rolled loop in
`tools/capture/src/http/fetcher.rs:789-862`. Both use `backoff` 0.4 (product with the
`tokio` feature, capture without). Candidate boundary: a `girpr-retry` leaf owning the
policy/schedule factory and a hook-based sleep driver; the side-effectful capture loop
(logs every attempt into `RequestLog`, keeps error bodies) should stay local. The deleted
report's §A2 acknowledges the pair. Confidence: high; see A6.

### Test infrastructure

**F16 — `logs_of` exists in three copies.** `src/repair/test_support.rs:12-37` (sync),
`tests/common/mod.rs:843-883` (async, with `RuntimeFlavor::CurrentThread` debug assert —
the comment at `:839-842` explains the duplication), `src/repair/phases/metadata.rs:557-594`
(the same async copy, caller at `:1016`); plus `tools/sync/tests/common/mod.rs:137-173`
(`SharedBuffer` `MakeWriter`). A shared dev-only `log_capture` module would remove them;
this is blocked today by A5 (the helper is not public). Confidence: high.

**F17 — The consolidation precedent is strong and recent.** `crates/tempdir/src/test_support.rs`
is a 725-line module moved from two byte-identical 27,348-byte `tests/common/mod.rs` files
(`f9b6914e`), leaving 9- and 13-line re-export shims. Remaining generic duplication:
`md5hex` (F5), `fs2` cross-process fixture locks
(`tests/common/run_lock.rs:1-220` vs `tools/capture/tests/common/mod.rs:261-368`), and
`temp_dir`/`write_game_file` helpers (`tests/common/mod.rs:801-817` vs
`crates/tempdir/src/test_support.rs:80-95,252-254`). Tool-specific fixture builders and
the four fake HTTP servers (`src/test_http.rs`, `tests/common/mod.rs:1011-1092`,
`tools/capture/tests/common/mod.rs:1198-1278`, `tests/artifact_bridge/server.rs`) encode
different contracts and are not candidates (N7, N8). Confidence: high.

### Logging, CLI and reporting implementation

**F18 — Logging *implementation* is three near-copies below the shared policy.**
`open_log_file` (`src/logging.rs:219-228`, `tools/sync/src/logging.rs:218-228`,
`tools/capture/src/logging/mod.rs:217-223`), the "log file: …" announcement
(`src/logging.rs:178-181`, `sync:123-126`, `capture:201-204` and each `main.rs`), the
env-warning print (`src/logging.rs:109-112`, `sync:71-74`, `capture:121-124`), the
subscriber-install warning (`src/logging.rs:159-176`, `sync:110-117`, `capture:179-190`),
and identical `non_blocking` writer + filter construction. The guard lifetime differs by
policy: product uses a process-global slot and explicit `flush_log()` (`src/logging.rs:56-65,192-211`),
sync/capture use RAII or `finish()` (`tools/sync/src/logging.rs:48-59`,
`tools/capture/src/logging/mod.rs:52-68`). Candidate boundary: format-agnostic
installation helpers added to `girpr-log`; leave layer construction and guard lifetime
local. Confidence: high.

**F19 — Timestamp format is duplicated despite `girpr_log::timestamp` existing.**
`crates/log/src/plan.rs:320-328` owns `%Y%m%dT%H%M%S%.6fZ`;
`tools/capture/src/config/mod.rs:763-765` re-implements it in `capture_id()`. The shared
helper's own doc names backup/snapshot/converter uses, so capture appears to have missed
it. Confidence: high.

**F20 — The five log flags and `--output` are declared three times.**
`src/cli.rs:224-273`, `tools/sync/src/cli.rs:21-67`, `tools/capture/src/cli/mod.rs:70-119`
share `value_name`, `default_value`, `girpr_log::parse_level`, `conflicts_with` and
word-for-word help text, differing only in `global`/flattened placement and project name.
`OutputFormat` is defined three times (`src/report/record.rs:30-58`,
`tools/sync/src/report/mod.rs:58-85`, `tools/capture/src/report/mod.rs:35-65`) and reused
by `tools/sled2redb/src/lib.rs:57`. clap-error → exit-code mapping is three implementations
(`src/main.rs:148-161`, `tools/capture/src/cli/mod.rs:476-494`, `tools/sled2redb/src/main.rs:31-53`)
with one more using clap's default (`tools/sync/src/main.rs:14`). Candidate boundary: a
`girpr-cli` leaf with `OutputFormat`, a clap-error mapper parameterised by usage code, and
a `LogArgs` macro/builder that injects the project name. `--version` presence differs
(capture only) and blocks a single "standard flags" bundle. Confidence: high.

**F21 — Report data planes are three large implementations of the same idea.**
`Val`/`Record`/`Serialize`/emit/summary exist in `src/report/{record.rs:71-226,emit.rs:37-211,summary.rs:356-411}`,
`tools/sync/src/report/{mod.rs:96-280,record.rs:20-285}` (positional/keyed/pair layouts,
injectable `ReportSink`) and `tools/capture/src/report/mod.rs:35-367,715-732`. Only
`sled2redb` reuses sync's (`tools/sled2redb/src/lib.rs:36,66-88`), proving a shared writer
works when contracts are identical. The deleted report's §A1 called this the highest-payoff
hand-rolled duplication but stopped short of a full shared `Record` because the value
vocabularies differ. Candidate boundary, if any: a narrow ordered-JSON/`write_line` core
with event placement, escaping and bool-vs-1/0 as parameters; a monolithic shared `Record`
would break A3/A8. Confidence: high on the duplication, medium on the boundary.

**F22 — Exit-table *mechanism* is duplicated; tables are not.** `define_run_exits!`
(`src/config.rs:70-164`) generates enum + `ALL` + code mapping + compile-time pin;
`tools/capture/src/exit/mod.rs:23-112` hand-writes the equivalent with `slug`, `describe`
and `artifact_is_untrustworthy`. A parameterised table macro is a mechanical candidate;
the values and meanings are independent (A9). Confidence: high.

**F23 — Audio language normalisation shares a primitive and diverges around it.**
`src/cli.rs:385-420` (lower + `_`→`-` + alias map + `HashSet` + none-sentinel) vs
`tools/capture/src/config/mod.rs:469-509` (lower + trim + `_`→`-`, preserves code, validates
it as a path component). `DISPLAY` order and the four code/name pairs are identical in
`src/repair/audio.rs:16-22,163-167` and `tools/capture/src/emit/gamedir.rs:120-130,166`.
Confidence: high.

### Domain data model (girpr ↔ gircap)

**F24 — The Sophon protobuf messages are wire-identical duplicates.** Verified side by
side: `src/sophon/proto.rs:9-47` and `tools/capture/src/manifest/proto.rs:16-118` declare
the same three types with the same tags and types, including the misspelled `chuncks`
(field 1) and opaque field 6; the only field-name difference is `unknown` (product) vs
`unknown_field6` (capture). Both carry the same Starward-parity rationale comments.
Capture adds methods (`is_folder_entry`, `chunk_bytes`, `covered_end`,
`validate_arithmetic`, `identity`, `normalise`) and a `Serialize` derive; its dump tests
pin the exact key set (`tools/capture/src/manifest/proto.rs:348-377`,
`tools/capture/src/manifest/dump.rs:738-757`). Candidate boundary: a `girpr-proto` leaf
holding the wire types; field names `chuncks` and `unknown_field6` and the `Serialize`
key names are load-bearing (A3). Confidence: high.

**F25 — Region identity, ids and base URLs are duplicated.** `Biz` values and
`as_str`/`endpoints`/`channel_tuple` in `src/biz.rs:3-37` vs the clap `ValueEnum` in
`tools/capture/src/cli/mod.rs:279-311` and `Biz::resolve` in `tools/capture/src/biz/mod.rs:31-61,98-177`;
the four production base URLs appear in `src/hyp.rs:531-544` and `tools/capture/src/biz/mod.rs:31-61`;
channel-only-on-deprecated in `src/hyp.rs:331-346` vs `tools/capture/src/biz/mod.rs:80-95,235-252`;
`DOWNLOAD_MODE_CHUNK` in `src/hyp.rs:525-529` and `tools/capture/src/hyp/types.rs:29-35`.
Both test suites pin the values (`src/hyp.rs:738-757`, `tools/capture/src/biz/mod.rs:436-474`).
Candidate boundary: a `girpr-model::region` leaf with clap behind an optional feature.
Capture's `Endpoints<'a>` override seam (`tools/capture/src/biz/mod.rs:179-265`) stays local.
Confidence: high.

**F26 — API DTO field sets match, decoding strategies deliberately do not.**
`src/hyp.rs:29-244` (`Wrapper<T>`, `GameConfig`, `GameBranch`, `ChunkBuild`,
`ChunkManifestMeta`, `ManifestFile`, `ManifestUrl`, deprecated types) vs
`tools/capture/src/hyp/types.rs:107-390,528-745` (`Wrapper<T>`, `GameConfigs`, `Branch`,
`GetBuild`, `BuildManifest`, `ManifestRef`, `Download`, deprecated types) cover the same
JSON fields. Sizes-as-strings is handled by `num_from_string` (coercing Visitor,
`src/hyp.rs:163-202`) in girpr and `IntOrStr` (untagged, wire-preserving,
`tools/capture/src/hyp/types.rs:37-105`) in capture. URL joining differs deliberately:
`src/sophon/stream.rs:568-591` uses `Url::join` with a trailing slash;
`tools/capture/src/hyp/types.rs:614-626` formats prefix + id + suffix. A shared DTO crate
must keep both strategies side by side. Confidence: high.

**F27 — Chunk identity `(md5, size)` is one rule in four places.**
`SophonChunk::identity()` (`tools/capture/src/manifest/proto.rs:120-126`) with consumers
in `tools/capture/src/manifest/diff.rs:384-423` and `tools/capture/src/assemble/assembly.rs:302-308`;
product `ReuseEntry::matches`/`FileReuse` (`src/sophon/map.rs:37-134`) and `sources_for`
(`src/repair/reuse.rs:174-191`). All apply first-occurrence-wins, case-insensitive MD5,
size part of identity. Candidate boundary: a `ChunkIdentity` value type. Confidence: high.

**F28 — Bounded zstd decode + digest of decompressed bytes is the same invariant in two
shapes.** Product `src/sophon/stream.rs:66-85,169-245,489-551` streams into a hashing sink
without holding the blob; capture `tools/capture/src/hash/mod.rs:305-573` is buffered with
a ceiling and emits hex strings. Only the low-level safe bounded decode is a plausible
shared primitive; the async-vs-owned shapes are genuine. Confidence: medium.

**F29 — Within-capture validation is itself triplicated.** Arithmetic invariants
(offsets ascending, gapless, Σsize == size, no overflow) are implemented in
`tools/capture/src/manifest/proto.rs:70-98`, `tools/capture/src/manifest/mod.rs:682-796`
and `tools/capture/src/assemble/ranges.rs:145-235`, with different message vocabularies;
`tools/capture/src/assemble/assembly.rs:81-116` notes two were only recently unified.
By contrast, the paired `zstd_measure`/`zstd_measure_from` loops
(`tools/capture/src/hash/mod.rs:476-573`) are deliberately duplicated to keep distinct
error text (`:518-523`) and must stay that way. Confidence: high.

### Documentation seams

**F30 — Docs under-describe the current sharing.** `README.md:44-45,62` and
`tools/capture/docs/00-spec.md:40-42` name only two shared crates; `girpr-filter` is shared
by `girpr` and `girsync` but is not listed in those tables (`root Cargo.toml:77-79`).
Any new shared crate therefore needs the doc tables updated as part of the change, not
as an afterthought. Confidence: high.

## Negative findings — duplicate-looking code that should not be extracted

**N1 — INI parsers.** `src/util/ini.rs` (last match, no section, full merge/render) and
`tools/capture/src/emit/gamedir.rs:46-118` (first match, `[General]` only, `ini_core`)
are intentionally different (A6). Capture already deduplicated its own reader in
`emit/selfcheck.rs:656-697`.

**N2 — Limits, counters and concurrency gates.** `src/util/counter.rs:19-23` has no
second consumer; `src/repair/scope/limits.rs` and `tools/capture/src/select/budget.rs:18-91`
share a shape but explicitly disclaim shared code (`scope/limits.rs:169-170`);
`tools/sync/src/rw/*` is a different primitive (F14).

**N3 — Exit-code tables.** See A9/F22. Capture's `artifact_is_untrustworthy`, `slug` and
`describe` are its own contract.

**N4 — Progress/stream reporting.** `src/report/emit.rs:83-131` (`ProgressGuard`,
stdout data plane, paused-time tests) has no counterpart; sync/capture log elapsed time
via `tracing` with different formats. `girpr_log::timestamp` is already shared.

**N5 — `DigestSet`, `AlgoSet`, `HashAlg`.** `tools/sync/src/hash/{algo.rs,set.rs,digestset.rs}`
are a deliberately forward-compatible raw-byte vocabulary, pinned by a byte-identical wire
test (`cache/wire.rs:124`) and an 80-byte/no-allocation guarantee noted in the deleted
report's §H. Same for girpr's `Scan`/`scan_*` range scanner (`src/util/md5.rs:90-330`) and
`HashingWriter` byte ceiling — no counterpart exists.

**N6 — The `rw` gate, FNV lock key, one-syscall/fail-fast cache open, symlink classification,
bounded task runner, `_tmp` resume artifacts and Starward/Collapse parity rules.** These are
the deleted report's §H "hand-rolled code that should stay" and the current
`tools/sync/DESIGN-NOTES.md:822-863` "not on the table" list. Changing them is out of scope
for a code-sharing refactor.

**N7 — Fake HTTP servers.** The four servers listed in F17 have deliberately independent
route/fixture logic; `tools/capture/tests/common/mod.rs:1-9` explicitly borrows nothing
from the repair binary's mock, and `docs/06-artifact-bridge-spec.md:707-709` forbids
sharing code that would couple the tools.

**N8 — Capture's artifact writer.** `tools/capture/src/blobs/mod.rs` named temp slots and
`tools/capture/src/emit/mod.rs:356-419` deliberately non-atomic documents are part of the
byte-stable artifact contract (A3).

**N9 — `Url`/status helpers and retcode policy.** Product `terminal_retcode`
(`src/hyp.rs:246-274`) vs capture's all-nonzero-is-fatal; the two URL builders in F26;
percent-encoding in `tools/capture/src/encoding.rs` (shares the word "hex" but is not a
digest encoder) — all deliberately distinct.

**N10 — `zstd_measure` and `zstd_measure_from`.** Deliberately duplicated for distinct
error messages (`tools/capture/src/hash/mod.rs:518-523`). See F29.
