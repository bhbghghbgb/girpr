# Shared-crate ownership research: assumptions and findings

This report is standalone. It contains only assumptions and findings; it is not a
plan and contains no implementation steps.

## Assumptions

- **A1 — Program boundaries.** The workspace contains four programs: `girpr` (the
  root package and shipped repair binary), `gircap` (`tools/capture`), `girsync`
  (`tools/sync`) and `sled2redb` (`tools/sled2redb`). The root README states
  "One policy, four programs". A crate is a *program family*, not a fifth program,
  when it exists only to split one of those four for build time or to support its
  tests.
- **A2 — `sled2redb` belongs to the girsync family.** It is described as "extracted
  from girsync", takes a normal dependency on `girsync`, and consumes
  `girpr-sync-cache` and `girpr-sync-test-support` directly. There is no consumer
  of the `girpr-sync-*` crates outside `girsync` + `sled2redb`. Therefore a crate
  consumed only by those two is single-project (girsync-owned), not shared.
- **A3 — Ownership rule.** Direct Cargo dependency edges in the manifests are the
  authority for "consumer"; source `use` statements were checked and agree. A
  crate is *shared* when it has a direct consumer in at least two distinct
  program families (A1–A2), and *project-owned* when all consumers are in one
  family. Dev-dependency edges count as real consumers.
- **A4 — Project-owned crate convention already in the workspace.** A
  project-owned crate lives under the owning project's directory, carries the
  owning project's name prefix, and, for dev-tool-owned crates, uses the
  tools-only lint table. The three gircap-owned crates
  (`tools/capture/{http,hash,test-support}` = `gircap-http`, `gircap-hash`,
  `gircap-test-support`) are the in-repo precedent.
- **A5 — `crates/` is the shared-crate area by layout convention, but location
  and the `girpr-` prefix are not evidence of actual sharing.** A crate in
  `crates/` can be single-project; the consumer map (F1) decides.
- **A6 — Renames are treated as consistency consequences of A4, not as new
  policy.** No rename of a genuinely shared crate is required by the ownership
  rule; the report does not assume one.
- **A7 — A physical move changes more than the crate directory:** workspace
  `members`, dependent path dependencies, `Cargo.lock` package entries, and
  path text in READMEs and doc comments. Those edits are outside this report's
  scope (no plan is included), but their existence is a finding (F8).

## Findings

### F1 — Complete consumer map of `crates/` (35 members)

"Consumers" lists the direct manifest consumers; the family column applies A1–A3.

| Crate (`crates/…`) | Direct consumers | Family | Class |
|---|---|---|---|
| `audio-language` | root, `audio`, gircap | girpr + gircap | shared |
| `chunk-cost` | root, `repair-plan`, gircap | girpr + gircap | shared |
| `chunk-identity` | `fetch`, gircap | girpr + gircap | shared |
| `cli` | root, gircap, sled2redb | girpr + gircap + girsync-family | shared |
| `digest` | root, `chunk-identity`, `fetch`, `sync-cache`, `test-fixtures`, `util-hash`, gircap, `gircap-hash`, `gircap-http`, sled2redb | all four | shared |
| `exit` | root, gircap | girpr + gircap | shared |
| `filter` | root, `repair-plan`, `sync-leaves`, girsync | girpr + girsync | shared |
| `fs-identity` | root, `lock`, `sync-cache`, `sync-leaves` | girpr + girsync | shared |
| `fs-names` | root, `path-shape`, `util-path`, gircap, girsync | all four | shared |
| `log` | root, `report`, `sync-test-support`, `test-fixtures`, `util-ini`, gircap, girsync | all four | shared |
| `loopback-mock` | root (dev), `fetch` (dev), gircap (dev), `gircap-http` (dev) | girpr + gircap | shared (test-only) |
| `model` | root, `fetch`, `test-fixtures`, gircap | girpr + gircap | shared |
| `path-shape` | `util-path`, gircap | girpr + gircap | shared |
| `proto` | root, `chunk-identity`, `chunk-verify`, `fetch`, gircap | girpr + gircap | shared |
| `report-core` | `report`, gircap, girsync | all four | shared |
| `retry` | root, `fetch`, gircap, `gircap-http` | girpr + gircap | shared |
| `tempdir` | root, 10 other `crates/` members, gircap (3 of its crates), girsync, sled2redb | all four | shared (test-only) |
| `time` | `lock`, `log`, `sync-cache`, gircap, `gircap-http` | all four | shared |
| `value-ranges` | root, gircap, girsync | girpr + gircap + girsync | shared |
| `audio` | root | girpr only | project-owned (girpr) |
| `chunk-verify` | root | girpr only | project-owned (girpr) |
| `fetch` | root, `repair-plan`, `test-fixtures` | girpr only | project-owned (girpr) |
| `lock` | root, `test-fixtures` | girpr only | project-owned (girpr) |
| `repair-plan` | root, `test-support` | girpr only | project-owned (girpr) |
| `repair-policy` | root, `test-support` | girpr only | project-owned (girpr) |
| `report` | root | girpr only | project-owned (girpr) |
| `test-fixtures` | root (dev), `test-support` | girpr only | project-owned (girpr) |
| `test-support` | root (dev) | girpr only | project-owned (girpr) |
| `util-counter` | `report` | girpr only | project-owned (girpr) |
| `util-hash` | root, `chunk-verify`, `fetch` | girpr only | project-owned (girpr) |
| `util-ini` | root | girpr only | project-owned (girpr) |
| `util-path` | root, `audio`, `fetch`, `lock`, `repair-plan`, `util-ini` | girpr only | project-owned (girpr) |
| `sync-cache` | girsync, sled2redb | girsync family only | project-owned (girsync) |
| `sync-leaves` | girsync | girsync family only | project-owned (girsync) |
| `sync-test-support` | girsync (dev), sled2redb (dev) | girsync family only | project-owned (girsync) |

### F2 — Counts

35 crates in `crates/`: 19 shared, 13 girpr-only, 3 girsync-only. No crate in
`crates/` is gircap-only or sled2redb-only: gircap's project-owned crates have
already been moved out to `tools/capture/`.

### F3 — The three girsync-only crates are the clearest naming/location mismatch

`girpr-sync-cache`, `girpr-sync-leaves` and `girpr-sync-test-support` are consumed
only by girsync and its converter (F1), yet they use the shared `girpr-` prefix and
live in the shared `crates/` area. The gircap equivalents (A4) already follow the
project-owned convention. The ownership-consistent identities implied by A4 are:

| Current | Owning project | Consistent name | Consistent directory |
|---|---|---|---|
| `girpr-sync-cache` (`girpr_sync_cache`) | girsync | `girsync-cache` (`girsync_cache`) | `tools/sync/cache/` |
| `girpr-sync-leaves` (`girpr_sync_leaves`) | girsync | `girsync-leaves` (`girsync_leaves`) | `tools/sync/leaves/` |
| `girpr-sync-test-support` (`girpr_sync_test_support`) | girsync | `girsync-test-support` (`girsync_test_support`) | `tools/sync/test-support/` |

### F4 — The 13 girpr-only crates are build-time split leaves, not shared leaves

F1 shows their consumers are the root package and other girpr-only crates only.
The root README already describes them as "this binary's" mechanics
(`audio`, `chunk-verify`, `fetch`, `lock`, `repair-plan`, `repair-policy`,
`report`, `test-fixtures`, `test-support`, `util-counter`, `util-hash`,
`util-ini`, `util-path`), in contrast to the leaves it describes as shared. Their
`girpr-` prefix already names their owner, so A4 implies no rename for them; what
changes is their classification and, if `crates/` is to mean "shared only", their
placement in a girpr-owned subtree at the workspace root. This report does not fix
that subtree's name.

### F5 — The shared set is stable under the ownership split

None of the 16 single-project crates is consumed by gircap, and none of the 19
shared crates is consumed by only one family (F1). Moving or renaming the
single-project crates therefore does not remove the second consumer of any shared
crate. In particular, `girpr-fs-identity`, `girpr-filter` (girpr + girsync only),
`girpr-loopback-mock` (girpr + gircap, test-only) and `girpr-report-core`
(girpr + gircap + girsync) remain shared.

### F6 — Dependency-direction facts that hold today

- `girpr-test-support` depends on the root `girpr` package as a normal (test-only
  inversion) dependency; it cannot be placed where the product depends on it in a
  shipped build.
- `girpr-repair-plan` depends on `girpr-fetch`; both are girpr-only, so their edge
  stays inside one project under either layout.
- `girpr-sync-cache` is consumed directly by sled2redb, not through girsync's lib
  facade, and pins the digest vocabulary in its `postcard` wire format. Its
  direct consumer edge and wire vocabulary must survive any move.
- `girpr-sync-test-support` depends on girsync and is taken as a dev-dependency by
  girsync and sled2redb; that inversion must survive any move.
- `girpr-retry`, `girpr-log`, `girpr-time`, `girpr-digest` and `girpr-fs-identity`
  are dependencies of crates that would move; all five are shared and stay in the
  shared area.

### F7 — Lint-table difference between the two crate areas

Every `crates/` member sets `[lints] workspace = true`. The three gircap-owned
crates under `tools/capture/` set their own tools-only `[lints.rust]` /
`[lints.clippy]` tables (denied `unsafe_code`, `clippy::all` warning, no
`unwrap_used`/`expect_used`). Crates moved under `tools/sync/` for girsync
ownership follow the tools-only table under A4; girpr-owned crates keep the
workspace table.

### F8 — Blast radius of any move (text references found)

- The root `Cargo.toml` workspace `members` list enumerates all 35 `crates/`
  paths plus the tool paths; it is the single place that defines membership.
- Path strings for each of the 16 single-project crates occur in 2–8 files each
  (manifests, README.md, `tools/sync/README.md`, `tools/sled2redb/README.md`,
  and source doc comments), in addition to the dependency keys themselves.
- `Cargo.lock` records every package name; the three `girpr-sync-*` renames would
  change lockfile package entries.
- The root integration tests shim the two girpr test crates through
  `tests/common/mod.rs`; girsync and sled2redb shim `girpr-sync-test-support`
  through their own `tests/common/mod.rs`. The shims are thin and the crate
  identities behind them are what change.

### F9 — Explicit non-findings (to bound the implementation)

- No crate under `tools/capture/` is shared with another program: `gircap-http`,
  `gircap-hash` and `gircap-test-support` are already gircap-owned and correctly
  placed/named; they are not candidates.
- `girpr-audio-language` is shared because gircap uses it, even though the repair
  binary is the other consumer; it is not a girpr-only crate.
- `girpr-proto`, `girpr-model`, `girpr-chunk-identity`, `girpr-chunk-cost`,
  `girpr-exit`, `girpr-cli`, `girpr-value-ranges`, `girpr-retry`, `girpr-time`,
  `girpr-fs-names`, `girpr-path-shape` and `girpr-report-core` are all consumed by
  gircap and therefore cannot be classified as single-project.
- `girpr-lock` is consumed only by girpr (root + its test fixtures); the dev
  tools never take it, despite the "shared by the product and its integration
  tests" phrasing in its manifest description.
