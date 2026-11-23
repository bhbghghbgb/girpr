# Do not touch (or touch only as a task says)

These findings were examined and deliberately kept, or are contracts that are **not**
tasks. Before editing anything, a session should confirm its target is not on this
list. Full reasoning is in the report; only pointers are given here. If a session
finds one of these should change after all, it **stops** and raises a follow-up — it
does not change the keeper in place.

## Non-findings — deliberately not reported (§12, lines 890–905)

- **Integer overflow in range arithmetic (892–895):** `r.offset + r.len` and
  `offset + len` cannot overflow `u64` (both operands derive from `i64`-bounded
  manifest fields). Do not "fix" it.
- **`RepairPlan::fingerprint()` (896–897):** already cached at construction; not a
  hot cost. No change.
- **Verify-path multiple reads / temp-mode resume (898–900):** documented and
  internally consistent; no off-by-one found. Do not restructure on suspicion.
- **`write_config_ini` / `read_game_version` (901–903):** already avoid the
  exists-then-read TOCTOU and pin last-match-wins; no safe simplification. Leave alone.
- **Cargo profile settings (904–905):** already tuned with measured host data;
  changing them without the same measurements is guesswork. Do not touch any
  `[profile.*]` setting.

## Build surface that stays

- **"Already tuned — no change proposed" (lines 810–816):** `profile.dev`,
  `profile.test`, `profile.release`, `.cargo/config.toml` rust-lld and
  `.config/nextest.toml` setup/fhooks are deliberate. T25 concerns only the `chrono`
  dependency (B-01, lines 818–823), and B-02 (lines 825–829) is explicitly not a
  change.

## Contracts that must survive

- **Data plane (LOG-01, lines 527–530):** the `REPORT` / `PLAN` / `PROGRESS` /
  `SUMMARY` stdout contract is unchanged by logging work. T16 deliberately extends
  `SUMMARY` with `purge_errors=N` (COR-07, lines 486–495) but must keep its
  machine-readable shape stable; that is its sole exception.
- **Deterministic purge log order (S-05, lines 179–183):** parallel stat/delete must
  emit lines in plan order via index slots, not completion order.
- **Pinned girsync wire vocabulary (T24, §9 TOOL-02/16/18):** the cache's `postcard`
  digest vocabulary and file record shapes stay byte-for-byte. IO reductions must
  not change stored bytes.
- **Artifact bundle contract (assumptions 14–17, lines 875–888):** gircap's recorded
  request/response JSON must stay byte-identical (TOOL-01/07/08/09/10); artifact
  bytes and documented write order must stay deterministic (TOOL-13/14/15/17), and
  the close-out fixpoint passes must be unaffected.
- **Conservative correctness fallbacks (COR-02, COR-05, lines 429–441, 464–472):**
  keep "estimate in full" when a scan has no verdict; do not turn a diagnostic into
  a silent success.
- **Best-effort phases (IO-08, COR-06, lines 286–293, 474–485):** post/audio-cache
  and deprecated-file fetches stay best-effort; an early scope refusal must still
  allow a purge candidate to rescue the selection.

## Cross-task pairings (coordinate, don't split)

- **S-01 + S-02 (assumption 1, lines 837–840):** do not shrink the runtime before
  repair file work is off the async workers; land them together. If one is skipped,
  record the reason in `STATE.md` and pass it to the other's session.
- **NET-07 + S-02 (lines 404–413, 139):** the streaming decode/write offload is the
  same class of problem; T10 and T01 must be consistent about blocking context.
- **IO-01 + IO-02 + IO-03 (assumptions 3–4, lines 843–849):** the shared scan verdict
  and single-pass reuse I/O changes depend on the same guard semantics; T06 handles
  them together.

## Scope boundary

- This package and the report are deleted at cleanup. No shipped source, test,
  comment, manifest, doc table or commit message may reference them.
- A finding that is a deliberate trade-off (e.g. TOOL-04, lines 671–678) is a task
  with its own validation, not a license for an unbounded rewrite. Keep the change
  bounded and preserve the stated contracts.
