# T10 — Small app wire helpers → serde-aux / quick-xml / base64 / hex

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §APP-3 (lines 296–306) — the custom
  serde "number or string" visitor is `serde-aux`'s
  `deserialize_number_from_string`.
- §APP-4 (308–319) — XML tag scanning for RSA components becomes `quick-xml`
  deserialization into a five-field struct.
- §APP-9 (371–378) — base64 padding normalization becomes
  `STANDARD_NO_PAD.decode(...)`.
- §APP-10 (380–389) — manual hex validation becomes `hex`; the digest helper returns
  `finish()` directly instead of rendering and re-parsing hex.
- Validate assumptions first: §A1 (15–16), §A2 (17–18), §A4 (22–23), §A5 (26–28).
  Keep items: `guidance/DO-NOT-TOUCH.md`, especially the permissive version spellings
  at report lines 822–823.

## Scope
- `app/fetch/src/hyp.rs` — `num_from_string` (`:236–270`), uses at `:286`, `:288`,
  tests `:769–823`.
- `app/dispatch/src/metadata.rs` — `rsa_key_from_xml` (`:254–294`);
  `app/dispatch/src/tests.rs:382`.
- `app/dispatch/src/crypto.rs:15–21`; `app/dispatch/src/tests.rs:244`, `:321`, `:402`.
- `app/persistent-plan/src/manifest.rs:99–106` (`parse_hash_hex`), tests `:127`, `:135`;
  `app/persistent-plan/src/digest.rs:140–145`, test `:232`.
- Affected manifests. Locate current sites before editing; the list above is a hint.

## Implementation plan
1. Validate each premise in the current code. In particular, confirm the XML is well
   formed and the base64 fields' padding shapes before choosing `STANDARD_NO_PAD`
   (keep `STANDARD` as a fallback if both shapes occur).
2. Replace the visitor with `#[serde(default, deserialize_with = "deserialize_number_from_string")]`
   on `compressed_size`/`uncompressed_size`; delete the visitor. Keep negative values
   and the `i64` range for the consumer to clamp.
3. Deserialize the RSA XML into a struct with five renamed `String` fields via
   `quick_xml::de::from_str`; keep the base64 decode and key construction. Unmapped
   `InverseQ` becomes "field not in struct" (§APP-4 line 316).
4. Switch base64 decoding to `STANDARD_NO_PAD` with the whitespace filter intact, and
   use `hex` for hex validation/round-trip; let the XXH64 helper return `finish()`.
5. Adjust error-text pins and keep tests of intended size/XML/key/digest behavior.

## Constraints / validation
- Error wording is not an output contract (§APP-3 line 306, §APP-10 line 389); negative
  sizes and uppercase hex acceptance must survive.
- Do not use `semver` here — this task does not touch version spellings (non-finding,
  lines 822–823).
- `serde-aux` and `quick-xml` are **not** currently locked; verify a resolved build.

## Done when
All four helpers use libraries, the affected fetch/dispatch/persistent-plan tests pass
with adjusted error pins, and no hand-rolled visitor/tag-scanner/padding/hex validator
remains. If a premise is stale or a keeper would change, **stop** and record `skipped`.
