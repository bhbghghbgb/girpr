# T06 — Reuse scan verdicts and remove double reads in the repair path

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §3 **IO-01** (lines 202–214) — `attempt_temp` redoes
  its own length + full `md5_file_digest` skip check even when a plan scan produced a
  whole-file digest; share the mtime/length-guarded `reusable_verdict` from the
  in-place path.
- §3 **IO-02** (lines 215–228) — the reuse slice is read twice (hash pass, then copy
  pass after a seek back); read once into a staging buffer, hash, then write, or spool
  with a bounded buffer and a post-hoc digest gate.
- §3 **IO-03** (lines 229–240) — staged source bytes are hashed again after the scan
  already verified the range; trust the scan per-range verdict, or fold the re-check
  into the single capture read.
- §3 **IO-04** (lines 241–251) — `file.layout()` reallocates and re-sorts all chunks
  up to three times per file; compute it once and pass `&Layout` down.
- §6 **COR-04** (lines 453–462) — reuse-path I/O errors (`File::open`, slice digest,
  seek, reuse-copy read/truncation) are swallowed into trace/debug logs and converted
  to `Download`; keep the fallback but record unexpected I/O failures in diagnostics
  and at `info`/`warn`, reserving `debug` for the expected MD5 mismatch. This is the
  same code region as IO-02, so handle it here.
- §11 **assumptions 3–4** (lines 843–849) — verdict reuse must respect the same guard
  (a file written earlier in the run must not be skipped); bounded staging bytes must
  fit `--max-ram-bytes`/`--max-write-bytes`; a required TOCTOU guard must be folded,
  not dropped.
- Keep items: §12, conservative fallbacks, `guidance/DO-NOT-TOUCH.md`.

## Scope
- `src/repair/file/temp.rs` (~35–55, 350–394, 436–467), `file/mod.rs` (`FileCtx.scan`
  ~459, scratch buffers ~114–119), `file/inplace.rs` (~213–239, 400–429, 537, 668,
  711).
- `app/repair-plan/src/plan.rs` (`file.layout()` ~62–107), `app/chunk-verify` scan
  verdict types, `app/util-hash` per-range verification.
- Locate the current structures before editing; the list above is a hint.

## Implementation plan
1. Validate assumptions 3–4 against the current tree: confirm the scan verdict's
   digest covers the same bytes under the same guard, and that a bounded staging
   buffer fits the RAM/write budgets. If a TOCTOU re-check is required, keep it but
   fold it into the single read.
2. Share the in-place `reusable_verdict` with the temp skip check; read only when no
   verdict is reusable (IO-01).
3. Read the reuse slice once — hash and write from one buffer (or spool with a
   bounded sink and commit on digest match) — instead of the seek-back double read
   (IO-02).
4. Trust the scan per-range verdict for staged sources; remove the redundant hash or
   fold it into the single capture read (IO-03).
5. Compute `file.layout()` once in `attempt_inplace_inner` and pass `&Layout` to all
   consumers (IO-04).
6. Surface unexpected reuse-path I/O errors as diagnostics / `info`/`warn` while
   keeping the download fallback and the concrete `io::Error` text (COR-04).
7. Deliver in several commits if useful: verdict sharing, single-pass reuse I/O,
   layout plumbing, diagnostics.

## Constraints / validation
- The guard's mtime/length conditions must be identical for in-place and temp; do not
  skip a file modified earlier in the same run.
- Staging buffers must stay within `--max-ram-bytes` / `--max-write-bytes`.
- Do not weaken a required TOCTOU guard; a fidelity vs. double-read trade-off must be
  explicit and documented in code.
- Repair result bytes and data-plane output must be unchanged.

## Done when
Intact files are read once, reuse slices are read once, staged bytes are hashed once,
`layout()` is computed once per file, and unexpected reuse I/O errors are visible,
with budgets and guards intact and tests passing. If an assumption fails or the guard
cannot be preserved, **stop** and record `skipped` with a one-line reason.
