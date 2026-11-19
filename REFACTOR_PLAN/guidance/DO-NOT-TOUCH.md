# Do not touch (or touch only as a task says)

These findings were examined and deliberately kept. Before editing anything, a
session should confirm its target is not on this list. Full reasoning is in the
report; only pointers are given here.

- **All of report §H** (lines 785–803) — the "hand-rolled code that should stay"
  table: progress reporting, `_tmp` resume artifacts, OS advisory locks,
  lock-file FNV key bytes, `DigestSet`/`AlgoSet`, symlink/reparse-point
  classification, the bounded ordered task runner, the replay/bridge HTTP
  server, range diagnostics and Starward/Collapse parity rules.
- **Report §A11** (lines 333–345) — log level parser, `File::try_lock`,
  `FileReuse` and lock-file FNV key stay. Only the named-constant cleanup is a
  task (T04); the key bytes must not change.
- **Report §E6** (lines 621–636) — digest containers, symlink handling, sidecar
  file lock stay; the legacy recursive copy is not worth delegating.
- **Report §B3** (lines 425–426) — `PhaseTimer` → tracing was not shown to be an
  improvement; leave it.
- Per-task caveats that must survive even when the surrounding code is
  delegated (see each task's "Constraints / validation"): girpr deterministic
  backoff vs gircap's no-jitter/diffable artifact (§A2, §I.3), gircap artifact
  byte-stability for `config.ini` (§A4), truncated/lying `Content-Length` mock
  tests (§I.11), `_tmp`/audio move not delegated (§B4), link-count guard (§E2),
  and the one-syscall/fail-fast invariants in `cache/open.rs` (§E5).

If a session believes a "keep" item should be changed, it must stop and raise a
follow-up rather than expanding its task's scope.
