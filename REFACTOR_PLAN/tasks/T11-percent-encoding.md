# T11 — Percent encoding → `percent-encoding`

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §F5 (lines 665–682).
- Assumption to validate: §I.7 (lines 834–836).

## Scope
- Package: `tools/capture`, `src/encoding.rs` (`percent_encode`,
  `percent_decode`, `percent`). `percent-encoding` is already transitive via
  `reqwest`/`url`.

## Implementation plan
1. Define `const TEXT: &AsciiSet = &CONTROLS.add(b' ').add(b'%').add(b'\t')
   .add(b'\n').add(b'\r')`, and allow bytes ≥ 0x80 so non-ASCII characters stay
   unencoded in the text form.
2. Use `percent_encode(bytes, TEXT)` and
   `percent_decode_str(s).decode_utf8_lossy()`.
3. Keep the public re-export `report::percent_decode` path (`src/report/mod.rs`).

## Constraints / validation
- Current `percent_encode` leaves non-ASCII untouched; a byte-oriented
  `AsciiSet` would otherwise escape their UTF-8 bytes (§I.7).
- URL form must stay uppercase.
- Tests named in §F5, including the text-line assertion and the non-ASCII case.

## Done when
The hand-rolled percent codec is gone, the re-export path is intact, and the
escape-set assumptions are validated.
