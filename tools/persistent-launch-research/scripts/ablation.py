#!/usr/bin/env python3
"""Reversible marker-ablation driver for the F: game copy.

Never run while the game is open. All state lives on E: (separate spindle from
F:), so backup/restore do not contend with F: reads.

Usage:
  python ablation.py backup              # copy F: Persistent -> E: backup
  python ablation.py remove NAME [NAME...]  # move named files out of F: Persistent
  python ablation.py restore             # delete F: Persistent, copy backup back
  python ablation.py status              # list removed files and backup info
"""
import os
import shutil
import stat
import sys

PERSIST = r"F:\Genshin Impact game\GenshinImpact_Data\Persistent"
BACKUP = r"E:\Symlinks\Temp\opencode\girpr-test\persist-backup"
REMOVED = BACKUP + "_removed"


def status():
    print("persist:", PERSIST, "exists=", os.path.isdir(PERSIST))
    print("backup :", BACKUP, "exists=", os.path.isdir(BACKUP))
    if os.path.isdir(REMOVED):
        print("removed (staged out):")
        for n in sorted(os.listdir(REMOVED)):
            print("  ", n)


def clear_readonly(root):
    """Game hotfix files are often read-only; clear that before deleting."""
    for dirpath, _dirs, files in os.walk(root):
        for name in files:
            p = os.path.join(dirpath, name)
            try:
                os.chmod(p, stat.S_IWRITE | stat.S_IREAD)
            except OSError:
                pass


def backup():
    os.makedirs(os.path.dirname(BACKUP), exist_ok=True)
    if os.path.isdir(BACKUP):
        shutil.rmtree(BACKUP)
    shutil.copytree(PERSIST, BACKUP)
    print("backup ->", BACKUP)


def remove(names):
    if not os.path.isdir(BACKUP):
        raise SystemExit("run 'backup' first")
    os.makedirs(REMOVED, exist_ok=True)
    for name in names:
        src = os.path.join(PERSIST, name)
        dst = os.path.join(REMOVED, name)
        if not os.path.isfile(src):
            raise SystemExit(f"not a file: {src}")
        if os.path.exists(dst):
            raise SystemExit(f"already removed: {name} (restore first)")
        clear_readonly(os.path.dirname(src))
        shutil.move(src, dst)
        print("removed", name)


def restore():
    if not os.path.isdir(BACKUP):
        raise SystemExit("run 'backup' first")
    if os.path.isdir(PERSIST):
        clear_readonly(PERSIST)
        shutil.rmtree(PERSIST)
    shutil.copytree(BACKUP, PERSIST)
    if os.path.isdir(REMOVED):
        clear_readonly(REMOVED)
        shutil.rmtree(REMOVED)
    print("restored", PERSIST, "from", BACKUP)


if __name__ == "__main__":
    cmd, *args = sys.argv[1:]
    {"backup": lambda: backup(), "remove": lambda: remove(args),
     "restore": lambda: restore(), "status": lambda: status()}[cmd]()
