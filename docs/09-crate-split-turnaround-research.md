# Crate-split turnaround research: assumptions and findings

Static research over the workspace at `79660c61` (`perf: shrink debug/test links to cut
build time on HDD`). No source was changed, no build or test was run, and no line was
executed. Every item below is a claim to be validated during implementation; line
numbers are from the current tree and may drift.

The goal framing is **turnaround after a change**: Cargo recompiles the changed package
and every downstream dependent, then re-links the binaries that depend on them. The
items below describe where code is concentrated, which edges propagate a change, and
where crate boundaries would or would not shrink that set. Candidate boundaries are
descriptions of structure, not an implementation ordering.

Items are labelled `A` (assumptions) and `F` (findings), including a subsection of
findings that record exclusions.

## Assumptions

**A1 — Rebuild unit.** Cargo fingerprints a package at source-file granularity for a
(package, feature-set, profile, target) tuple. Editing any file in a package invalidates
that target; upstream dependencies stay cached; downstream dependents and their test
binaries recompile and re-link. `#[cfg(test)]` code is not compiled into the lib consumed
by `main` and integration tests, but an edit inside such a block still invalidates the
source file's fingerprint for every build. Sources: `Cargo.toml:16-20`,
`src/lib.rs:47-60`, `tools/capture/src/http/mod.rs:74-75`.

**A2 — Splitting only helps when the dependent set actually shrinks.** If a low-level
crate is split out but the remaining facade still depends on and re-exports both halves,
the facade and all of its test targets remain downstream of the split, so the change adds
a crate to compile without removing any rebuild edge. A gain requires the high-level
consumers (binaries, integration tests) to depend on the smaller crates directly, or the
facade to stop being an ancestor of every target. Sources: `tools/sync/src/lib.rs:44-59,65-98`,
`tools/capture/src/lib.rs:50-71,73-86`, `src/main.rs:8-10`.

**A3 — Baseline.** The last commit changed only `[profile.*]` in the root `Cargo.toml`
(+24 lines); no source restructuring has happened since the T01–T20 extraction work.
The 13 shared crates under `crates/` are the product of that extraction and are treated
as existing facts, not as new candidates. Source: `git show 79660c61`, `git log`.

**A4 — Crate conventions.** Shared code lives in `crates/<policy>`, is named
`girpr-<policy>`, carries `[lib] name/path`, a one-line `description = "The shared …"`,
`edition = "2024"`, `version = "0.1.0"`, the stable-toolchain comment, and
`[lints] workspace = true`. Root lint policy: `unsafe_code = "deny"`,
`clippy::all`/`unwrap_used`/`expect_used = "warn"`. Sources: e.g.
`crates/audio-language/Cargo.toml:1-12`, `crates/log/Cargo.toml:1-13`, `Cargo.toml:22-38`,
`rust-toolchain.toml:1-5`.

**A5 — gircap independence is a hard boundary.** `gircap` may not depend on `girpr`, may
not invoke its binary, and may depend only on leaf crates holding no program logic. The
independence table currently names eleven shared leaves; adding a leaf requires updating
both that table and the matching manifest comment. The constraint is documented as not
mechanically enforced. Sources: `tools/capture/docs/00-spec.md:34-48` (table at `:42`),
`tools/capture/Cargo.toml:1-44`.

**A6 — Byte contracts are load-bearing.** Bundle, report and dump bytes are pinned by
tests: protobuf field names `chuncks` / `unknown_field6` are observable;
`girpr-report-core` deliberately omits `preserve_order` so workspace-wide feature
unification cannot change its output; `girpr_log::format_timestamp` pins its format.
An extraction must not change key order, casing, spacing or flag conditions. Sources:
`crates/proto/src/lib.rs:18-25,74-97`, `crates/report-core/src/lib.rs:19-22`,
`crates/log/src/plan.rs:320-340`.

**A7 — nextest shapes test helpers and names.** nextest runs one process per test and
does not run doctests, so shared test helpers must be a public library surface reached
through thin per-consumer shims; doctest examples are mirrored as unit tests. Setup
scripts run on every invocation regardless of the package selected.
`.config/nextest.toml` pins skip-visibility overrides to exact binary names and
fully-qualified test paths, so a crate split or test relocation changes those filters.
Sources: `.config/nextest.toml:15-19,59-85,106-214`, `crates/log/src/lib.rs:53-56,66-72`,
`crates/tempdir/src/lib.rs:14-17,252-258`.

**A8 — Dev-dependency feature unification.** Edition 2024 implies resolver 3; for a
package under test its dev-dependency features are unified into its own normal
dependencies, while `cargo build` keeps a feature-reduced artifact set. The same crate
can therefore exist as two cached builds (normal and test-unified). Sources:
`Cargo.toml:73,124-148` (comment at `:134-135`), `crates/log/Cargo.toml:15-20`,
`crates/retry/Cargo.toml:19-29`.

**A9 — Profile effects.** `[profile.dev] codegen-units = 4`;
`[profile.test] debug = 0, codegen-units = 1, incremental = false`;
`[profile.release] lto = "thin", codegen-units = 1, strip = true`. `codegen-units` and
`debug` affect object-file count and link size, not which packages rebuild. With
`incremental = false` every affected target is compiled from scratch. Profile settings
apply workspace-wide even though only `.` is a default member. Source: `Cargo.toml:150-181`.

**A10 — Cycles constrain boundaries.** Several module graphs contain cycles that are
legal inside one crate but forbid a cut through them; a boundary must sit at an acyclic
leaf or around a whole strongly-connected group, with shared types moved down first.
Product cycles: `report::summary ↔ repair::scope::ledger`, `config ↔ repair`,
`scope ↔ phases`. Tool cycles: `util ↔ cache`, `effective ↔ planner`, `cli ↔ config`
(both tools), `http ↔ biz`. Sources: `src/report/summary.rs:3-5`,
`src/repair/scope/ledger.rs:17-18`, `tools/sync/src/cache/open.rs:137`,
`tools/sync/src/util.rs:34`, `tools/capture/src/http/probe.rs:32`,
`tools/capture/src/biz/mod.rs:26`.

**A11 — Moving tests out has a limited payoff.** `#[cfg(test)]` files are compiled into
the lib *test* target only, not the shipped lib. Moving them out to `tests/` shrinks the
lib test target and stops a test edit from invalidating the shipped file, but it does
not reduce the cost of re-linking integration binaries after a production-source edit.
Sources: `tools/capture/src/http/mod.rs:74-75`, `tools/sync/src/hash/mod.rs:18-19`,
`src/lib.rs:58-59`.

**A12 — io-profile helpers are isolated.** `tests/scripts/io-profile/iot-bench` and
`iotime` declare their own empty `[workspace]` tables and are absent from the root member
list, so they add no targets to `cargo test --workspace`. Sources:
`iot-bench/Cargo.toml:14-16`, `iotime/Cargo.toml:19-21`, `Cargo.toml:17`.

**A13 — Divergent policies are intentional keepers.** The prior research report's
negative findings N1–N10 (INI parsers, limits/counters/gates, exit tables, progress
reporting, `DigestSet`/`AlgoSet`/`HashAlg`, the `rw` gate / FNV key / fail-fast cache open
/ symlink classification / bounded task runner / `_tmp` artifacts / parity rules, fake
HTTP servers, capture's artifact writer, URL/status/retcode helpers, the `zstd_measure`
pair) are duplicate-looking code deliberately left local; they are not extraction
candidates. Source: `git show e76c69c9^:docs/08-shared-crate-refactor-research.md`.

**A14 — Measurement basis.** Line counts come from `Measure-Object -Line` over the
current tree; subtree and "test tail" totals (last `mod tests` to EOF) are approximate.
Workspace totals: ~28.8k lines in `src/**` (50 files), ~27.6k non-blank in `tools/sync`,
~41.5k in `tools/capture`, 311 entries in `Cargo.lock`.

## Findings

### Dependency graph: hubs, leaves, and the inversion

**F1 — Member and edge inventory.** The workspace has 17 members: root `girpr`, 13
`crates/*`, 3 `tools/*`; `default-members = ["."]` means tool sources do not rebuild on a
default root build. Local path edges: root → 12 shared crates (+ dev `girpr-log` with
`test-support`, `girpr-tempdir`); `girsync` → 6 shared crates (+ dev tempdir);
`gircap` → 10 shared crates (+ dev tempdir); `sled2redb` → `girsync`, `girpr-digest`,
`girpr-cli` (+ dev tempdir). Sources: `Cargo.toml:17-20,46-63,81-112,124-133`,
`tools/sync/Cargo.toml:37-82,108-112`, `tools/capture/Cargo.toml:58-167`,
`tools/sled2redb/Cargo.toml:25-53`.

**F2 — Direct fan-in of the shared crates.** `girpr-digest` and `girpr-tempdir` have
fan-in 4; `girpr-log`, `girpr-fs-names`, `girpr-cli`, `girpr-report-core` have fan-in 3
(`log` has a second, feature-distinct root dev edge); the remaining seven have fan-in 2
(`audio-language`, `exit`, `filter`, `fs-identity`, `model`, `proto`, `retry`).
Source: consumer manifests as cited in F1.

**F3 — The `girpr-tempdir → girsync` inversion is the dominant fan-out fact.**
`crates/tempdir/Cargo.toml:24-25` declares `girsync` and `serde_json` as **normal**
dependencies because `pub mod test_support` (`crates/tempdir/src/lib.rs:70-81`,
`test_support.rs:22-33`, 707 lines) constructs girsync option/cache/planner types.
`girpr-tempdir` is a dev-dependency of all four programs. Therefore a change to any of
`log`, `fs-names`, `fs-identity`, `digest`, `filter`, `report-core` rebuilds `girsync`,
which rebuilds `girpr-tempdir`, which invalidates the test builds of `girpr`, `gircap`
and `sled2redb`; `sled2redb` also depends on `girsync` normally, so its shipped build is
in the cascade. Until this edge changes, splitting those six leaves further cannot
shrink their rebuild scope. The contrast is explicit in `girpr-fs-identity`, whose tests
build their own scratch directory because "the leaf has no dependency on any program"
(`crates/fs-identity/src/lib.rs:192-193`), and in `girpr-report-core`, which keeps
`serde_json` as a dev-dependency (`crates/report-core/Cargo.toml:22-25`).

**F4 — Heavy external dependencies are concentrated in identifiable places.**
`reqwest 0.13` (+`json`,`stream` in root; defaults in gircap) resolves through hyper/h2/
rustls/aws-lc-rs; `aws-lc-sys` uses `cmake`+`cc`+`pkg-config` and `zstd-sys` uses `cc`,
so native builds dominate first/full builds (`Cargo.lock:108-128,281-287,1757-1797,3004-3029`).
`redb = "=4.6.0"` is confined to `girsync` (`tools/sync/Cargo.toml:95`); `sled` is
confined to `sled2redb` (`tools/sled2redb/Cargo.toml:34`, and the two are explicitly
separated at `tools/sync/Cargo.toml:65-66`). `wiremock 0.6` is a dev-dep of root and
gircap only (`Cargo.toml:148`, `tools/capture/Cargo.toml:173`). `prost` lives in
`girpr-proto` plus direct root/gircap edges; `tracing-subscriber` `json` is enabled by
the tools but not root.

**F5 — Lock presence is not compilation.** `quinn*` and `ring` are in `Cargo.lock` via
optional reqwest `http3`/`rustls` alternatives, but no manifest enables `http3`
(`Cargo.lock:1562-1616,1800-1811`); `--all-features` invocations documented at
`README.md:315-321` would enable them. `futures-macro` is in the lock but root avoids it
with `futures-util` `default-features = false, features = ["std"]` (`Cargo.toml:113-117`).

**F6 — Distinct feature variants in flight.** tokio has at least three sets: root normal
(`rt-multi-thread,macros,time` + reqwest-unified `net/sync/io-util/fs`), root test
(adds `test-util,process,signal-hook-registry`; `net`/`io-util` at `:144` are redundant
with the normal graph), and gircap normal without `stream` → no `tokio/fs`
(`tools/capture/Cargo.toml:89-97`). `girpr-log` is built with and without
`test-support` (`Cargo.toml:46,129`). `girpr-model/clap` is enabled by root and gircap,
so the "without clap" configuration is effectively untested by the programs.
`girpr-retry` adds `macros,rt` in dev (`crates/retry/Cargo.toml:25-29`), and `girpr-log`
adds `rt-multi-thread,macros` in dev (`crates/log/Cargo.toml:22-25`).

### Profile and test-binary mechanics

**F7 — Last commit and documented bottleneck.** `79660c61` added `[profile.dev]` and
`[profile.test]`; `[profile.release]` already existed. The file states debug/test builds
are "link-bound on low-core HDD hosts", that the default 256 codegen units produced
"hundreds of object files per crate", that a test run links dozens of binaries with a
100–190 MB MSVC PDB each and dropping debug info cut measured link time "several-fold",
and that the incremental cache meant "~140k tiny files" per test binary. Source:
`Cargo.toml:150-172`.

**F8 — Measured turnaround datum.** `docs/06-artifact-bridge-spec.md:148-159` records
63 s wall for six bridge tests and 67 s including the rebuild on a 4-core Windows
x86_64 host. No other wall-clock target measurement is documented.

**F9 — Test-target count multiplies link cost.** Integration binaries: root `girpr`
**14**, `girsync` **16**, `gircap` **10**, `sled2redb` **1**; plus lib/bin unit-test
targets and 12 leaf-crate lib-test targets (71 unit tests; `girpr-exit` has none).
Unit-test counts in `src/`: root ~435, `girsync` ~346, `gircap` ~728, `sled2redb` 2.
Sources: `README.md:310-321`, `tools/sync/Cargo.toml:30-33`, `Cargo.toml:17`.

**F10 — No workspace build scripts; proc-macro surface.** No manifest declares `build =
` or `[build-dependencies]`; `tools/sync/src/commands/sync/plan/build.rs` is an ordinary
module, not a build script. Proc macros in the graph: `clap_derive`, `serde_derive`,
`thiserror-impl`, `tokio-macros`, `prost-derive`, `tracing-attributes`. Sources: all
`Cargo.toml` files, `Cargo.lock:2325-2333`, `tools/sync/src/commands/sync/plan/build.rs:148`.

### Product (`src/`) structure and candidate boundaries

**F11 — Module tree.** `src/lib.rs:47-60` declares `biz, cli, config, hyp, logging,
phase, repair, report, retry, sophon, util`, `#[cfg(test)] test_http`, plus an
*unconditional* `pub mod test_support` (`src/lib.rs:111-324`) and `#[cfg(test)]
clock_support` (`:328-355`). Subtrees: `repair` (`repair/mod.rs:41-50`), `phases`
(`repair/phases/mod.rs:14-18`), `file` (`repair/file/mod.rs:87-89`), `scope`
(`repair/scope/mod.rs:28-31`), `report` (`report/mod.rs:27-32`), `sophon`
(`sophon/mod.rs:13-16`), `util` (`util/mod.rs:22-36`).

**F12 — Dependency polarity inside the product.** Low-level / zero product-internal
outgoing deps: `biz`, `phase`, `logging`, `retry`, all of `util`, `report/record`,
`repair/lock`, `repair/audio`, `repair/scope/filter`. High-level hubs: `repair/mod.rs`,
`repair/file/{mod,inplace,temp}.rs`, `repair/phases/{manifest,metadata,plan,check,post}.rs`,
`repair/scope/report.rs`, `repair/scan.rs`, `config.rs`, `report/{summary,emit}.rs`.
`util` is the widest stable surface: ~3.6k production lines with no product-module
dependencies outside `util`, imported by `cli`, `sophon`, `report`, `config`-adjacent
code and nearly every repair file. Sources: per-file `use crate::` lines, e.g.
`src/repair/lock.rs` (production, none), `src/util/{md5,path,counter}.rs` (none),
`src/repair/file/inplace.rs:13-23`, `src/repair/scope/report.rs:28-33`.

**F13 — Product cycles.** `report/summary.rs:4` ↔ `repair/scope/ledger.rs:17`;
`config.rs:17` → `repair::scope` while `repair/phases/manifest.rs:34` → `config::RunFailure`
and `repair/mod.rs:62` → `config`; `repair/scope/report.rs:29-31` ↔ `repair/phases/plan.rs:25`
and `repair/scope/limits.rs:26-27`; `report/emit.rs:25` → `hyp` types. These four cuts are
not structurally free (A10).

**F14 — Candidate: `util` leaf crate.** Files `src/util/{mod,atomic,counter,ini,md5,path}.rs`
(~3,569 lines) plus test-only `junction_support.rs` (841) and `symlink_support.rs` (375)
≈ 4,785; deps are std/`anyhow`/`serde_json`/`md-5`/`girpr-digest`/`tempfile` (+ dev
tempdir). Today a change in `repair`, `report` or `cli` recompiles all of it as part of
the lib; as a separate crate it stays cached. Edit *to* `util` still invalidates all
downstream targets. Source: `src/util/mod.rs:22-36`.

**F15 — Candidate: writer/file-repair crate.** `src/repair/file/{mod,inplace,temp,mode}.rs`
(~4,826 lines, ~2.1k tests) is the largest cluster of reqwest + zstd + tokio + tempfile
repair code. It is high-level (few dependents: `repair/mod.rs`, `repair/scan.rs`,
`repair/scope/report.rs`) but names plan/scan/ledger types, so the payoff depends on a
narrow interface. Source: `src/repair/file/mod.rs:75-85`, `src/repair/file/inplace.rs:13-23`.

**F16 — Candidate: plan/purge/scope "decision" crate.** `repair/phases/plan.rs` (1032),
`repair/purge.rs` (1141), `scope/{limits,report,ledger,filter}.rs` (491+1039+920+196)
≈ 4,819 lines; isolates `RepairPlan`, `PurgePlan`, `Limits`, `PathScope`, `Meter`.
Constrained by the `Meter`/`Summary` cycle (F13).

**F17 — Candidate: HTTP + Sophon fetch crate.** `src/hyp.rs` (782) +
`src/sophon/{mod,filter,map,proto,stream}.rs` (1529) ≈ 2,311 lines, the sole home of
reqwest/zstd/prost-facing repair code; `report/emit.rs:25` (`ReportProvenance` naming
`GameConfig`/`GameBranchPackage`/`ChunkBuild`) is the blocker.

**F18 — Candidate: report plane crate.** `src/report/{mod,record,summary,emit}.rs` (820)
+ `src/phase.rs` (143) ≈ 963 lines, widely imported but small; keeps `Record`/`emit`/
`Summary`/`ProgressGuard` cached when repair internals change. Constrained by F13.

**F19 — Cheapest product boundaries.** `repair/lock.rs` (796, no production product deps;
exposed to tests through `src/lib.rs:117`), `repair/audio.rs` (346), `repair/scope/filter.rs`
(196), `biz.rs` (6), `phase.rs` (143), `retry.rs` (281), `logging.rs` (400) are already
effective leaves requiring no restructuring.

**F20 — Embedded product test code.** Total ~28.8k lines in `src/`, ~13k (45%) is
`#[cfg(test)]`: whole test-only files `util/junction_support.rs` (841),
`util/symlink_support.rs` (375), `report/tests.rs` (512), `test_http.rs` (73); largest
inline tails `repair/file/inplace.rs:1074` (1,139), `repair/phases/post.rs:740` (746),
`repair/file/temp.rs:762` (694), `util/ini.rs:550` (816), `repair/phases/check.rs:390`
(739), `repair/scope/report.rs:451` (589), `repair/phases/manifest.rs:284` (639).
Test-only hooks live in shipped files at `repair/file/mod.rs:367-395,484-486`,
`repair/purge.rs:450-489`, `repair/reuse.rs:92-95`, `repair/scan.rs:107-125`,
`report/emit.rs:96-103`, `util/path.rs:418-430`.

**F21 — Biggest current rebuild triggers.** An edit to `src/lib.rs` (333 lines: module
declarations + re-exports + `test_support`) or `src/util/mod.rs` (38 lines) invalidates
the lib → bin + all 14 test binaries. `src/repair/mod.rs` (1,338 lines) holds `run`,
both phase halves and `spawn_blocking_traced`; an edit there cannot be cached by any
other target. An edit inside a `#[cfg(test)]` tail such as `inplace.rs` or
`scope/report.rs` is paid by `cargo build` too (A1). A change in `sophon/stream.rs`,
`repair/file/temp.rs` or `repair/phases/manifest.rs` forces a re-link of every test that
inlines `tests/common/mod.rs`.

**F22 — Narrow tests exist.** `tests/artifact_server.rs` and `tests/artifact_bridge.rs`
import only `girpr_tempdir` plus their own helpers; `tests/cli_contract.rs:38` needs only
`girpr::Args`; `tests/process.rs:9-13` drives the binary. If `cli`/`config`/`report` were
a crate, the CLI-contract tests could link that crate instead of the whole product lib.
The shipped `pub mod test_support` is used by both integration tests and the lib's own
unit tests, so it cannot simply be `cfg(test)`-gated. Sources: `src/lib.rs:111-125`,
`tests/common/mod.rs:28-38`.

### Tool (`tools/`) structure and candidate boundaries

**F23 — Package shapes.** `girsync`: 62 `.rs` files, `src/lib.rs` (94) with 16 `pub mod`
roots and broad re-exports, 16 integration binaries + 1 bench (`tree_shapes`, harness
false, `tools/sync/Cargo.toml:30-33`). `gircap`: 78 `.rs`, `src/lib.rs` (123) with 21
module roots, 10 integration binaries. `sled2redb`: 3 `.rs`, 1 integration binary.

**F24 — `girsync` leaves and cache concentration.** True acyclic production leaves:
`hash/*` ~941 lines (+ 1,036-line test file), `rw/*` ~628, `filter.rs` 405,
`hardlink.rs` 171, `link.rs` 218, `scan.rs` 373. The cache subtree is ~6,710 lines
(`cache/open.rs` 2947, `cache/db/cachedb.rs` 1507, `cache/backup/publish.rs` 877,
`cache/write.rs` 568, `wire.rs` 241, plus small `schema`/`lock`/`db` files) and owns
`redb`, `postcard`, `tempfile`. `commands/sync/*` is ~7,625 lines (~6,413 production,
including `plan/tests.rs` 1212). `planner` (~2,161) and `effective` (~2,593) are a
cycle and would move together. Sources: per-file `use crate::` edges, e.g.
`cache/open.rs:126-143`, `cache/backup/publish.rs:21-29`, `effective/phase_c.rs:13-21`.

**F25 — `gircap` leaves and subsystem sizes.** Production leaves: `encoding.rs` 54,
`paths.rs` 507, `pathmap.rs` 156, `rejections.rs` 331, `hash/*` ~1,146, `exit/mod.rs`
144, `logging/mod.rs` 249, `report/*` ~1,300, `tasks.rs` 223. Subsystems: `http/*`
~5,376 (3,462 production; owns `reqwest`, `wiremock` in tests, `thiserror`), `emit/*`
~6,015 (owns `flate2`, `walkdir`, `chrono`, `ini_core`), `blobs/*` ~4,851 (3,359
production; `zstd`), `manifest/*` ~4,307 (3,778 production; `prost`, `zstd`; `diff.rs`
1546, near-leaf `proto.rs` 376), `verify/*` ~4,365 (2,974 production; top-level, no
network), `select/*` ~3,365, `assemble/*` ~3,049, `hyp/*` ~2,101, `cli/mod.rs` 936 +
`config/mod.rs` 771. Cycles: `http/probe.rs:32` ↔ `biz/mod.rs:26`, `cli/mod.rs:135` ↔
`config/mod.rs:17`. `config/mod.rs:286` uses `reqwest::Url`, so an `http` split alone
does not remove reqwest from the whole tool graph.

**F26 — `sled2redb` depends on the whole `girsync` lib.** `tools/sled2redb/Cargo.toml:29`
depends on the `girsync` lib; `lib.rs:36-39` uses `girsync::report`, `main.rs:58-61` uses
`girsync::init_tracing`, `convert.rs:31-38` uses `girsync::hash::DigestSet`. Any girsync
source edit recompiles sled2redb. `sled` is isolated to this package (`convert.rs:28-29`).

**F27 — Heavy deps group by subsystem, but the facades defeat isolation.** `redb`/
`postcard`/`tempfile` → girsync cache; `tokio`/`rayon` → girsync `rw` (though `rayon`
also appears in `effective/phase_c.rs` and `apply/phases.rs`); `reqwest`/`wiremock` →
gircap `http` (but `config` also uses `reqwest::Url`); `zstd`/`prost` → gircap
`manifest`/`blobs`/`hash`; `flate2`/`walkdir`/`chrono`/`ini_core` → gircap `emit`/
`verify`. Because each tool has one lib crate depending on and re-exporting all of them,
an edit in any subsystem still rebuilds that lib and every test binary (A2).

**F28 — Embedded tool test code.** Nine `#[cfg(test)]` files in gircap total ~9,646
lines (`http/tests.rs` 1914, `blobs/tests.rs` 1492, `verify/tests.rs` 1391,
`assemble/tests.rs` 1292, `select/tests.rs` 1097, `config/tests.rs` 755,
`report/tests.rs` 600, `hash/tests.rs` 576, `manifest/tests.rs` 529) plus ~10.7k inline
test lines (largest tails `emit/index.rs:695` ≈1,365, `manifest/diff.rs:795` ≈845,
`verify/materialize.rs:394` ≈807). girsync has `commands/sync/plan/tests.rs` (1212) and
`hash/tests.rs` (1036) plus ~8.9k inline test lines (`cache/open.rs:1332` ≈1,851,
`cache/db/cachedb.rs:804` ≈792). Net: ~48% of gircap `src/` and ~40% of girsync `src/`
is test code. They cannot move out as-is: gircap `verify/mod.rs:62-76` and
`assemble/mod.rs:24-36` keep private `#[cfg(test)] use` names for `use super::*`;
`http/mod.rs:36-44` has `pub(crate)` test-only re-exports; girsync `cache/mod.rs:106-115`
and `cache/open.rs:150-155` expose test-only seams. The `girpr-log` `test-support`
feature is the in-workspace precedent for promoting such seams.

### Test structure and dev-dependency consequences

**F29 — Inline duplication multiplies recompilation.** `tests/common/mod.rs` (1,438
lines: offline HoYoPlay/Sophon mock and fixture builders) is `mod common;`-inlined by 11
of the 14 root integration binaries, ~15.8k lines recompiled per lib change.
`tests/artifact_bridge/server.rs` (990) is included by both `artifact_bridge.rs:113`
and `artifact_server.rs:25` via `#[path]`. `tests/common/run_lock.rs` (220) is included
at `common/mod.rs:120`, `artifact_bridge.rs:111` and `run_lock.rs:13`;
`tests/common/digest.rs` at `common/mod.rs:252`, `artifact_bridge.rs:109`,
`artifact_server.rs:28`. `tools/capture/tests/common/mod.rs` (2,021 lines) is inlined by
8–10 gircap binaries; its refusal to share with the product mock (`:1-9`) protects
cross-tool independence but does not apply within gircap. `tools/sync/tests/common/mod.rs`
is now a thin re-export (`:1-14`) but still defines `run_cli`, `run_cli_with_hooks`,
`run_args`, `capture_plan` (`:27-60+`) compiled by all 16 girsync binaries.

**F30 — Binary coupling is deliberate and expensive.** `CARGO_BIN_EXE_girpr` is
referenced directly at `tests/common/mod.rs:149`, `tests/cli_contract.rs:49,459,954,
969,1123,1515`, `tests/process.rs:21`, `tests/repair_machinery.rs:261`,
`tests/artifact_bridge.rs:298,550`, and indirectly through `common::run_binary` by
`process`, `repair_post`, `repair_plan`, `repair_offline`, `repair_scan`. Cargo builds
the bin for those tests; a lib edit causes lib compile → bin recompile → bin link →
recompile/link of each binary-driven test. The `tokio/process` dev feature exists solely
for them (`Cargo.toml:138`) and forces the second tokio variant plus
`signal-hook-registry`. The rationale is recorded at `tests/common/mod.rs:141-147`
(only the binary proves a CLI flag reaches `HypClient`) and `tests/cli_contract.rs:4`.
The same pattern uses `CARGO_BIN_EXE_girsync`, `CARGO_BIN_EXE_gircap`,
`CARGO_BIN_EXE_sled2redb` in the tools (`A8` of the tool research).

**F31 — nextest filters are coupled to crate/test layout.** `.config/nextest.toml:106-214`
pins six override groups to `package(=girpr)` + `binary(=girpr)` or
`binary(=repair_post)`/`binary(=artifact_bridge)` and to exact paths such as
`util::ini::tests::config_bump_refuses_a_dangling_junction_game_dir`,
`repair::file::inplace::tests::…`, `repair::scope::report::tests::…` and
`repair::lock::tests::…`. Moving those files into a crate or out of `src/` changes the
binary and/or fully-qualified test name and invalidates the filters.

**F32 — Ignored suites and merge candidates.** `tests/artifact_bridge.rs` (862 lines)
plus `server.rs` (990) compiles a binary whose six `#[ignore]`d cases never run in the
default gate; its stated value is compile-time rot detection (`:27-74`, `:72-74`).
`tests/run_lock.rs` is a one-module wrapper around `common/run_lock.rs` (`:13`). girsync
has six subdirectory `main.rs` harnesses (`why`, `sync_plan`, `stat_trust`, `rw_threads`,
`lazy`, `hash_mode`) each linking the lib and recompiling the common runner. Merging
these reduces link count but increases recompiled test code per edit — the two goals
trade off (F7).

**F33 — io-profile residue.** `tests/scripts/io-profile/iot-bench/target/` exists on
disk (debug, doc, incremental caches) even though `README.md:56-59` directs builds to
`--target-dir target/io-profile/iot-bench-target`. It is untracked and ignored by
`tests/scripts/io-profile/.gitignore:1`; it is disk residue, not a workspace build cost.
`iot-bench/Cargo.toml:11-12` overrides only release `opt-level = 2`; its dev/incremental
settings are defaults, unlike the root profiles.

### Shared-crate split candidates and remaining duplication

**F34 — `girpr-digest` has visibly disjoint consumer halves.** Fan-in 4; sled2redb's
manifest declares only "hex decoding of legacy digest strings" and uses the hex API;
girsync keeps its own `AlgoSet`/`DigestSet` and maps at the call site (chiefly a
`stream.rs` consumer); root uses both `Md5` and the sink; gircap uses hex + `Sink`.
Today an edit to `crates/digest/src/stream.rs` (304 lines) rebuilds sled2redb although
it does not use it. `pub mod vectors` (test data) is a separate low-churn surface.
Sources: `tools/sled2redb/Cargo.toml:34-38`, `tools/sync/Cargo.toml:50-53`,
`crates/digest/src/lib.rs:18-20,220-279`.

**F35 — `girpr-log` split has low fan-out payoff.** `install.rs` (190) pulls
`tracing-appender`/`tracing-subscriber`; `level.rs` (120) is pure std; but all three
programs consume level + filter + plan + install, so splitting policy would not shrink
their rebuilds. The genuinely isolatable part (`test-support`) is already feature-gated
(`crates/log/Cargo.toml:15-19`, `crates/log/src/lib.rs:71-72`). `plan.rs`'s 741 lines are
~93 module-doc lines and ~374 test lines, leaving ~305 code lines.

**F36 — `girpr-tempdir` is the clear shared-crate split case.** The 707-line
`test_support` is a different concern from the 360-line `TempDir` policy, is not
feature-gated, and drags a whole program into four dev graphs (F3). Splitting would
change dependency edges rather than merely duplicate them.

**F37 — Leaves where further splitting would not help.** `girpr-exit` (72 lines, zero
deps, one macro), `girpr-cli` (42), `girpr-audio-language` (91, no deps), `girpr-proto`
(92), `girpr-filter` (132), `girpr-report-core` (215, all three consumers use both
exports), `girpr-retry` (233, both consumers need `Schedule` + `Sleep`), `girpr-model`
(250, clap behind a feature), `girpr-fs-identity` (236, both consumers use both APIs),
`girpr-fs-names` (246, no consumer uses a pure subset — only `canonical_no_verbatim`
pulls `dunce`).

**F38 — Duplication remaining outside any crate.** Atomic temp-then-publish:
`src/util/atomic.rs:62` (`write_atomic_with`), girsync's cache backup publish and
sled2redb's converter; a narrow leaf would need explicit `durable`/parent-creation/
replace-vs-noclobber parameters. `OutputFormat` is still declared three times
(`src/report/record.rs:34`, `tools/sync/src/report/mod.rs:60`,
`tools/capture/src/report/mod.rs:38`) while `girpr-cli` exports only `clap_error_code`
(`crates/cli/src/lib.rs:16`). `girpr_log::format_timestamp` exists at
`crates/log/src/plan.rs:328` but capture reimplements it at
`tools/capture/src/config/mod.rs:770`. API DTO field sets (`Wrapper<T>` and the
config/branch/build/manifest sets) remain two implementations with deliberately
different decoders (`num_from_string` vs `IntOrStr`) and URL joining. Chunk identity
`(md5, size)` remains in both girpr and gircap without a shared type. Bounded zstd
decode, in-capture validation triplication, product link helpers, locks and walkers
remain local; the last four are covered by A13.

**F39 — Documentation seams for any new crate.** `README.md:44-45` (crate table),
`README.md:390-422` (prose inventory), `tools/capture/docs/00-spec.md:42` (eleven-leaf
independence table), `tools/capture/Cargo.toml:1-44` (matching comment), and the
workspace member list at `Cargo.toml:17` must stay consistent. Additional files naming
shared crates: `docs/03-repair-rust-implementation-plan.md`,
`docs/06-artifact-bridge-spec.md`, `tools/sled2redb/README.md`, `tools/sync/README.md`,
`tools/capture/docs/02-plan.md`. The existing docs already under-describe sharing
(e.g. `girpr-filter` is shared by girpr and girsync but absent from the capture table).

### Findings — exclusions (duplicate-looking code that is not a turnaround candidate)

**F40 — Exclusions.** Log-capture shims that remain (`tests/common/mod.rs:835`,
`src/repair/phases/metadata.rs:559`) are intended thin delegates over
`crates/log/src/test_support.rs:72,96`, not un-extracted duplication. Capture's
`zstd_measure`/`zstd_measure_from` pair is deliberately duplicated for distinct error
messages (`tools/capture/src/hash/mod.rs:518-523`). Product junction/symlink support is
unit-test-only by design (`src/util/mod.rs:25-36`); lock policies, walker policies and
`rw` gates diverge by intent; the four fake HTTP servers encode different contracts and
the capture mock explicitly refuses to borrow from the product's. These are recorded as
A13 and the prior report's N1–N10; none is a crate-split candidate for rebuild scope.

**F41 — Scope of validation.** Every item above is a static claim derived from the
current tree and git history. No item has been implemented, measured in this session, or
tested; all are subject to validation during implementation.
