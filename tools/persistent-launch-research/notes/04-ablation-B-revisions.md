# Ablation B — remove `res_revision` + `res_revision_eternal`

Setup: clean synced state restored from the E: backup; only the two `res_*`
revision files were moved out. Every other marker, the three persisted
manifests and all 393 hotfix files were intact. User launched F:, logged in,
chose region, pressed Start; the early stages flashed by with a brief freeze
before "preparing to load data" (the dispatcher re-query / file rewrites), then
closed the game.

## Observation

- `DownloadError.log` appended `File num to verify = 4` /
  `CheckAllFinish return true` at 16:46:00 — the **same fast path** as an
  untouched launch. No download, no 2632-file check, no `*_versions_remote`.
- The game **recreated both files with the correct live values**
  (`49530578`), and also refreshed `res_versions_persist`,
  `silence_data_versions_persist`, `PatchDone`, `silence_revision`,
  `base_res_version_hash`, `ChannelName`, `ScriptVersion`, `CodeSwitch.config`,
  `test_file`, `DownloadPref` (mtime only; byte-identical manifests/revisions).
- 0 files added/removed; 23 changed (13 marker mtimes + 10 SDK cache files).
  The "freeze" the user noticed corresponds to this re-query + rewrite burst
  between `OnConnectRegionDispatch` (16:45:58) and `File num to verify = 4`
  (16:46:00).

## Conclusion

The revision pair is **self-healing**: with `base_res_version_hash` and the
asset database intact, the game re-queries the dispatcher, rewrites the revision
files and still skips verification. This does not contradict `docs/09` (writing
them is still correct and avoids the freeze / extra network round trip), but it
shows they are *not* the master switch. The master switch is
`base_res_version_hash` (ablation A). `res_revision_eternal` is regenerated
alongside `res_revision` from the same live value, which corroborates the
dual-write requirement in GenshinSlimmer issue #19.

State restored from the E: backup.
