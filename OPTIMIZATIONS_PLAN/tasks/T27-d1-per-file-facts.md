# T27 - D1: give each planned file its facts instead of parallel side tables

Report: `../../OPTIMIZATIONS_REPORT.md` lines 619-643 (D1).

Dependencies: T15 (shared local map), T16 (scan reuse). This is a broad
refactor; review it alone.

## Goal

The same rel string keys `RepairPlan::url_prefix_by_file`,
`scope::report::DiskPlan::per_file`, `scan::ScanReport::files`, the
`PlanDocument` entries, the purge expected set, and the local reuse map.
Every producer must clean the path the same way, every consumer pays a String
hash, and a spelling difference is a silent miss. New per-file facts become
new parallel maps.

Choose one convention and apply it consistently. The report offers fields on
`PlannedFile` or a dense `FileId`; do not end up with both a field and a map
for the same fact.

## Where to start

- `src/repair/phases/plan.rs`: `PlannedFile` (around line 30), `RepairPlan`
  (around line 151), `url_prefix_by_file`, `build_plan` (around line 281).
- `src/repair/scope/report.rs`: `DiskPlan::per_file` (around line 67),
  `DiskPlan::new` (around line 79), `for_file` (around line 146),
  `PlanDocument` (around line 299), `write_to` (around line 351).
- `src/repair/scan.rs`: `ScanReport.files` (around line 57),
  `est_download_bytes_by_file` (around line 99), `scan_plan` (around line 218).
- `src/repair/file/mod.rs`: `prefix = plan.url_prefix_by_file.get(...)`
  (around line 448).
- `src/repair/purge.rs`: `expected_rels` (around line 104) and the local
  reuse lookups.
- Test constructors: `rg -n "url_prefix_by_file|per_file:|ScanReport \{" src tests`.

## Recommended convention

Use **fields on `PlannedFile` for plan-static facts, and a dense index for
run-computed tables**:

1. Move the URL prefix onto `PlannedFile` as `pub url_prefix: Arc<str>` and
   remove `url_prefix_by_file`. `build_plan` already has the prefix per
   manifest; set it when the file is built. The file writer reads
   `file.url_prefix` directly.
2. Add a `FileId` newtype (index into `RepairPlan::files`) plus
   `RepairPlan::file(id)`/`len`/`iter`. `build_plan` sorts, so ids are assigned
   after the sort.
3. Change `DiskPlan::per_file` from `HashMap<String, FileDisk>` to
   `Vec<FileDisk>` aligned with `plan.files`. `DiskPlan::new` pushes in plan
   order; `for_file(plan, rel)` or `for_id(FileId)` indexes directly. Update
   `PlanDocument` to iterate `plan.files` by index, so the document's order is
   the plan's order.
4. Change `ScanReport::files` from `HashMap<String, Verdict>` to
   `Vec<Option<Verdict>>` aligned with `plan.files` (or a `Vec<Verdict>` plus
   a `scanned` bitmap). `scan_plan` already has the index; store by index.
   `est_download_bytes_by_file`, check-only, and T16's guard look up by
   index. Keep a `get(FileId)` accessor; do not keep the rel-keyed map as a
   second copy.
5. Purge: keep `expected_rels()` derived from `plan.files[*].rel`. Do not add
   a field for the key; the purge folds it into its keyed set once (T24).
6. Local reuse map: it is keyed by the *local manifest's* rel, not the plan's
   file id, so it cannot become a field without joining the two manifests at
   plan time. Leave it keyed by `rel` and document why: it is a producer-owned
   lookup, not a per-file fact. If you can attach `Option<FileReuse>` to
   `PlannedFile` cheaply, do so; otherwise keep one documented exception.
7. Update every test constructor. Add `..Default::default()` only if the type
   can derive a sensible default; do not make `url_prefix` silently empty in
   production. A test-only helper may fill it.

## Tests

- All existing plan, scan, disk, purge, and repair tests pass with the new
  types.
- Plan order is unchanged (sorted by rel); `--plan-out` entries are in the
  same order with the same fields.
- A duplicate rel/differing spelling still dedups exactly as before.
- `ScanReport` and `DiskPlan` accessors return the same values for a plan with
  files that fail to stat/scan (the `Option` cases).
- `cargo test -p girpr` passes.

## Acceptance

- One source of truth for the URL prefix (the file field).
- `ScanReport` and `DiskPlan` are index-aligned with the plan, not rel-keyed.
- No fact exists as both a field and a parallel map.
- The local reuse map is either integrated or documented as the one exception.
- `cargo test -p girpr` passes.

## Commit plan

1. `refactor(plan): put the URL prefix on the planned file`
2. `refactor(report): index the disk plan by file id`
3. `refactor(scan): index the scan report by file id`
4. `test: update constructors for the per-file facts`

## Notes

- Do not add a `FileId` that is just a `usize` alias in name; make it a real
  newtype so an index cannot be mixed with an offset or a chunk index.
- Keep debug output direct: a `FileId` should print its index, and logs should
  still carry the rel.
- T31/D5 and T34/D8 build on this; keep the refactor mechanical.
