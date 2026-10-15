# Review note — `gircap`

**This file is temporary.** Fix what it lists, then delete it in the same series.
Do not mention it, refer to it, quote it, or leave a pointer to it in a commit
message, a source comment, a test name, a doc or the README. It is a handover,
not a document, and nothing about it should outlive the fixes.

Scope: this file plus `src/` and `tests/`. Evidence is quoted from the bundle at
`artifacts/hk4e_global/7.0.0__7.1.0/20261014T055348Z/` — the one that passes
`gircap verify` with 115 rows and `selfcheck.json` `pass`. Re-capture or amend
the numbers in the same commit that fixes the shape; a bundle and a spec that
disagree is the defect being fixed.

---

## 1. `patchmaps/added.json` contradicts the artifact's own schema, and the one number it does carry is wrong

`01-artifact-schema.md` §13 says, verbatim:

> For `removed` captures there is no `new`, no `plan`, and no `expectations`; for
> `added` there is no `old`, and every `plan` entry is `FETCH` with
> `source_range: null`.

So `added` is documented as having a `plan`. It has none:

```jsonc
// patchmaps/added.json, as emitted
{
  "capture_id": "added",
  "path": "GenshinImpact_Data/Plugins/libHttpClient.GDK.dll",
  "old": null,
  "new": { "tag": "7.1.0", "size": 249256, "md5": "cebc7048…", "nchunks": 1 },
  "plan": null,          // §13 says every entry is FETCH — there are none
  "totals": null,
  "expectations": {
    "result_path": "expected/GenshinImpact_Data/Plugins/libHttpClient.GDK.dll",
    "result_md5": "cebc704824d361b8fe34b2ceaed0c810",
    "download_bytes": 0,        // ← wrong, see below
    "must_not_fetch": []
  }
}
```

`download_bytes: 0` is not "not recorded". It is **false**, and the bundle
already holds the number that disproves it. `chunks/index.json`:

```jsonc
{ "chunk_id": "82ec130ee3168c70_cebc704824d361b8fe34b2ceaed0c810",
  "bytes": 114357, "uncompressed_size": 249256,
  "referenced_by": [ … { "role": "materialize_new", … }, { "role": "patch_fetch", … } ],
  "sets": ["materialize_new", "patch_fetch"] }
```

That chunk is in `patch_fetch`. The file is absent from `game/`. A consumer that
repairs this path **must** download 114 357 bytes, so an assertion written as
"`download_bytes` equals `expectations.download_bytes`" compares 114 357 against
0 and fails against a run that is behaving perfectly.

Three documents make the same promise and the bundle breaks all three:

| source | claim |
|---|---|
| `01-artifact-schema.md` §13 | `added` has a `plan`, every entry `FETCH`, `source_range: null` |
| `00-spec.md` §8.1.1 table | `added` → reuse / fetch / moved = `0 / 1 / 0` |
| the emitted `patchmaps/added.json` | `plan: null`, `totals: null`, `download_bytes: 0` |

**Fix.** Give `added` the `plan` and `totals` §13 promises — one entry,
`decision: "FETCH"`, `source_range: null`, `chunk_id` and `url` populated,
`compressed_size: 114357` — so `totals` reads `chunks: 1, reuse: 0, fetch: 1,
moved: 0, download_bytes: 114357`. The figure is already computed: it is the
`patch_fetch` sum that `select/budget.rs` and `blobs/chunks.rs` use for every
other class. `must_not_fetch: []` is already correct and should stay.

Whichever way this lands, **`download_bytes` for a one-sided capture must never
be a default-initialised `0`.** Absent data has to be absent (`null`), because
`0` is a value a consumer will compare against.

## 2. Nothing asserts either shape

`tests/schema.rs` and `tests/capture_layout.rs` name `added` and `removed` as
matrix rows but never open a `patchmaps/*.json` and look at `plan` or `totals`.
`gircap verify` does not re-derive them either — §6 of `00-spec.md` lists the
seven ids it re-derives and `patchmaps` is not among them.

That is why §1 survived M9 and M10: three documents state the shape, the writer
emits a different one, and the suite is silent. **Add a case that fails on the
current output**, asserting that every capture's `patchmaps/<id>.json` carries
the fields §13 says it carries — the table in §1 and §2 below is the minimum.

## 3. `patchmaps/removed.json` has no `expectations` at all, so a consumer has nothing to assert

```jsonc
{ "capture_id": "removed", "path": "…/00664172.blk",
  "old": { "tag": "7.0.0", "size": 1731133, "md5": "6c220a33…", "nchunks": 2 },
  "new": null, "plan": null, "totals": null, "expectations": null }
```

`00-spec.md` §8.1.1 nevertheless gives `removed` a row with `2 / 0 / 0` in the
reuse / fetch / moved columns — figures the bundle cannot supply, because there
is no new side to patch to.

**Fix**, one of:

- **(a)** give `removed` an `expectations` of its own shape —
  `{"result_path": null, "absent_after": true, "download_bytes": 0,
  "must_not_fetch": []}` — so the consumer has something to assert (the file is
  gone afterwards, and nothing was downloaded), and correct the §8.1.1 table to
  show the figures the bundle actually carries; **or**
- **(b)** make `totals` present for `removed` too, counting the `materialize_old`
  side (`chunks: 2, reuse: 2, fetch: 0, moved: 0`), which is what the table's
  `2 / 0 / 0` is reaching for, and state in §13 that `expectations` is absent
  only because there is no result path.

(a) is the smaller change and the more useful one: it gives the one-sided
capture an assertion a consumer can make, which is the whole point of the file.

## 4. `00-spec.md` §8.1.1 step 4 states a reason that is no longer true

Step 4 reads:

> **Do not** pass `--purge-after` or `--purge-before`. Without path filters they
> delete every file the live manifest does not contain, which for a 9-file
> fixture is most of it.

The conclusion is still the right advice. The reason is not: a path outside an
active `--include` is classified as kept **before** the manifest check, so a
scoped cleanup cannot see the fixture at all. §8.1's own update block already
says so — *"a filtered-out path is invisible to the files-cleanup as well as to
the repair"* — and §8.1.1 was not updated to match.

**Fix.** Rewrite step 4's reason to the scoped-cleanup rule, and record the
consequence a consumer actually needs: under a scope, passing a cleanup flag is
safe, and the **only** way to act on a `removed` path is a cleanup, because such
a path is by construction absent from the new manifest and therefore from any
repair plan. That sentence is the missing half of §8.1.1.

## 5. §8.1.1 does not record a flag that will fail the consumer's run

An explicit `--audio` under an active path scope is refused when the audio scan
file is outside that scope. The scan file here is
`GenshinImpact_Data/Persistent/audio_lang_14` and it holds `English(US)`.

So a consumer that reaches for `--audio en-us` — the natural reflex, and the
flag the capture itself was made with — gets a usage error. Omitting it is
correct: the run then autodetects, reads the scan file, and resolves to `en-us`,
which is exactly the state the bundle was captured in.

**Fix.** Add this to §8.1.1 as a stated constraint of the intended invocation,
alongside `--io-threads 1` and the no-cleanup-flag rule. It is a fact about the
consumer's invocation that the consumer cannot derive from the bundle.

---

## 6. M11 is still recorded as open, and step 4 is the only thing open

`02-plan.md` M11 step 4 remains `**BLOCKED** - not skipped` with *"M11 is not
complete until this runs"*, and §8.1 carries an update block saying the blocker
is resolved while §8.1.1's body is unchanged. Once the check in §8.1.1 becomes
runnable, that bookkeeping has to land with it: the step's outcome, and the
numbers, recorded the same way steps 1–3 are.

Do not do this before §1–§5 — a recorded pass against a bundle whose `added`
expectation is `0` would be a green row about something false, which is the
failure mode M10 already had to fix twice.

---

## Checklist

- [ ] `patchmaps/added.json` carries the `plan` and `totals` §13 promises, and a `download_bytes` that is not `0`
- [ ] No one-sided capture emits a defaulted `0` where the value is unknown
- [ ] `patchmaps/removed.json` has an `expectations` a consumer can assert, and §8.1.1's table matches what the bundle carries
- [ ] A test fails on the shapes as they are emitted today
- [ ] §8.1.1 step 4's reason is the scoped-cleanup rule, and the `removed` path is documented as cleanup-only
- [ ] §8.1.1 records that `--audio` must be omitted under a scope
- [ ] M11 step 4's outcome is recorded
- [ ] **this file is deleted**