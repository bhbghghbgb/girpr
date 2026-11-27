# Shared-crate ownership research — assumptions and findings

Research date: 2026-11-28. Source state: `main` at `47d4aed0`. Scope: every cargo
workspace member — the product (`girpr` at the root plus `app/*`), the capture
tool (`tools/capture*`), the mirror tool and converter (`tools/sync*`), and every
crate under `crates/`. Method: static survey of every `Cargo.toml` path edge
plus `use girpr_*` sites in source; no code was changed, no build was run and no
implementation was attempted.

This document contains **only assumptions and findings**. It is not an
implementation plan: it does not order work, group findings into tasks or
prescribe a sequence.

## Assumptions

- **A1 — Static reading only.** Every claim is a reading of the tree at the commit
  above. A closer read of a call site, a feature gate or a type detail could
  invalidate a finding. Nothing here is validated by `cargo check`/`nextest`.
- **A2 — Project boundary.** Three projects are in scope: `girpr` (root package,
  `src/`, `tests/` and `app/*`), `gircap` (`tools/capture` and its three crates
  under `tools/capture/{http,hash,test-support}`), and `girsync` (`tools/sync`
  and its four crates `cache`, `cache-json`, `leaves`, `test-support`). The
  converters and tool crates are counted as part of the tool project they sit
  under, not as separate projects.
- **A3 — Ownership signal.** The workspace's own convention is visible in
  `tools/capture/Cargo.toml:90-96`: crates with a `gircap-` prefix live under
  `tools/capture/` and "hold gircap's own mechanics and test fixtures"; `crates/`
  is described there as the convention "for crates shared between programs".
  `app/*` is the product's own crate root (`README.md:47-94`, `docs/03:18-64`).
  An earlier ownership report (added in `1b4832b7`, removed in `e9bf35a9`) stated
  the rule verbatim: "A crate is *shared* when it has a direct consumer in at least
  two distinct program families … and *project-owned* when all consumers are in
  one family. Dev-dependency edges count as real consumers", and "location and the
  `girpr-` prefix are not evidence of actual sharing". A crate in `crates/` whose
  consumers all belong to one project is therefore an ownership/location mismatch
  under that convention.
- **A4 — Direct consumers decide sharing.** A crate is counted as genuinely
  cross-project when it has at least one *direct* path consumer in two distinct
  projects. Transitive reach through a shared crate is not enough: for example,
  `girpr-tempdir` is used by all three projects, but that does not make a leaf it
  happens to depend on cross-project.
- **A5 — Test-only consumers count, with their project attributed by use.**
  A dev-dependency from a project's test target is a real consumer; a dependency
  used only by one project's fixture helper inside an otherwise shared crate is
  attributed to that project. This is how `girpr-cache-names` is attributed to
  `girsync` (see F4).
- **A6 — Policy at call sites, mechanics in leaves.** This is the house style
  (`crates/report-core/src/lib.rs:1-22`, `crates/value-ranges/src/lib.rs:1-70`).
  A leaf that owns one rule, flag spelling or published byte shape is treated as
  carrying program policy even if it is otherwise logic-free; this affects the
  ownership reading of `girpr-cache-names`.
- **A7 — Published bytes are contracts.** `--help` text, stdout record order,
  exit codes, bundle and NDJSON schemas, golden snapshots and file-name
  reservations are program contracts (`README.md:149-203`, `REPAIR_FLOW.md`).
  A relocation or rename is assumed to preserve every published byte unless a
  finding says a spelling is itself the subject of the finding.
- **A8 — Comments claiming "shared" are evidence of intent, not proof of use.**
  Several manifests and module headers say a crate is shared by programs that
  do not currently depend on it (`crates/test-http/Cargo.toml:15-20`,
  `crates/line-reader/Cargo.toml:6`, `README.md:53,57,93`). Consumer edges are
  treated as authoritative over prose.
- **A9 — Prior decisions are evidence of intent.** The tree still carries the
  rationale for several boundaries in comments; the deleted refactor package
  (`dfaca9da`) recorded contemporaneous decisions about relocating leaves
  (F27, F29, F52 there: product crates used by tools, and the test-http
  re-export, were deliberately left as they are). An earlier ownership report
  (`1b4832b7`) and its implementation commits (`8d16a9d8`, `29154608`,
  `1aa45d2b`, `835c0c1a`, `e875e6f3`) applied the same rule once already by
  moving single-program crates out of `crates/`. These are read as intent at
  that time, not as a ban on re-examining them.
- **A10 — Renames are independent of moves.** Package name, lib target name,
  directory and path dependency are separate fields. A rename that is not a
  move is only recorded where it would make the ownership signal in A3
  consistent; no rename is assumed to be required for a leaf that is genuinely
  shared.
- **A11 — Workspace plumbing follows the member list.** All packages are
  workspace members with `[workspace.package]`/`[workspace.dependencies]`
  inheritance (`Cargo.toml:40-58`). Any relocation is assumed to update the
  `members` list, every path dependency and `Cargo.lock` together; the finding
  itself does not enumerate those edits.

## Findings

### F1 — Inventory: which `crates/` leaves are actually cross-project

Every crate under `crates/` with its direct consumers grouped by project.
"girpr-only" and "girsync-only" rows are the ownership mismatches; the rest are
genuinely shared.

| crate | direct consumers | projects | reading |
|---|---|---|---|
| `async-join` | `girpr`, `app/util-path`, `tests/cli_contract.rs`, `gircap-http` | girpr, gircap | shared |
| `audio-language` | `girpr`, `app/audio`, `gircap` | girpr, gircap | shared |
| `cache-names` | `girsync`, `girsync-cache`, `girpr-tempdir` (test helper only) | girsync | **girsync-only** |
| `chunk-cost` | `girpr`, `app/{chunk-verify,repair-plan}`, `gircap`, `gircap-test-support` | girpr, gircap | shared |
| `chunk-identity` | `app/fetch`, `gircap` | girpr, gircap | shared |
| `chunk-layout` | `girpr`, `app/{fetch,repair-plan}`, `gircap` | girpr, gircap | shared |
| `cli` | `girpr`, `gircap`, `girsync`, `girsync-cache-json` | all three | shared |
| `digest` | `girpr`, `app/*`, `gircap`, `gircap-{hash,http}`, `girsync-cache`, `girsync-cache-json` | all three | shared |
| `exit` | `girpr`, `gircap` | girpr, gircap | shared |
| `filter` | `girpr`, `app/repair-plan`, `girsync`, `girsync-leaves` | girpr, girsync | shared |
| `fs-identity` | `girpr`, `app/lock`, `girsync-cache`, `girsync-leaves` | girpr, girsync | shared |
| `fs-names` | `girpr`, `gircap`, `girsync`, several `app/*` and `crates/*` | all three | shared |
| `hyp-wire` | `app/fetch`, `gircap`, `gircap-http` | girpr, gircap | shared |
| `line-reader` | `app/{audio,dispatch,persistent-plan,util-ini}` | girpr | **girpr-only** |
| `log` | `girpr`, `gircap`, `girsync`, `girsync-test-support`, `app/*` | all three | shared |
| `loopback-mock` | `girpr`, `app/fetch`, `gircap`, `gircap-http` | girpr, gircap | shared (test-only) |
| `model` | `girpr`, `app/{dispatch,fetch}`, `gircap`, `app/test-fixtures` | girpr, gircap | shared |
| `path-shape` | `girpr`, `app/util-path`, `gircap` | girpr, gircap | shared |
| `proto` | `girpr`, `gircap`, `app/{fetch,chunk-verify}`, `app/test-fixtures` | girpr, gircap | shared |
| `report-core` | `girpr`, `app/report`, `gircap`, `girsync` | all three | shared |
| `request-url` | `app/{dispatch,fetch,persistent-plan}`, `gircap`, `gircap-test-support` | girpr, gircap | shared |
| `retry` | `girpr`, `app/fetch`, `gircap`, `gircap-http` | girpr, gircap | shared |
| `retry-driver` | `app/fetch`, `gircap-http` | girpr, gircap | shared |
| `tempdir` | every project and most of their crates (dev-dependency) | all three | shared (test-only) |
| `test-http` | `girpr` only (`src/test_origin.rs`) | girpr | **girpr-only** |
| `time` | `girpr` (`app/lock`, `crates/log`), `gircap`, `gircap-http`, `girsync-cache` | all three | shared |
| `value-ranges` | `girpr`, `gircap`, `girsync` | all three | shared |

The full edge list was read from the `[dependencies]` and `[dev-dependencies]`
tables of all 53 workspace manifests and cross-checked against
`cargo metadata --no-deps --format-version 1` (the two agree); the source-side
check (`use girpr_*`) found no consumer outside those edges.

### F2 — `crates/line-reader` has no consumer outside girpr

- Every direct consumer is a product crate: `app/audio/Cargo.toml:29`,
  `app/dispatch/Cargo.toml:34`, `app/persistent-plan/Cargo.toml:26`,
  `app/util-ini/Cargo.toml:29`; source uses at
  `app/audio/src/lib.rs:30`, `app/dispatch/src/proto.rs:11`,
  `app/persistent-plan/src/manifest.rs:14`, `app/util-ini/src/ini.rs:8`.
- Neither `tools/capture*` nor `tools/sync*` has a path edge to it, and no
  `girpr_line_reader::` import exists under `tools/`.
- Its own description says "The **shared** BOM-stripping, 1-based,
  blank-skipping line iterator" (`crates/line-reader/Cargo.toml:6`) and its
  header says "the workspace's text readers share" (`crates/line-reader/src/lib.rs:1-9`),
  but the only readers that actually use it are girpr's own.
- The capture-side duplicate identified in the earlier research remains
  capture-local: `tools/capture/src/emit/gamedir.rs` spells its own first-match
  `[General]` scan, and `gircap` depends on `girpr-util-ini` only for the pure
  `game_version_from_text` helper (`tools/capture/Cargo.toml:282-285`).
- Creation intent was cross-project: `08d7fea0` — "share BOM/blank lines and
  parameterized audio/ini renderings"; the realized direct consumers are all
  girpr crates. The earlier review declined moving the remaining shared helper
  to a lower leaf (historical F27 in `dfaca9da^:REFACTOR_PLAN/STATE.md`).
- Ownership reading: `girpr`; location under the shared-crate root is
  inconsistent with A3.

### F3 — `crates/test-http` has no consumer outside girpr

- The only path dependency is the root package's dev-dependency
  (`Cargo.toml:349-351`); the only source uses are in the product's test-only
  module, `src/test_origin.rs:22` (`pub(crate) use girpr_test_http::{InFlight,
  Server as TestServer}`) and `:129` (`girpr_test_http::serve(...)`).
- The manifest claims "the repair facade's route origin, the capture mock and
  the fixture servers all need the same socket/stop/gate mechanics"
  (`crates/test-http/Cargo.toml:15-20`) and the module header repeats that
  "every axum test server in the workspace" uses it
  (`crates/test-http/src/lib.rs:1-7`). No such edge exists: the capture mock and
  fixture servers use `girpr-loopback-mock`/local helpers instead, and the leaf's
  own re-export was declined in the earlier review (historical F52 in
  `dfaca9da^:REFACTOR_PLAN/STATE.md`: "if a third axum server adopts it").
- The two other axum servers named in the manifest use *different* mechanics:
  the artifact replay server's custom accept loop
  (`app/test-fixtures/src/artifact_server.rs`) and capture's synchronous mock
  (`tools/capture/test-support/src/lib.rs`). The wiremock-based
  `girpr-loopback-mock` is the leaf shared by girpr and gircap
  (`Cargo.toml:342-345`, `tools/capture/Cargo.toml:321-326`).
- Creation intent was cross-project: `d3165243` — "share axum socket/stop/gate
  mechanics", with the plan step "adopt capture/test-fixture axum scaffolds".
  Those adoptions were declined (historical F50, F51 and F52 in
  `dfaca9da^:REFACTOR_PLAN/STATE.md`; F52 says "if a third axum server adopts
  it"), leaving one direct consumer.
- Ownership reading: `girpr`, test-only; location under the shared-crate root is
  inconsistent with A3.

### F4 — `crates/cache-names` is girsync's reservation, with one test-only leak

- Production consumers are girsync's own crates: `tools/sync/Cargo.toml:56`
  and `tools/sync/cache/Cargo.toml:57`, with source uses at
  `tools/sync/src/util.rs:35,164` and `tools/sync/cache/src/schema.rs:15`.
  No gircap crate and no girpr production crate depends on it.
- The remaining consumer is `crates/tempdir/Cargo.toml:30`, used at exactly one
  site: `crates/tempdir/src/test_support.rs:327` filters `girpr-cache*` out of
  `entry_names`. That helper's only callers are girsync tests
  (`tools/sync/tests/sync.rs:10,393,400,516,...` and
  `tools/sync/tests/symlink.rs:24,894,947`). The other four `entry_names_with`
  callers pass `|_| true` (`src/repair/phases/post.rs:868`,
  `src/repair/scope/report.rs:528`, `app/util-path/src/atomic.rs:254`,
  `app/util-ini/src/ini.rs:652`).
- The crate documents itself as girsync state: "girsync reserves the whole
  `girpr-cache*` prefix for its own state" (`crates/cache-names/src/lib.rs:1-8`),
  including the Windows-only ASCII-case rule that was adopted by production
  only in the recent batch B6 (`dfaca9da^:REFACTOR_PLAN/STATE.md`, F46).
- The reason it sits in `crates/` is a dependency-inversion workaround:
  `crates/tempdir/Cargo.toml:31-35` says it is deliberately girsync-free, and
  `crates/cache-names/Cargo.toml:15-17` says the leaf exists because tempdir
  cannot depend on `girsync-cache`. The workaround puts a girsync-owned
  file-name policy in the shared root and makes every project that uses
  tempdir compile it, even though only a girsync test helper reads it.
- Creation was explicitly build-driven: `1e8f305e` — "add a std-only
  `girpr-cache-names` leaf and route tempdir's `entry_names` filter through it",
  with "blast radius is tests only". Production adoption came later in
  `db5caaa8` (batch B6 / F46) but stayed inside the girsync family. This crate
  was added *after* the earlier ownership cleanup, so it is a new exception
  rather than a leftover.
- Ownership reading: `girsync`; location under the shared-crate root is
  inconsistent with A3, and the tempdir edge is the constraint to validate (A5,
  A11). The `girpr-` package/lib prefix also conflicts with the tool-crate
  convention (`gircap-http`, `girsync-cache`, `girsync-leaves`).

### F5 — The remaining shared crates have real cross-project consumers

The other 24 entries in the F1 table each have a direct edge from two projects
or from all three. Representative evidence:

- gircap's manifest comment enumerates the shared path leaves it consumes,
  pairing each with the policy it deliberately keeps local
  (`tools/capture/Cargo.toml:8-61` and the per-dependency comments at `:133-296`).
- girsync consumes `log`, `cli`, `fs-names`, `cache-names`, `value-ranges`,
  `filter`, `report-core` and `util-path` in its lib (`tools/sync/Cargo.toml:38-108`)
  and `tempdir`/`log` in its tests (`:112-139`).
- `girpr-retry-driver` and `girpr-async-join` are consumed by the *production*
  fetch loops of both projects (`app/fetch/src/retry.rs:21`,
  `tools/capture/http/src/fetcher.rs:12`; `src/repair/{scan.rs,mod.rs,file/mod.rs}`
  and `tools/capture/http/src/tasks.rs:14-68`), not by tests alone.
- `girpr-hyp-wire`, `girpr-request-url`, `girpr-model` and `girpr-proto` are
  consumed by both projects' API/decoding paths (`app/fetch/src/lib.rs`,
  `tools/capture/src/biz/mod.rs`, `tools/capture/http/src/lib.rs`).
- No crate under `crates/` is unused; no crate under `crates/` is consumed by
  gircap alone.

### F6 — Two product-owned crates are consumed by other projects (inverse direction)

- `app/util-path` is a girpr project crate (`README.md:87`) but has direct
  consumers in girsync: `tools/sync/Cargo.toml:97`,
  `tools/sync/cache/Cargo.toml:35`, `tools/sync/cache-json/Cargo.toml:36`.
  The earlier research recorded this as F29 ("three `tools/sync` crates now
  depend on it") and declined relocating the claim primitive to a lower crate;
  the cross-project edge itself was left in place.
- `app/util-ini` is a girpr project crate (`README.md:92`) but gircap depends on
  it for `game_version_from_text` (`tools/capture/Cargo.toml:282-285`). The
  earlier research recorded this as F27 ("gircap no longer depends on an `app/`
  crate that also owns the repair config writer") and declined it.
- These are the mirror image of F2-F4: cross-project crates living inside a
  project's crate root. They are findings about the current structure; they are
  not candidates for the "move a shared crate into its owning project" class.

### F7 — Documentation and naming drift around the shared root

- `README.md:53,57,93` list `girpr-cache-names`, `girpr-test-http` and
  `girpr-line-reader` in the inventory whose stated convention is a map of
  "entry points" for the product and shared leaves; all three rows were added
  by the earlier documentation batch (historical F26/F53 in `dfaca9da^`).
- `crates/test-http/Cargo.toml:6,15-20` and `crates/test-http/src/lib.rs:1-7`
  overstate the set of consumers; `crates/line-reader/Cargo.toml:6` and
  `crates/line-reader/src/lib.rs:1-9` overstate it too.
- `crates/cache-names` is named with the product's `girpr-` prefix even though
  every production consumer is in `tools/sync`; the three crates that do own
  gircap/girsync mechanics use `gircap-`/`girsync-` prefixes and live under
  their tool's directory (`tools/capture/Cargo.toml:90-96`, `README.md:51-55`).
  The `girpr-cache` string inside the crate is a *file-name* reservation
  (`crates/cache-names/src/lib.rs:19`) and is independent of the crate's name.
- `docs/03-repair-rust-implementation-plan.md:24-30` already distinguishes
  `crates/log`/`crates/tempdir` as shared from `tools/sync/test-support` as the
  place that "keeps `crates/tempdir/` girsync-free" — the same invariant that
  `crates/cache-names` was introduced to preserve.
- The same documentation batch (`8e9cacc3`) did not add inventory rows for
  `async-join`, `chunk-layout`, `retry-driver` or `loopback-mock`, although F1
  shows all four are genuinely shared; conversely it did add "shared" rows for
  the three single-project crates. `tools/capture/docs/00-spec.md:42` names an
  older shared-leaf set and omits the later direct leaves `request-url`,
  `hyp-wire`, `async-join`, `retry-driver` and `chunk-layout`, which
  `tools/capture/Cargo.toml` depends on directly.

### F8 — Rename surface, if consistency is the goal

- A package rename changes the `[package] name`, `[lib] name`, every path
  dependency key and the `use` crate path. `girpr-cache-names` is the only
  single-project crate in F1 whose prefix names another project; its consumers
  are all girsync-side, so a `girsync-` prefix would match A3 without changing
  the `girpr-cache` file-name reservation.
- `girpr-line-reader` and `girpr-test-http` already carry the product's prefix,
  which matches every other crate under `app/`; a rename is not indicated by
  the ownership evidence alone.
- The genuinely shared leaves in F5 already use the `girpr-` prefix uniformly;
  no ownership-driven rename is indicated for them.

### F9 — An earlier ownership cleanup already applied this exact rule

- `1b4832b7` ("docs: report shared-crate ownership assumptions and findings")
  surveyed 35 crates under `crates/` and classified 19 as shared, 13 as
  girpr-only and 3 as girsync-only under the same direct-consumer rule as A3/A4
  here; `e9bf35a9` removed the report after implementation.
- Its findings called the three girsync-only crates "the clearest naming/location
  mismatch" and the 13 girpr-only crates "build-time split leaves, not shared
  leaves", and defined the consistent identity as owning-project prefix under
  the owning project's directory.
- The implementation moved the 13 girpr crates into `app/`: `8d16a9d8` for
  `chunk-verify`, `fetch`, `lock`, `repair-plan` and `report`; `8ddd15cf` for
  `audio`, `repair-policy`, `util-counter`, `util-hash`, `util-ini` and
  `util-path`; `29154608` for `test-fixtures` and `test-support`. It moved and
  renamed the girsync crates
  into `tools/sync/` (`1aa45d2b` → `girsync-cache`, `835c0c1a` →
  `girsync-leaves`, `e875e6f3` → `girsync-test-support`).
- The three mismatches in F2-F4 were introduced afterwards (`1e8f305e`,
  `d3165243`, `08d7fea0`) and are now the only crates under `crates/` whose
  direct consumers do not span two projects. The earlier cleanup is therefore
  precedent for both the location and the prefix part of the convention in A3.
- The temporary report `d5ce501c`/`41e12a45` ("shared-crate generalization
  research") separately recorded that a split only pays off when high-level
  consumers depend on the narrow crates directly — consistent with the
  direct-edge rule used here rather than transitive reach.
