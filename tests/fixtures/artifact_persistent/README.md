# gircap artifact

- **tool** gircap 0.1.0
- **region** `hk4e_global`
- **captured** 2026-11-24T18:46:27Z (`20261124T184627.158579Z`, windows)

**complete**: `7.0.0` (`sMOCKold00`) → `7.1.0` (`sMOCKnew00`)

selfcheck: **Pass** — 84 of 84 assertions passed

## Matrix

| class | status | capture | path | field | bytes |
|---|---|---|---|---|---|
| `identical` | captured | identical | `GenshinImpact_Data/StreamingAssets/Audio/en-us/vo_all` | en-us | 512 |
| `no_reuse` | captured | no_reuse | `GenshinImpact_Data/StreamingAssets/blocks/zero.blk` | game | 1024 |
| `reuse_in_place` | captured | reuse_in_place | `GenshinImpact_Data/Plugins/HoYoChannel.dll` | game | 3072 |
| `reuse_moved` | captured | reuse_moved | `GenshinImpact_Data/StreamingAssets/blocks/deferred.blk` | game | 3072 |
| `added` | captured | added | `GenshinImpact_Data/Plugins/libNew.dll` | game | - |
| `removed` | captured | removed | `GenshinImpact_Data/StreamingAssets/blocks/old.blk` | game | 32768 |
| `persistent` | unavailable | - | `-` | - | - |

## Totals

- 6 files captured, 16 chunk blobs stored
- 51994 B downloaded, 195849 B on disk
- selection pool: `game`, `en-us`
- `materialize_new`: 10 chunks
- `materialize_old`: 11 chunks
- `patch_fetch`: 5 chunks

## Re-verifying

```
gircap verify --artifact <this directory>
```

Offline: no network, no writes. Exit 0 if every assertion still holds.
