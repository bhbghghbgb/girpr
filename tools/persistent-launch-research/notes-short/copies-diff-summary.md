# Copy comparison summary (extract of the gitignored `notes/copies-diff.txt`)

Regenerate with:

```
python scripts/compare_copies.py snapshots/F-fresh/F-snapshot.jsonl snapshots/E-marker-files/E-snapshot.jsonl
```

- F: 8,068 files / 155,182,786,241 B; E: 6,502 files / 159,533,438,713 B.
- **0 common files with a different MD5 or size.** Every difference is an extra
  file on one side.
- Only in E: (3,528 files): ~3,000 `Persistent/` hotfix files, ~2,758
  `webCaches/`, 350 stale `StreamingAssets/AssetBundles/blocks/**/*.blk`
  (old-version leftovers), 23 `SDKCaches/`, `config.ini`, `upload_err.log`,
  `Persistent/ctable.dat`, `StreamingAssets/ctable.dat`.
- Only in F: (5,094 files): essentially all `BeyondAssets/**` + UGC editor
  files (present in the torrent copy, copied nowhere else).

After the first F: launch, F:'s `Persistent/` (393 files) is a strict subset of
E:'s (395 files): F: lacks only `DownloadError.log.bak` (log rotation) and
`base_revision` (an E:-only stale file the clean game never wrote). All 391
common `Persistent/` files, including every hotfix `.blk`, have identical size
and MD5. F: also did **not** get `StreamingAssets/ctable.dat` — only
`Persistent/ctable.dat`, which is a byte-identical copy of
`StreamingAssets/ctable_streaming.dat` (MD5 `cd5f73407c6c4733f3c86a145e9f94c0`).
