# T34 — sled2redb temp claim → `NamedTempFile`

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §G1 (lines 745–770).

## Scope
- Package: `tools/sled2redb`, `src/convert.rs` (`replace_destination`,
  `claim_temp_sibling`, `temp_candidate_name`, `cleanup_temp`, docs lines
  41–97).

## Implementation plan
1. Claim a temp with
   `tempfile::Builder::prefix(CONVERT_TEMP_PREFIX).suffix(".redb").tempfile_in(parent)`,
   then `.persist_noclobber(dst)` for an atomic rename with a stronger no-clobber
   guarantee.
2. Claim the temp before reading the source (as today); still remove the girsync
   lock sidecar before persist (lines 283–290).
3. Drop now removes the temp on failure, so `cleanup_temp` shrinks dramatically.
4. Rewrite the obsolete docs about pid-in-name and `_2…_1000` spellings.

## Constraints / validation
- Temp names change to `tempfile`'s random suffix; error text for a failed rename
   changes.
- Tests named in §G1 (lines 764–766): overwrite, no residue on failure, rewrite
   replaces (also `tests/convert.rs` lines 326–342, 368–415, 432–460).

## Done when
The hand-rolled claim loop is replaced and the docs and no-residue tests pass.
