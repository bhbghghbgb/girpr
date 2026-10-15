# TO-FIX — artifact-bridge prerequisites

**Throwaway.** This file is a scratch plan, not a document. It is deleted once
the work below lands, and **nothing in `src/`, `tests/`, `README.md` or
`docs/` references it or this path**. Do not add a row, link or "see also"
anywhere.

Two projects: `girpr` (repo root) and `tools/capture` (`gircap`). Every item is
grouped into a commit; a commit lists its own code *and* doc changes, because
landing them apart leaves the README and the flag table lying.

Ordering matters: the commits in §1–§3 are prerequisites for writing
`tests/artifact_bridge.rs`, which is the next piece of work and is **not** in
scope here.

---

## Decisions applied

| # | decision |
|---|---|
| C1 | add `--log-file` to `girpr` |
| C3 | `girpr` replaces `--json-summary` with `--output <text\|json>`, modelled on `girsync`; the format applies to **every** stdout record |
| C4 | **docs only.** Remove the "a CI check rejects the string" / "acceptance grep" wording. The intent was "do not couple", not "enforce by CI". No test is added. |
| C5 | `gircap` already has `--output <text\|json>`; only the **per-bundle record** is new |
| C6 | record `build_id` on every `chunks/index.json` reference entry |
| D2 | the shim runs **two** blob listeners (manifest, chunk) alongside the hyp and sophon metadata ones; the rewritten `url_prefix` keeps its **path** and only scheme+authority change, so the live build-id segment stays in the URL |
| D8 | the bridge parses `--output json`; §3 makes that stream clean |
| D11 | tree diff is per-run |
| D15 | the **server's request log is primary** for `must_not_fetch`; `--log-file` is the documented last resort |
| D16 | state the `--check-only` expectation rather than dropping it |

Everything not listed here takes the recommended fix from the review.

---

## 1. `girpr` — small independent fixes

### Commit 1 — `girpr: point the doc link at the method that exists`

`RepairPlan::est_download_bytes_with` does not exist; the name is stale.

- `src/repair/scope/limits.rs:35` — fix the intra-doc link. This is the only
  rustdoc warning in the crate that names a *symbol* rather than a private
  module, and it is the one a reader would try to click.
- `README.md:43` — drop it from the key-symbol column.
- `docs/05-scope-limitation-implementation-plan.md:167` — update the reference.

**Accept:** `cargo doc --no-deps -p girpr` no longer warns about
`est_download_bytes_with`; `cargo test -p girpr` green.

---

## 2. `girpr` — new flags

### Commit 2 — `girpr: add --log-file`

`logging::init_tracing` computes the path and `main.rs:14` throws it away. The
log lands in `<exe-dir>/logs/girpr_<ts>.log` — `target/debug/logs/`,
append-mode, shared by every run that has ever happened. No external process
can tell which file belongs to *this* run, which is why the bridge has to read
a `--log-file` it named itself. `girsync` and `gircap` both already ship this
flag; the asymmetry is gratuitous.

- `src/logging.rs` — `init_tracing(level, log_file: Option<&Path>)`. With
  `None`, unchanged behaviour (`<exe-dir>/logs/…`). The file layer is **already
  always-`TRACE`**, so no `--log-level trace` is needed to get chunk lines —
  say so in the help text.
- `src/cli.rs` — `--log-file <FILE>`, next to `--log-level`.
- `src/config.rs` — `RunCtx.log_file: Option<PathBuf>`.
- `src/main.rs` — thread it; keep the `tracing::info!("log file: …")` line, and
  additionally `eprintln!` the path when `--log-file` was given explicitly, so a
  wrapper that forgot to capture it still learns where it went.
- `tests/cli_contract.rs` — add `--log-file` to
  `help_text_lists_every_documented_flag`; add a case that a bogus parent
  directory **degrades to a warning, not a failure** (`init_tracing` is
  infallible by design and must stay so).
- `README.md` — flag-table row; note the layer is always `TRACE`.

**Accept:** `girpr --game-path . --biz hk4e_global --log-file <tmp>/x.log …`
creates `<tmp>/x.log`; without the flag, nothing changes.

### Commit 3 — `girpr: replace --json-summary with --output <text|json>`

Modelled on `tools/sync/src/report/mod.rs`. **This is the one breaking change in
the batch** — `--json-summary` is a documented public flag. Blast radius, all
12 sites found by grep:

- `src/cli.rs:134,189` → `pub output: OutputFormat` (`clap::ValueEnum`,
  `#[default] Text`, `global = true`).
- `src/config.rs:95` → `RunCtx.output: OutputFormat`; `bool` params on
  `report::format_report_line` / `format_summary_line` become the format.
- `src/report.rs` — introduce a `Record` (event name + ordered fields, held once
  and rendered twice, like girsync's) and route all four stdout lines through
  it: `REPORT`→`report`, `PROGRESS`→`progress`, `PLAN`→`plan`,
  `SUMMARY`→`summary`. JSON is NDJSON, one object per line, `event` = label
  lowercased.
- `src/repair/mod.rs:281` — **delete the duplicate `PLAN` emit.** Today
  `--print-plan --json-summary` prints a bare text `PLAN …` line *and* a JSON
  twin; measured. With one record per line that stops being possible.
- **`scoped` type collision:** `plan` JSON emits `"scoped": true` (bool) while
  `summary` JSON emits `"scoped":1` (int). Emit both as `1`/`0` so one key name
  means one type across the stream; the `SUMMARY` shape is the documented one
  and does not change.
- `tests/cli_contract.rs:164` — `--json-summary` → `--output` in the help list;
  drop `--json-summary` from the bad-input cases if any; add a case that
  `--output xml` is exit `1`.
- `tests/repair_offline.rs:77` — `json_summary: false` → `output: OutputFormat::Text`.
- `README.md:99,227`, `docs/02:81`, `docs/03:53,65`, `docs/04:49`,
  `docs/05:105` — update the wording, keep the doc links.
- Add `tests/cli_contract.rs` coverage that `--output json` emits **only** JSON
  lines on stdout (no `KEY=value` text anywhere), which is the property the
  bridge now depends on.

Keep `report::emit`'s "print the rendered line *and* `tracing::info!` it"
behaviour, so whichever format is chosen, the log file carries the same data
plane a stdout parser would see.

**Accept:** `--output json` yields NDJSON only; every key name has one type;
`--output text` output is byte-identical to today's default run.

---

## 3. `gircap`

### Commit 4 — `gircap: record build_id on every chunk reference`

`chunks/index.json`'s `referenced_by` entries carry `tag` but not `build_id`.
The live chunk URL embeds the build id
(`…/chunks/cxhpq4g4rgg0/sMXGW2ll3Fuu`) and 00-spec §3.7 records that the same
chunk id under the *other* build's prefix is a 404 — so the build id is a real
part of a chunk's address, and the index should say so.

- `tools/capture/src/blobs/chunks.rs` — `Reference.build_id: String` (or
  `Option<String>` with `#[serde(default)]`; the capture has the
  `tag -> build_id` map from `captured.versions`, so it can always be filled).
- `tools/capture/docs/01-artifact-schema.md` §12 — document the field.

Existing bundles keep verifying: the field is additive and `verify` reads
whatever is on disk. A bridge must still fall back to
`MANIFEST.json → versions.{old,new}.build_id` so an old bundle works.

**Accept:** `cargo test -p gircap` green; `gircap verify --artifact` on both
existing bundles still exits `0` (they predate the field).

### Commit 5 — `gircap: list emits one record per bundle`

`docs/06` §3 Gate 2 tells discovery to use `gircap list --root artifacts`, but
today that prints **one** `SUMMARY` (`bundles=2 … first=… last=…`) and puts every
per-bundle fact on stderr at INFO. A machine can consume it only by parsing
narration.

- `tools/capture/src/report.rs` — generalize `Summary` into a `Record`
  (event + ordered fields) so a run may emit more than one line; keep `Summary`
  as the terminal record. Rewrite the module doc: it currently asserts
  "exactly one line per run".
- `tools/capture/src/run.rs` `list()` — emit `BUNDLE path=… tags=… patchmaps=…
  blobs=… selfcheck=… complete=…` / `{"event":"bundle",…}` per **valid** bundle
  before the `SUMMARY`. Skipped (unopenable) directories stay stderr-only, as
  today.
- Preserve the existing ordered-pair JSON writer. Do **not** route this through
  `serde_json::Map` to gain ordering — `src/report.rs` already documents why
  (`preserve_order` is a workspace-unified feature).
- `tools/capture/docs/00-spec.md` §7 Output — the "one SUMMARY line, or a single
  JSON object" sentence becomes "records, one per line, summary last".
- `tools/capture/tests/cli_contract.rs` — assert the NDJSON shape of
  `list --output json`.

**Accept:** `gircap list --root artifacts --output json` emits one `bundle`
record per bundle then one `summary`; stdout is pure NDJSON.

---

## 4. Docs — `docs/06-artifact-bridge-spec.md` (the bridge spec itself)

No code in this section; it is the rewrite the bridge will be written against.
Rewrite in place, keeping the existing §1–§3 structure where it is still right.

**§2 Placement.** Delete "the fifteen existing offline tests" — `repair_offline.rs`
has **50**, `cli_contract.rs` **4**, the lib **76**. State the module mechanics:
`tests/artifact_bridge.rs` plus `mod server;` resolving to
`tests/artifact_bridge/server.rs`, so neither `repair_offline.rs` nor
`cli_contract.rs` recompiles the server.

**§2/§3 Parallel safety — the defect that would have cost the most.**
§5 says one run per capture; §6.3 says one scratch copy that the runs
accumulate on; §2 puts the bridge in `tests/`. Rust runs `#[test]`s **in
parallel threads**, so a shared tree + shared listener + shared request log
means concurrent `girpr` processes fighting over one `game/`. Rewrite as:
**per capture, a fresh `game/` copy and its own listeners.** Measured — 8 fresh
copies of the fixture take **0.035 s**. The manifest blob cache lives outside
the listeners (shared behind the test module) so §4.2's caching still pays.
Keep the one-run-per-capture rule: it is what makes `download_bytes`
attributable.

**§4 Route table — align it with §4.1.** §4.1 said "replace the prefix with the
listener's own origin", which collapses the manifest and chunk namespaces into
one root and contradicts §4's two-prefix table. New rule: **change only the
scheme and authority; keep the path**, and run two blob listeners so the two
namespaces stay distinguishable by port. Resulting shapes (measured off the live
body):

```
manifest : …/client_app/sophon/manifests/cxhpq4g4rgg0/<build_id>/manifest_<16hex>_<32hex>
chunk    : …/client_app/sophon/chunks/cxhpq4g4rgg0/<build_id>/<16hex>_<32hex>
```

Each listener takes the **last two** path segments and ignores the rest, so the
route is indistinguishable from production's and the test still proves `girpr`
builds URLs from `url_prefix` + `id` alone. Write down the new invariant this
buys: **every blob request's `build_id` must equal the build whose manifest
declared the id** — the 00-spec §3.7 namespacing rule, now assertable.

**§4 getBuild.** Say explicitly that a tagless request serves
`api/getBuild.<versions.new.tag>.json` and that an unserved tag returns the
captured `retcode -202` body. Measured consequence of getting it wrong: exit
**0**, `local_manifest=unavailable`, `download_bytes=913040` instead of 274 426,
and the `must_not_fetch` chunk requested — the "degraded-but-green" of §7,
confirmed.

**§4 Manifest lookup.** Say how a `manifest_<16hex>_<32hex>` id becomes
`manifests/<tag>/<field>.manifest.zst`: build `id → (tag, field)` **while
rewriting** the body (the loop is already there), and 404 anything absent.
Without this an implementer reaches for a filesystem scan over
`manifests/*/*.meta.json` and gets a slower, different answer.

**§4 Fields served.** Stop hardcoding `en-us`. Measured on the real bundle:
the body carries five manifests (`game`, `zh-cn`, `en-us`, `ja-jp`, `ko-kr`);
the run fetches exactly `game` + `en-us`, **4 manifests per run and 4 per
pre-pass**, because `select_manifests` drops every `len == 5 && contains('-')`
field then re-adds only the selected one — and the other three blobs are not in
the bundle. Rule becomes "index whatever `manifests/<tag>/` holds; the run
selects `game` ∪ resolved audio". Keep the "`--audio` widens it" warning.

**§4.3 Request log — primary, with the log file as the fallback.** Say the
server's own `Vec<(kind, id)>` is the source of truth for `must_not_fetch`, and
that `--log-file` (commit 2) is the last resort: parse
`chunk downloaded chunk=<id>` and `no local reuse candidate -> download
chunk=<id>` from the trace lines, matching on the **download record**, never
the bare id — a reused chunk is logged too, and that is how the first attempt
false-positived on 19 chunks that were never downloaded.

**§5 Driver.** One flag table with a *which runs* column, replacing §5's and
§6.4's two. Add `--fail-fast` with the measured reason: on the real bundle,
one 404'd chunk costs **17.6 s** with the flag and **88.5 s** without (5
requests vs 25), both exit `3`. A missing blob is the most likely bridge
failure and the slowest one. Clarify the `--include` cell to "the `path` field
of `patchmaps/<id>.json`". Say the parse plane is `--output json` NDJSON
(`report` / `plan` / `summary` records), which commit 3 makes clean.

**§5.1 Pre-pass.** "`files=1` and `est_download_bytes` equal the patchmap total"
is false for `removed`, which has no `totals` and prints
`PLAN files=0 est_download_bytes=0 purge_files=1 purge_bytes=1731133 scoped=1`.
Carve it out — the purge counters are its pre-pass assertion.

**§6 Assertions.** State that the tree diff is **per run** (before-this-run vs
after-this-run), because §6.3's copy is per capture after the §2 rewrite. Add
the structural assertion: **capture paths are pairwise distinct.** True for this
bundle (8 distinct paths) and `select`'s exclusion set keeps it true, so it is
a cheap guard rather than a live bug — but a repeat would make the second run
silently skip and report `download_bytes=0` against a non-zero expectation.

---

## 5. Docs — corrections outside the bridge spec

### Commit 6 — `docs: correct the testing claims and de-couple the wording`

- **`README.md:250-252` is factually wrong**, and it is precisely the gap the
  bridge exists to close. It claims *"`tests/repair_offline.rs` drives the real
  binary through the flags against the mock, which is the only way to prove a
  flag actually reaches `HypClient`"*. Verified false: `repair_offline.rs` calls
  `repair::run(ctx)` **in-process** — 50 call sites, zero `Command`, zero
  `CARGO_BIN_EXE_*`. `cli_contract.rs` is the only file that spawns the binary
  and it never reaches the API. Rewrite, and point at `tests/artifact_bridge.rs`
  as the thing that delivers the property.
- `README.md:239` — "34 unit + 15 integration" → 76 / 50 / 4.
- **`tools/capture/docs/00-spec.md` §2** — remove the `Enforcement` column's
  "a CI check rejects the string" and any reading that independence is
  mechanically enforced. The intent is *do not couple*; say that and stop.
  No test is added.
- **`docs/06-artifact-bridge-spec.md` §1** — drop "the acceptance grep … must
  keep returning doc comments only"; same reason.
- **`tools/capture/docs/00-spec.md` §8.1.4 #3** — demote to the hand-run
  predecessor and point at §4.3's server log as normative, with `--log-file`
  as the fallback. Keep its caveat (a reused chunk is logged too) as the
  *reason* the server log is better.
- **`tools/capture/docs/00-spec.md` §8.1.1 step 2** — state the `--check-only`
  expectation instead of dropping it: its `expect_md5` / `actual_md5` are
  `tracing::warn!` lines, i.e. **stderr/log only, never the data plane**, and
  under `--include <capture path>` the seven repairable captures are *expected*
  to report damage (exit `4`). Only `identical` is clean.
- **`tools/capture/README.md`** — § Status says "Built to milestone M4"; it is
  through **M12** (`verify`/`materialize`/`list`, `game/`, `expected/`,
  `patchmaps/`, `chunks/`, gzip, A18/A19/A20 — both real bundles verify 90/90).
  Rewrite, and add `gircap list --root artifacts` and `--output json` to the
  command block, since the bridge spec sends the reader to exactly that command.

---

## After all of the above

`tests/artifact_bridge.rs` becomes writable. It is still `#[ignore]`d on every
test, still skips rather than fails when no bundle is present
(`GIRPR_ARTIFACT_BUNDLE` → `gircap list --root artifacts --output json` → skip
message naming the exact `gircap capture` command), and still runs
`gircap verify --artifact <dir>` first, skipping the precondition when `gircap`
is not built.