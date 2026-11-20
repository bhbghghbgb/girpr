# Do not touch (or touch only as a task says)

These findings were examined and deliberately kept, or are local/breakage-prone
duplication that is **not** a shared-crate extraction. Before editing anything,
a session should confirm its target is not on this list. Full reasoning is in
the report; only pointers are given here.

## Deliberately kept — changing them is out of scope

- **All of report §N** (lines 396–442), the negative findings:
  - N1 INI parsers (398–401) — last-match/no-section vs first-match/`[General]`
    are intentional (§A6); capture already deduplicated its own reader.
  - N2 limits/counters/concurrency gates (402–406) — no second consumer or
    explicit disclaimers; the `rw` gate is a different primitive (§F14).
  - N3 exit-code tables (407–408) — capture's values/meanings are its own (§A9).
  - N4 progress/stream reporting (409–411) — no counterpart; `girpr_log::timestamp`
    is already shared.
  - N5 `DigestSet`/`AlgoSet`/`HashAlg` and girpr's range scanner (412–419) —
    forward-compatible raw-byte vocabulary pinned by byte tests; do not fold
    into a shared digest type (§A7).
  - N6 the deleted report's "hand-rolled code that should stay" (421–425):
    `rw` gate, FNV lock key, one-syscall/fail-fast cache open, symlink
    classification, bounded task runner, `_tmp` resume artifacts, Starward and
    Collapse parity rules.
  - N7 fake HTTP servers (427–430) — four independent fixtures; artifact-bridge
    spec forbids sharing code that would couple the tools.
  - N8 capture's artifact writer (432–434) — named temp slots and deliberately
    non-atomic documents are part of the byte-stable artifact contract (§A3).
  - N9 URL/status helpers and retcode policy (436–439) — deliberately distinct.
  - N10 `zstd_measure` / `zstd_measure_from` (441–442) — deliberately duplicated
    for distinct error messages.
- **Report §A6** (52–60): divergent retry jitter, INI matching, exit tables and
  report formats are intentional; extraction must not "fix" them.
- **Report §A7** (62–67): do not collapse girpr's strict `Md5`, girsync's raw
  bytes + foreign names, and gircap's lowercase hex strings into one type.
- **Report §A8** (69–76): do not merge the three report data planes, enable
  `preserve_order`, or render booleans where the data plane forbids them.
- **Report §A9** (78–81): exit codes are separate contracts; only the table
  *mechanism* (T13) is shareable.
- **Report §A10** (83–88): the device-name difference is a defect *candidate*,
  not a verdict. T03 shares the rule mechanism without changing either
  predicate's behavior. Any behavioral change needs confirmation first and may
  move bundle bytes — raise a follow-up instead.
- **Report §A11** (90–94): `girpr-tempdir`'s non-dev dependency on girsync and
  serde_json is a real inversion; account for it, do not assume every leaf is
  program-independent.

## Local / not a shared-crate task

These are duplication findings whose boundary stays inside one crate, or where
a shared crate was judged low-value. They are not in the task list; raise a
follow-up rather than expanding a task into them:

- **Report §F12** (218–224) — file locking: only `try_lock` plus error
  classification is common; policy divergence makes a crate low-value.
- **Report §F13** (226–231) — directory walkers: policies are incompatible;
  the reusable pieces are already tasks T03–T05, T06, T19.
- **Report §F14** (233–237) — `rw` concurrency control has no product
  counterpart and is deliberately not shared (also §N2/N6).
- **Report §F29** (379–386) — within-capture validation triplication: same
  crate, different message vocabularies; not a cross-crate extraction. The
  paired `zstd_measure*` loops must stay duplicated (§N10).

If a session believes a "keep" item should be changed, it must stop and raise a
follow-up rather than expanding its task's scope.
