# Review note — `girpr`, scope-of-work limitations

**This file is temporary.** Fix what it lists, then delete it in the same series.
Do not mention it, refer to it, quote it, or leave a pointer to it in a commit
message, a source comment, a test name, a doc or the README. It is a handover,
not a document, and nothing about it should outlive the fixes.

Scope: `src/`, `tests/`, `docs/04-scope-limitation-options.md`,
`docs/05-scope-limitation-implementation-plan.md`, `README.md`. M0–M7 are
otherwise green and nothing below asks for a redesign.

---

## 1. The `--include`-matched-nothing guard makes a scoped cleanup unreachable, which is the one thing `--purge-only` exists for

`src/repair/mod.rs`:

```rust
// 4.1 A `--include` that selected nothing is a typo, …
if scope.has_includes() && plan.files.is_empty() {
    return Err(RunFailure::usage(anyhow::anyhow!(
        "--include '{}' matched no file in the live manifest (plan_files=0); …")));
}
…
// 4.6 the purge candidates, planned up front …
let purge_plan = (ctx.purge_before || ctx.purge_after || ctx.purge_only || ctx.print_plan)
    .then(|| plan::purge::plan_purge(&game_dir, &plan, &plan::purge::keep_set(), scope));
```

The guard at 4.1 tests `plan.files` and nothing else, and it runs **before** the
purge plan is built. So for a path that is on disk but absent from the new
manifest — the ordinary "this file was removed upstream" case — the run is
refused at 4.1 and never reaches 4.6, even under `--purge-only`, which skips the
repair loop entirely and does the cleanup as its whole job.

The classification underneath is already correct and is not the problem:
`plan::purge::classify_purge_path` checks the scope **first**
(`src/repair/plan/purge.rs:262`), so that path classifies `Purge` and is a
correct delete candidate. The only thing standing in the way is the guard's
condition.

**Fix.** Make the guard answer "did this run select nothing at all", not "is the
repair plan empty" — an empty plan alongside non-empty purge candidates is a
legitimate request, and refusing it is the one case where `--purge-only` under a
scope cannot do the job it was built for. Concretely, the guard has to run where
the purge plan is known, or the condition has to admit the purge path.

Two consequences to settle at the same time, because both are visible in output:

- the refusal text names only the manifest (`"matched no file in the live
  manifest (plan_files=0)"`). If a purge flag is active, the message has to say
  which of the two plans was empty.
- with `--purge-only` and a scope, `config_bumped=0` is already emitted and the
  cleanup runs; confirm the fixed guard leaves that path reporting the same way,
  and add the case to `tests/repair_offline.rs` — the mirror of the existing
  `a_scope_that_admits_an_orphan_still_deletes_it` unit test, driven through the
  binary.

## 2. `docs/05` contradicts itself about what M7 did to the capture's runbook

The M7 section says:

> `tools/capture/docs/00-spec.md` §8.1 is marked resolved, with §8.1.1 step 4
> struck through as obsolete (a scoped purge is now safe, because a hidden path
> is invisible to the cleanup too) and two steps added.

The "Defects found and fixed" section of the same document says the opposite:

> the `gircap` spec edit was +51/−15 against an agreed "one line" boundary; …
> The `gircap` edit is now a status line and a notification — the §8.1 body and
> §8.1.1 are untouched.

The second is what happened. §8.1.1 step 4 is on disk verbatim as "**Do not**
pass `--purge-after` or `--purge-before`. Without path filters they delete every
file the live manifest does not contain", and its reason is the one §8.1's own
update block refutes. A reader of M7 alone would believe the runbook was
updated; a reader of the runbook would find the stale reason still standing.

**Fix.** Make M7's section describe what M7 actually did — a status line and a
notification in §8.1, nothing in §8.1.1 — and cross-reference the defects
section that already records why. Then decide, deliberately rather than by
omission, whether §8.1.1's step 4 gets corrected here or stays a notification
for the capture side to own. Either is defensible; leaving the plan describing a
rewrite that was reverted is not.

Note also that `docs/04` §1.3 enumerates three assertions per capture and calls
the third — *"no chunk id in `expectations.must_not_fetch` was requested"* —
"invisible in a byte count and … the one that actually catches a broken S2 reuse
path". Nothing in the binary can surface it: `SUMMARY` is byte-level, and
per-chunk detail exists only as `trace` lines in
`<exe-dir>/logs/girpr_<ts>.log`. That is fine as long as it is stated where a
consumer will read it — say so in §1.3 rather than leaving a reader to discover
it.

## 3. A run that reused nothing is indistinguishable from one that could not reuse

`src/repair/metadata.rs`:

```rust
let mut local_build = None;
if let Some(lv) = local_version.as_deref() {
    match hyp.chunk_build(&branch, Some(lv)).await {
        Ok(b) => { …; local_build = Some(b); }
        Err(e) => tracing::warn!("local build unavailable ({e}); full-fetch fallback"),
    }
}
```

The fallback is correct for a real install — a tag the server no longer serves
must degrade to more downloads, never to a wrong result, and that reasoning is
recorded. But the run then reports `download_bytes` as if every byte had to be
fetched, `est_download_bytes` rises with it, and **the run still exits `0`**.
Nothing on the data plane distinguishes:

- a file patched with 8 chunks reused and 2 downloaded, from
- the same file patched with 10 chunks downloaded because the local manifest
  could not be loaded.

`--max-download-bytes` is the sharpest version of the problem: its pre-flight
estimate is computed against the same empty map, so the ceiling silently moves
and the refusal a caller expected never fires.

**Fix.** Surface it. A conditional `SUMMARY` key — `local_manifest=ok` /
`local_manifest=unavailable`, emitted only when a local version was known and
the lookup was attempted, so an unflagged run's line stays byte-identical — makes
the degradation assertable by a machine consumer instead of discoverable only by
reading a warning. Follow the existing conditional-key discipline in
`src/repair/scope/report.rs`: fixed order, emitted only when active, and
cross-checked by the test that pins the key order.

This is an addition, not a behaviour change: the fallback stays, and the exit
codes stay.

---

## Checklist

- [ ] A `--purge-only` run whose `--include` matches a path absent from the manifest is not refused at 4.1, and the cleanup happens
- [ ] The 4.1 refusal names the plan it found empty, including the purge plan when a cleanup flag is active
- [ ] `tests/repair_offline.rs` drives that case through the binary
- [ ] `docs/05`'s M7 section matches what M7 did; the contradiction with its own defects section is gone
- [ ] §8.1.1 step 4's fate is decided and recorded rather than left implicit
- [ ] `docs/04` §1.3 states that `must_not_fetch` needs a request log, because the binary cannot report it
- [ ] `SUMMARY` distinguishes a local manifest that loaded from one that did not, without changing an unflagged run's line
- [ ] **this file is deleted**