# Do not touch (or touch only as a task says)

These findings were examined and deliberately kept, or are low-value/local
duplication that is **not** a crate-split task. Before editing anything, a
session should confirm its target is not on this list. Full reasoning is in the
report; only pointers are given here.

## Deliberately kept — changing them is out of scope

- **Report §A13 (lines 108–114) and §F40 (lines 456–466).** The prior report's
  N1–N10 are duplicate-looking code deliberately left local: INI parsers,
  limits/counters/gates, exit tables, progress reporting, `DigestSet` /
  `AlgoSet` / `HashAlg`, the `rw` gate, FNV lock key, fail-fast cache open,
  symlink classification, bounded task runner, `_tmp` artifacts, parity rules,
  the four fake HTTP servers, capture's artifact writer, URL/status/retcode
  helpers, and the `zstd_measure` pair. None is a split candidate. In
  particular:
  - Capture's `zstd_measure` / `zstd_measure_from` pair stays duplicated for
    distinct error messages (§N10 via §A13).
  - The four fake HTTP servers encode different contracts; the capture mock
    explicitly refuses to borrow from the product's (§F40).
  - Product junction/symlink support is unit-test-only by design (§F40).
- **Report §A6 (lines 55–61).** Byte contracts are load-bearing: proto field
  names `chuncks` / `unknown_field6`, no `preserve_order` in report-core, the
  pinned `girpr_log::format_timestamp`. A split must not change key order,
  casing, spacing or flag conditions.
- **Report §A5 (lines 48–53).** gircap independence is a hard boundary: no
  dependency on girpr, no invoking its binary, only logic-free leaves. Do not
  couple programs to make a split "share" more code.
- **Report §A10 (lines 86–94) and §F13 (lines 228–232).** Cuts through the
  named cycles are not structurally free. A boundary must sit at an acyclic
  leaf or around a whole strongly-connected group, with shared types moved down
  first. Report §F32 (lines 385–392) also records the trade-off when merging
  test harnesses: fewer links but more recompiled test code per edit.

## Leaves and splits where further work would not help

- **Report §F37 (lines 424–430).** Further splitting of `girpr-exit`,
  `girpr-cli`, `girpr-audio-language`, `girpr-proto`, `girpr-filter`,
  `girpr-report-core`, `girpr-retry`, `girpr-model`, `girpr-fs-identity` and
  `girpr-fs-names` was judged low-value: both consumers use both exports, or the
  crate is already tiny/zero-dep. Do not reopen these as split tasks; raise a
  follow-up only with a concrete rebuild-edge argument.
- **Report §F35 (lines 412–417).** `girpr-log` split has low fan-out payoff
  because all three programs consume level + filter + plan + install; the
  genuinely isolatable part (`test-support`) is already feature-gated. T19 is
  only a payoff *assessment* — if no rebuild edge shrinks, it is skipped.
- **Report §A11 (lines 96–101), §F20 (lines 266–274), §F28 (lines 335–347).**
  Moving `#[cfg(test)]` code out of `src/` has limited payoff: it shrinks the
  lib *test* target but does not reduce integration re-links after a production
  edit, and many test-only seams use private `#[cfg(test)] use` names that
  cannot simply move. Only promote such seams when a split task says so.
- **Report §F33 (lines 394–399).** `tests/scripts/io-profile/*/target/` is disk
  residue, not a workspace build cost; `iot-bench` / `iotime` are outside the
  root member list (§A12). Do not treat them as split targets.
- **Report §F30 (lines 365–375).** `CARGO_BIN_EXE_*` binary coupling is
  deliberate (only the binary proves a CLI flag reaches its client). Do not
  remove it to make a split look cheaper; account for it in the rebuild set.

## Duplication whose boundary stays inside one crate

These are not cross-crate split tasks. Raise a follow-up rather than expanding
a task into them (report §F38, lines 432–445):

- `OutputFormat` declared three times (`src/report/record.rs`,
  `tools/sync/src/report/mod.rs`, `tools/capture/src/report/mod.rs`) — the CLI
  leaf keeps only `clap_error_code`; unifying the type changes output bytes.
- `girpr_log::format_timestamp` vs capture's local reimplementation.
- API DTO field sets (`Wrapper<T>` and config/branch/build/manifest) with
  deliberately different decoders and URL joining.
- Chunk identity `(md5, size)` duplicated in girpr and gircap without a shared
  type.
- Bounded zstd decode, in-capture validation triplication, product link helpers,
  locks and walkers remain local; the last four are covered by §A13.

If a session believes a "keep" item should be changed, it must stop and raise a
follow-up rather than expanding its task's scope.
