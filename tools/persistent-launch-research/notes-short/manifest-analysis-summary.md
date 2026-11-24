# Manifest analysis summary (extract of the gitignored `notes/manifest-analysis.txt`)

Regenerate with:

```
python scripts/analyze_manifests.py "F:\Genshin Impact game" "E:\Volume 1\gem\power\Genshin Impact game"
```

All manifests are NDJSON with keys `remoteName`, `md5`, `fileSize`, `hash`
(XXH64 hex in 7.1+); `res_*` also has `category`, `localName`, `isPatch`.
Dispatcher rows may carry `isPatch`, which Collapse ignores for placement.

| manifest | rows | note |
|---|---|---|
| F: `res_versions_streaming` | 2,624 | base install |
| F: `data_versions_streaming` | 533 | base install |
| F: `silence_versions_streaming` | 4 | base install |
| F: `VideoAssets/video_versions_streaming` | 279 | base install |
| E: `res_versions_persist` | 2,631 | 7 more rows than streaming (new hotfix paths) |
| E: `data_versions_persist` | 533 | **368 rows differ** from streaming (md5/hash/size) |
| E: `silence_data_versions_persist` | 4 | all 4 differ from streaming |

F: and E: base manifests are byte-identical (same MD5/size), so the two copies
share one base install; all differences are extra files.

`python scripts/verify_persist_manifest.py <E-snapshot> <E-game-dir>` found, for
the persisted rows whose file exists under `Persistent/` and was hashed:
`res_versions_persist` 4 OK / 0 mismatch, `data_versions_persist` 314 OK /
0 mismatch. Most rows resolve to base files under `StreamingAssets/` (they are
in the torrent), not to `Persistent/`: e.g. the four silence rows live in
`StreamingAssets/AssetBundles/blocks/00/`. A persisted manifest is therefore
not a list of files *only* under `Persistent/`.

After one clean launch F:'s three mirrors are byte-identical to E:'s:
`res_versions_persist` MD5 `9a1b5efc2e2b11475e74c88789d0f339`,
`data_versions_persist` MD5 `09f2c36615dc806f67f55e3d2c8a0c8d`,
`silence_data_versions_persist` MD5 `2bcf33810856cb0820725300ff2d8666`.
