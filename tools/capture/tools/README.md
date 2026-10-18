# `gircap` bundle differ

The instrument behind [`../docs/04-output-contract-history.md`](../docs/04-output-contract-history.md).
Read that document for what it found and
[`../docs/05-bundle-differ.md`](../docs/05-bundle-differ.md) for why it is
built this way. This file is how to run it.

Standard library only. Nothing here is part of the crate: `gircap` has no
dependency on these scripts and they have no dependency on the tool beyond
reading bundles and, for the probe driver, running the `gircap` binary.

## What each file is

| file | what it does |
|---|---|
| `bundle_differ.py` | **the instrument.** normalises a bundle and compares two of them, file-set first then byte-for-byte |
| `mutation_control.py` | proves the instrument: injects the nine kinds of change this history made and requires all nine to be caught |
| `search.py` | the divide-and-conquer that finds which commits moved the output |
| `probe.ps1` | build one commit and capture it twice; fingerprints and rejects it if the two disagree |
| `zz_probe.rs` | the capture driver, copied into `tests/` by `probe.ps1` |
| `zz_probe_arm.rs` | the same, plus the mock arming that produces degraded inputs |
| `mockpatch.py` | teaches the loopback mock to misbehave; anchored, refuses to half-apply |

## Running it

```powershell
# one commit
pwsh -File tools/capture/tools/probe.ps1 -Commit 7771323 -Profile default

# the whole search (caches; safe to re-run, safe to interrupt)
python tools/capture/tools/search.py

# prove the differ still works - run this after any edit to bundle_differ.py
python tools/capture/tools/mutation_control.py
```

Set `GIRCAP_PROBE_HOME` to choose the scratch root; it defaults to
`%TEMP%\gircap-probe`. A `git worktree` and a shared `CARGO_TARGET_DIR` live
under it, which is what keeps a rebuild at ~3 minutes instead of ~8.

## The two rules that make it mean something

1. **Never compare one binary to itself and conclude "nothing changed".** That
   is `tests/bundle_diff.rs`'s job, and it is blind by construction to a change
   applied uniformly to every capture — which is the entire subject here. This
   tool exists to compare *across* binaries.
2. **Prove the differ before trusting it.** `mutation_control.py` is that proof,
   including the negative control: an unmutated copy must read identical, or a
   harness that always fires would prove nothing.

Both exist because this work went wrong without them: an early version reported
13 of 58 documents as changed between two commits that had changed nothing,
because the mock's ephemeral port is recorded throughout a bundle. A self-check
that only ever compares one binary to itself cannot find a field that one binary
happens to share with itself.

## Before changing `bundle_differ.py`

Re-run `mutation_control.py`. If a new exemption matches nothing, the differ
fails rather than quietly comparing nothing — that is deliberate, and it is the
same rule `tests/bundle_diff.rs` states about its own exemptions. The cheapest
way to find a missing volatile field is to diff **raw** first: `save --raw` and
compare two captures before normalising anything, and read what actually moved.