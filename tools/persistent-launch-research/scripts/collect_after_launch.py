#!/usr/bin/env python3
"""One-shot collector to run after the user has launched + closed the F: game.

Copies the game's user-data logs (so a subsequent launch does not lose them),
takes a fresh snapshot of the F: game tree, and prints a diff against the
baseline snapshot. All reads are sequential; E: is not touched.

Usage: python collect_after_launch.py [LABEL]   (default label: after-launch)
"""
import os
import shutil
import subprocess
import sys

BASE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GAME = r"F:\Genshin Impact game"
ULOG = os.path.join(os.environ["USERPROFILE"], "AppData", "LocalLow", "miHoYo", "Genshin Impact")
SNAP_BASE = os.path.join(BASE, "snapshots", "F-fresh", "F-snapshot.jsonl")


def main():
    label = sys.argv[1] if len(sys.argv) > 1 else "after-launch"
    outdir = os.path.join(BASE, "logs", label)
    os.makedirs(outdir, exist_ok=True)
    for name in ("info.txt", "LocalLog.log", "output_log.txt",
                 "output_log.txt.last", "UidInfo.txt",
                 "security_server_default_iplist_os_asia.txt",
                 "il2cpp-codeswitch.dat"):
        src = os.path.join(ULOG, name)
        if os.path.exists(src):
            shutil.copy2(src, os.path.join(outdir, name))
    sdk = os.path.join(ULOG, "logs")
    if os.path.isdir(sdk):
        for name in ("NativeSDK.log", "MiHoYoSDK.log"):
            src = os.path.join(sdk, name)
            if os.path.exists(src):
                shutil.copy2(src, os.path.join(outdir, name))
    print("logs ->", outdir)

    snap = os.path.join(BASE, "snapshots", "F-fresh", f"F-{label}.jsonl")
    subprocess.run([sys.executable, os.path.join(BASE, "scripts", "snapshot.py"),
                    GAME, snap, "--hash-max-size", "2097152"], check=True)
    subprocess.run([sys.executable, os.path.join(BASE, "scripts", "diff_snap.py"),
                    SNAP_BASE, snap], check=False)


if __name__ == "__main__":
    main()
