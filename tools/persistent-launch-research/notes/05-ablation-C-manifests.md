# Ablation C — remove the three `*_versions_persist` mirrors

Setup: clean synced state restored from the E: backup; only
`res_versions_persist`, `data_versions_persist` and
`silence_data_versions_persist` were moved out. The base hash, all revision
files and all 390 hotfix files were intact. User launched F:, logged in, chose
region, pressed Start; the early stages flashed by as in ablation B and they
closed after "preparing to load data".

## Observation

- `DownloadError.log` appended `File num to verify = 4` /
  `CheckAllFinish return true` at 16:54:35 — the **fast path again**. No
  download, no 2632-file check, no `*_versions_remote` files.
- After the launch, `res_versions_persist` and `data_versions_persist` are
  **still missing**. `silence_data_versions_persist` was recreated byte-identical
  (MD5 `2bcf33810856cb0820725300ff2d8666`) — this matches the earlier finding
  that the silence manifest is rewritten on every clean launch.
- The game cleared the **read-only attribute on all ~390 hotfix `.blk` files**
  (`RA` -> `A`); `ctable.dat` also became plain `A`. No bytes changed.
- 0 files added, 2 removed (the two missing manifests), 397 changed (mostly
  the attribute flip).

## Conclusion

With `base_res_version_hash`, the revision files and the hotfix bytes intact,
the game skips verification **without** `res_versions_persist` and
`data_versions_persist`. Those mirrors are therefore not the immediate
"skip verification" switch; they are the asset database used when revisions change
or a new hotfix arrives. `docs/09`'s decision to write them (C3/C19) remains the
safe behaviour — the game regenerates `silence_*` on every launch, and having the
res/data mirrors present avoids relying on a dispatcher round trip — but the
ablation shows the *load-bearing* file is unambiguously
`base_res_version_hash` (ablation A).

This also confirms the game itself strips read-only flags on hotfix files, so
girpr's per-file read-only handling (C17(b)) is parity, not a novelty.

State restored from the E: backup; final F: state verified at 393 files.
