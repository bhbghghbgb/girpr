# T07 — Add validated path joins and cache canonicalization

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §3 **IO-05** (lines 252–266) — `PathRoot::join` re-runs
  `contained_rel` → `validate_rel` → `girpr_path_shape::validate` and
  `resolve_existing_prefix` (a `canonicalize` syscall) on the joined path, even though
  the rel was already cleaned/validated in `build_plan`; repeated in every consumer.
  Add `PathRoot::join_validated(&self, clean_rel)` for plan-derived rels and cache
  resolved parent directories in a `HashMap<PathBuf, PathBuf>`.
- §3 **IO-06** (lines 267–276) — `game_dir` is canonicalized 3–4× per run; add a
  canonical-form flag/`lock_path_for_canonical` and skip the second `PathRoot::new`
  when `create_dir_all` did not change the path.
- §11 **assumption 5** (lines 850–852) — `join_validated` must be used only from
  `build_plan` consumers; no untrusted rel that only looks plan-derived may take the
  fast path.
- Keep items: path-safety validation, §12, `guidance/DO-NOT-TOUCH.md`.

## Scope
- `app/util-path/src/path.rs` (`join` ~310–326, `resolve_existing_prefix` ~398–425,
  parent helpers ~226–232), `crates/path-shape/src/lib.rs` (~136–182).
- Callers `src/repair/phases/check.rs` (~110–113), `scope/report.rs` (~91–94),
  `scan.rs` (~165–168, 352–355).
- `src/repair/mod.rs` (~190–193, 838–841), `app/lock/src/lib.rs` (~110–114).
- Locate every call site before editing; the list above is a hint.

## Implementation plan
1. Validate assumption 5: enumerate every `PathRoot::join` caller and confirm which
   rels are genuinely plan-derived/clean. If a caller passes untrusted input, keep the
   full validation for it.
2. Add `join_validated` that skips lexical revalidation for clean plan rels, and use
   it only in confirmed `build_plan` consumers.
3. Cache resolved parent directories keyed by parent path so files in one directory
   canonicalize once.
4. Add the canonical-form lock path and avoid redundant canonicalization / a second
   `PathRoot::new` when `create_dir_all` did not change the path (IO-06).
5. Deliver in several commits if useful: validated join, parent cache, canonical flag.

## Constraints / validation
- Path containment and `path-shape` validation must still hold for all untrusted
  input; the fast path is strictly opt-in for clean rels.
- Cached canonicalization must not go stale if the tree changes during a run; bound
  the cache lifetime to the plan/invocation.
- Do not change data-plane output or any §12 non-finding.

## Done when
Plan-derived rels bypass only redundant revalidation, parent canonicalization is
cached, `game_dir` is canonicalized once, path-safety tests pass, and no untrusted rel
uses the fast path. If assumption 5 cannot be confirmed, **stop** and record `skipped`
with a one-line reason.
