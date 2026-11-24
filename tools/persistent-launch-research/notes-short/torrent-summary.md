# Torrent summary (extract of the gitignored `notes/torrent-files.txt`)

Regenerate the full 8,068-line list with:

```
python scripts/torrent_inspect.py <F:\*.torrent> > notes/torrent-files.txt
```

| property | value |
|---|---|
| torrent name | `Genshin Impact game` |
| files | 8,068 |
| total | 155,182,786,241 bytes (144.53 GiB) |
| piece length | 4,194,304; pieces = 36,999 |
| announce | `http://bt.t-ru.org/ann` |

Relevant entries (the questions this research asked):

- **No `GenshinImpact_Data/Persistent/` directory at all** in the torrent — no
  revision files, no `*_versions_persist`, no markers, no hotfix files.
- **No root `config.ini`.**
- `GenshinImpact_Data/StreamingAssets/` contains `res_versions_streaming`
  (375,889 B), `data_versions_streaming` (71,416 B),
  `silence_versions_streaming` (521 B), `ctable_streaming.dat` (28,160,000 B),
  `VideoAssets/video_versions_streaming` (46,377 B), and
  `AssetBundles/{res,data,silence}_revision` (8 B each).
- Only `StreamingAssets/Persistent/plugindelayed.config` (240 B) exists inside
  the torrent — a plugin config, unrelated to the game's top-level
  `GenshinImpact_Data/Persistent/`.
- Root has `pkg_version`, `Audio_English(US)_pkg_version`, `beyond_pkg_version`,
  `GenshinImpact.exe`, `mhypbase.dll`, `HoYoKProtect.sys`, `rtlbase.dll`, plus
  the whole `BeyondAssets/` tree (UGC editor, out of scope).
- The torrent's path set is **exactly** the fresh F: copy: 0 missing, 0 extra.

Installation page (gitignored HTML at `F:\`) supplies the launcher's
`config.ini` template:

```ini
[General]
game_version=7.2.0
channel=1
sub_channel=0
cps=hoyoverse
uapc={"hk4e_global":{"uapc":"f70c36d30125_"},"hyp":{"uapc":"9daf59024777_473e0dd97e651fe6"}}
downloading_mode=
wpf_version=7.2.0.49085326
plugin_OQKz9izioj_version=1.0.0
```

E:'s actual `config.ini` instead has `game_biz=hk4e_global`,
`cps=hyp_hoyoverse`, empty `sdk_version`, and no `uapc`/plugin keys. Root
`config.ini` is launcher bookkeeping; the game never created one on F:.
