# T03 — XXH64 numeric/hex boundary in `girpr-digest`

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Give `girpr-digest` ownership of the XXH64 numeric/hex conversions and adopt
them at the sites that currently hash, parse, read and re-spell the value by
hand.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:176-187` — X9, XXH64 boundary.
- `docs/09-shared-code-refactor-findings.md:407-411` — APP-04, numeric hashing and parsing.
- `docs/09-shared-code-refactor-findings.md:551-555` — LF-04, no leaf owner.
- `docs/09-shared-code-refactor-findings.md:569-573` — LF-07, bypassed hex wrappers.
- `docs/09-shared-code-refactor-findings.md:628-635` — LF-15, `normalize_hex` placement.

## Boundary / approach

Widen `girpr-digest` with `xxhash64_u64`, `parse_xxhash64_hex`,
`format_xxhash64_hex` and a numeric accessor. Adopt the existing
`encode_hex`/`decode_hex` wrappers in `girsync-cache-json`. Move `normalize_hex`
into `girpr-digest` and re-export it from `girpr-chunk-identity`.

## Guardrails

- The exact 16-digit lowercase spelling and the digest-wins policy stay at
  call sites.
- The *when to normalize* decision stays at the capture decode boundary.
- Do not move `normalize_hex` into `girpr-proto`; that changes dump bytes.
- Do not enable `serde_json/preserve_order` (A10); use local ordered forms.
- Direct `hex`/`xxhash-rust` dependencies should be dropped where they become
  unused.

## Commit breakdown

1. Add conversions/accessor to `girpr-digest` with round-trip tests.
2. Adopt in `app/persistent-plan` and `app/fetch`.
3. Adopt `encode_hex`/`decode_hex` in `cache-json`; drop its direct `hex` dep.
4. Move `normalize_hex`, re-export from `girpr-chunk-identity`, drop redundant deps.

## Validation

- Golden bundle and NDJSON byte tests; hex round-trip/lowercase tests.
- `cargo nextest` for digest, persistent-plan, fetch and cache-json.

## Done when

No crate outside `girpr-digest` derives or spells an XXH64 value by hand, and
all published hex strings remain byte-identical.
