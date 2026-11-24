# Findings — short version

## Answer

**Does the server provide Sophon for Persistent? — CONFIRMED NO, for the actual
hotfix bytes.** (`hk4e_global`, dispatcher res revision `49530578`, 2026-11-23
UTC; builds 7.0.0 / 7.1.0 / 7.2.0.)

Live evidence, from `README.md §3`:

* 3164 dispatcher persistent rows (2631 `res_versions_external` + 533 `data_versions`).
* 15 Sophon chunk manifests + 5 Sophon patch manifests downloaded from the
  server's own `getBuild` / `getPatchBuild` responses.
* **0** hotfix MD5s found in any of them. The 376 dispatcher rows whose MD5 is
  absent from every Sophon build are exactly the genuine hotfix set, including
  all **10** rows the dispatcher itself flags `isPatch: true`.
* The 1661 dispatcher rows that *do* match Sophon by path+MD5 are **base-install
  copies** in `StreamingAssets/`, byte-identical to the 7.x base build — not
  Persistent hotfixes. Collapse's MD5-diff rule
  (`Fetch.Persistent.cs:200-211`) keeps those in `StreamingAssets`.
* The only Sophon path containing "Persistent" is
  `GenshinImpact_Data/StreamingAssets/Persistent/plugindelayed.config` (a plugin
  config), not a game hotfix.
* CDN probes: Sophon chunks/manifests return 404 under dispatcher-derived
  `output_<rev>_<hash>` paths; hotfix full files return 200 with the exact
  manifest MD5, ignore `?chunk=`/`?sophon=`, and support `Range` → 206.

Collapse's `Repair.cs:167` gate (`isNullOrEmpty(remoteURL)`) is a **client-side
routing decision**; this research shows it aligns with the server's data
layout rather than concealing an available Sophon path.

## Confidence

High for the scoped claim above. Cannot prove the nonexistence of an
*unadvertised* private manifest, and CN/Bili regions were not queried. Re-run
the scripts in this directory to refresh the snapshot if revisions change.

## Implications for girpr

1. **The design in docs/01 §6 stands.** Dispatcher query → persistent NDJSON manifests →
   full-file HTTP (primary + alt URL) → MD5 re-verify → markers last. No Sophon
   chunk-fetch path is needed for Persistent.
2. Do not "optimise" a dispatcher row into Sophon just because a same-path base
   file exists: that would write the base version instead of the hotfix. The
   MD5-diff test is the discriminator (369 same-path hotfixes + 7 new-only
   files at this revision).
3. The hotfix CDN answers `Range: bytes=0-15` with **206** and
   `Accept-Ranges: bytes`. girpr can add resumable/ranged full-file downloads
   as an improvement; Collapse does not use this.
4. Keep the two-manifest parse atomic and tolerant of live-only keys
   (`isPatch`, `localName`, `category`), and keep the post-download MD5 gate —
   Collapse's HTTP path has none.
5. Marker writes, purge keep-set extension, read-only stripping and `--check-only`
   / `--dry-run` behaviour from docs/01 §§6.4.2–6.4.4 are unchanged.
