# Do not touch (or touch only as a task says)

These findings were examined and deliberately kept, or are low-value/local items that
are **not** tasks. Before editing anything, a session should confirm its target is not
on this list. Full reasoning is in the report; only pointers are given here. If a session
finds one of these should change after all, it **stops** and raises a follow-up — it does
not change the keeper in place (§A9, lines 88–97).

## Negative findings — these stay (§3)

- **N1 (lines 697–699):** INI parsers stay divergent (girpr last-match/no-section vs
  gircap first-match `[General]` streaming). Not a shared DTO/reader.
- **N2 (lines 701–703):** `_tmp` resume/rollback and the audio move are durable artifacts,
  not generic temps; a generic `AtomicPublish` (F33) must not absorb them.
- **N3 (lines 705–707):** the FNV lock key, OS advisory locks and sidecar cache lock stay;
  changing the FNV key orphans existing files.
- **N4 (lines 709–711):** `DigestSet`/`AlgoSet`/`HashAlg` stay raw-byte and
  allocation-free; F4/F28 must not absorb girsync's digest vocabulary.
- **N5 (lines 713–716):** the rw gate, lock ordering, bounded ordered runner, symlink
  classification and `select/budget.rs` stay local; F29 must not absorb them.
- **N6 (lines 718–720):** the replay/bridge server and all domain fake servers stay
  independent; only the mechanism in `girpr-loopback-mock` is shareable (F49).
- **N7 (lines 722–723):** the `zstd_measure`/`zstd_measure_from` pair stays duplicated —
  distinct error text is the reason; F52 likely re-confirms this.
- **N8 (lines 725–727):** exit-code values, URL/status/retcode helpers and flag sets stay
  local; only `girpr-exit`/`girpr-cli` mechanisms are shared.
- **N9 (lines 729–732):** capture's artifact writer and its selfcheck wording table stay
  local; F22's envelope must not absorb either.
- **N10 (lines 734–737):** the four gircap-owned convention deviations (`gircap-` prefix,
  location under `tools/capture/`, local lint policy, no product deps) must not be
  "corrected"; this blocks naive promotion in F26/F52.
- **N11 (lines 739–744):** hex spelling, ordered-JSON writing, sleep drivers and
  clap-error mapping have no remaining duplication — do not re-propose these extractions.
- **N12 (lines 746–748):** `logs_of` is not duplicated; the separate capture buffers are
  (F40).
- **N13 (lines 750–752):** path spelling/case predicates, digest spelling and
  glob-compile options are already leaves; F13 concerns only the two-sided existence rule.
- **N14 (lines 754–756):** the product's `RepairDiagnostics` field set and capture's
  matrix/budget/require-class semantics stay local.
- **N15 (lines 758–761):** cache codec/schema/publisher and `FileRec.kind` are closed
  decisions; F28 does not reopen the literal-byte pin, `FMT_V2`, `CACHE_VERSION`, the
  closed `kind` registry or the one-syscall gaps.
- **N16 (lines 763–766):** the offline mock, artifact replay server, fetch scripting
  helper and gircap's server encode different contracts; only the shared
  listener/journal leaf exists.

## Standing invariants from §1 — changing one is out of scope

- **Byte contracts (§A5, lines 57–67).** Bundle, dump and report bytes, schema versions,
  protobuf field names, cache wire format and `SUMMARY` key/flag conditions are pinned.
  Do not change key order, casing, spacing, wording or flag conditions even if tests pass.
- **gircap independence (§A4, lines 48–56).** gircap has no `girpr` dependency, never
  invokes its binary, owns its codes/format/flags/fixtures, and may depend only on
  logic-free shared leaves. Adding a leaf requires synchronized updates to the §2 table,
  manifest comment block, README inventory and `docs/06-artifact-bridge-spec.md`; the
  bridge must not depend on gircap.
- **Report planes (§A6, lines 68–75).** girpr, girsync and gircap report planes cannot be
  merged verbatim; only mechanics are shared. Do not enable `preserve_order` in
  `girpr-report-core`; do not unify boolean vs `1`/`0` flags, percent-encoding or key order.
- **Exit codes (§A7, lines 76–80).** girpr, gircap and the tools have independent code
  contracts; only the enum/`ALL`/pin generation and clap-error mapping patterns are shared.
- **Digest semantics (§A8, lines 81–87).** There is no single "digest": girpr's strict
  16-byte parsed `Md5`, girsync's raw bytes + foreign algorithm names, and gircap's
  lowercase hex must not be collapsed; `DigestSet`/`AlgoSet`/`HashAlg` are not absorbed.
- **Divergent policies are keepers (§A9, lines 88–97).** Retry jitter (0.5 vs 0.0), INI
  last-match vs first-match, `path_key` case fold (all hosts vs Windows only), and cache
  fail-open vs fail-closed are intentional. A session that finds one should be changed
  stops and raises a follow-up.
- **Verification is an independent re-derivation (§A13, lines 122–128).** girsync's two
  reads + failed flush, gircap's independent pass 2, and girpr's hash gates/MTIME guard
  are correctness arguments, not caches. No "generic engine" may reuse an observation
  buffer and turn "verified" into a tautology.
- **Prior exclusion list (§A15, lines 136–141).** Prior report 08's `N1`–`N10`, the
  `DO-NOT-TOUCH` §H items (rw gate, FNV lock key, fail-fast cache open, symlink
  classification, bounded ordered runner, `_tmp` resume, parity rules, replay/bridge
  server) and the cache/publish invariants remain in force. A finding may revisit a
  premise, but none proposes deleting them.

## High-risk / near-forbidden candidates — implement only on hard evidence

These are real candidate boundaries in the report but carry an explicit "likely skip",
"forbidden by A4/A9", or "low confidence" verdict. The default outcome is a validated
skip; do not force an abstraction:

- **F17 (lines 325–332)** decision-record transport primitive — one transport is a
  published artifact (§A5), confidence low.
- **F18 (lines 334–341)** install-provenance value crate — near-forbidden the moment it
  emits a key or fixes field order (§A4, §A6).
- **F20 (lines 356–365)** refusal ledger — creates a new shared observation format that
  §A4 forbids and whose sampling is byte-visible (§A5).
- **F26 (lines 412–422)** HTTP transport leaf — reverses the documented reason
  `gircap-http` exists and couples three programs' reqwest features (§A2, §A4).
- **F28 (lines 437–446)** generic verified KV cache — would tempt reinstating a
  deliberately forbidden product cache and hide a literal-byte contract (§A5, §N15).
- **F29 (lines 448–456)** generic engine trait — crosses three exit contracts, three
  report planes, the open digest vocabulary and the tool SCCs (§A7, §A6, §A8, §A12);
  plausible outcome is a trait shape at most.
- **F41 (lines 567–576)** generic command harness — forcing a common typed outcome
  contradicts §A4.
- **F44 (lines 599–605)** umbrella `test-support` facade — feature unification (§A11)
  makes it a generalization trap.
- **F48 (lines 634–642)** `girpr-primitives` consolidation — the value of these leaves is
  their rebuild domain (§A2), not their code volume.
- **F52 (lines 674–682)** `bounded_decode` — streaming invariant is an always-on test and
  F52 likely re-confirms N7's skip.
