#!/usr/bin/env python3
"""
Find every commit where `gircap capture`'s artifact bundle changes.

The predicate is monotone, which is what makes this a search and not a sweep:

    P(lo, hi)  ==  "some boundary lies in (lo, hi]"

is true iff F(mid) != F(lo).  One probe of `mid` splits the interval in two and
one half is provably boundary-free.  Cost is O(k log(n/k)) builds for k
boundaries among n commits, instead of n-1 adjacent comparisons.

Fingerprints are cached by probe.ps1, so re-running is free and probes
accumulate.

    python tools/capture/tools/search.py [--repo <path>] [--since <sha>]

The baseline is the parent of the first commit that touched src/ after the
refactor plan was added.  It is index 0 deliberately: without it the first
interval compares the first commit against a midpoint, which silently attributes
that commit's own change to a later one.
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

H = Path(os.environ.get("GIRCAP_PROBE_HOME", str(Path(tempfile.gettempdir()) / "gircap-probe")))
STATE = H / "search-state.json"


def git(repo, *a):
    r = subprocess.run(
        ["git", *a], cwd=repo, capture_output=True, text=True, errors="replace",
        encoding="utf-8",
    )
    return r.stdout or ""


def commit_list(repo, plan_commit):
    """The baseline, then every commit that touched src/, in order."""
    out = [git(repo, "rev-parse", "--short", f"{plan_commit}~1").strip()]
    log = git(
        repo, "log", "--reverse", "--format=%h", f"{plan_commit}~1..HEAD",
        "--", "tools/capture/src",
    )
    out += [line.strip() for line in log.splitlines() if line.strip()]
    return out


def fp_of(sha, profile):
    p = H / "fp" / profile / sha[:7]
    if not p.is_dir():
        raise SystemExit(f"no fingerprint for {sha} - {p} is missing; refusing to diff")
    return p


def same(a, b, profile, differ):
    for attempt in range(3):
        r = subprocess.run(
            [sys.executable, str(differ), "diff", str(fp_of(a, profile)), str(fp_of(b, profile)),
             "--limit", "0"],
            capture_output=True, text=True, cwd=str(H),
        )
        try:
            return json.loads(r.stdout)["identical"]
        except Exception:
            print(f"    (diff attempt {attempt}: rc={r.returncode} {r.stderr[:300]!r})", flush=True)
    raise SystemExit(f"differ failed repeatedly: {a} vs {b}")


def probe(sha, repo, profile):
    if (H / "fp" / profile / f"{sha[:7]}.done").is_file():
        return True
    r = subprocess.run(
        ["pwsh", "-NoProfile", "-File", str(H / "probe.ps1"), "-Commit", sha, "-Profile", profile],
        capture_output=True, text=True, cwd=str(H),
    )
    tag = r.stdout.strip().splitlines()[-1] if r.stdout.strip() else "NO-OUTPUT"
    print(f"  probe {sha[:7]} -> {tag}", flush=True)
    if not (H / "fp" / profile / f"{sha[:7]}.done").is_file():
        print(f"    !! {r.stdout}\n{r.stderr}", flush=True)
        return False
    return True


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", default=str(Path(__file__).resolve().parents[3]))
    ap.add_argument("--plan-commit", default=None,
                    help="the commit that added docs/03-refactor-plan.md (default: found)")
    ap.add_argument("--profile", default="default")
    args = ap.parse_args()

    repo = Path(args.repo)
    plan = args.plan_commit
    if not plan:
        plan = git(repo, "log", "--format=%H", "--diff-filter=A", "-1", "--",
                   "tools/capture/docs/03-refactor-plan.md").split()[0]
    if not plan:
        raise SystemExit("could not find the commit that added 03-refactor-plan.md; "
                         "pass --plan-commit")

    cs = commit_list(repo, plan)
    subj = {s: git(repo, "log", "-1", "--format=%s", s).strip() for s in cs}
    n = len(cs)
    differ = Path(__file__).with_name("bundle_differ.py")

    print(f"baseline {cs[0]}  ({subj[cs[0]]})")
    print(f"range    {plan}~1..HEAD, {n - 1} commits touching tools/capture/src\n")

    st = json.loads(STATE.read_text()) if STATE.is_file() else {"boundaries": []}
    probed = set(st.get("probed", []))

    if not probe(cs[0], repo, args.profile):
        raise SystemExit("baseline probe failed")
    probed.add(cs[0])

    # max index is n-1, not n: cs[hi] is read on the leaf branch
    frontier = [(0, n - 1)]
    queued = {(0, n - 1)}
    while frontier:
        lo, hi = frontier.pop(0)
        if lo + 1 == hi:
            ident = same(cs[lo], cs[hi], args.profile, differ)
            print(f"  boundary? [{lo}..{hi}] {cs[hi]} differs={not ident}", flush=True)
            if not ident:
                st["boundaries"].append(cs[hi])
            continue
        mid = (lo + hi) // 2
        if mid not in probed:
            if not probe(cs[mid], repo, args.profile):
                raise SystemExit(f"probe failed at {cs[mid]}")
            probed.add(cs[mid])
            st["probed"] = sorted(probed)
            STATE.write_text(json.dumps(st, indent=2))
        ident = same(cs[lo], cs[mid], args.profile, differ)
        print(f"  [{lo}..{hi}] mid={mid} ({cs[mid]}) same_as_{lo}={ident}", flush=True)
        # The split is asymmetric, and the asymmetry is the whole algorithm:
        #   F(lo) == F(mid)  ->  (lo, mid] has no boundary at all; drop it.
        #   F(lo) != F(mid)  ->  (lo, mid] HAS one, and (mid, hi] may have more,
        #                       so both must eventually be searched.
        # Popping one item and pushing one item back is a binary-search chain,
        # not a tree: it follows one branch and abandons the other. It reported
        # one boundary having examined half the range.
        #
        # And the frontier starts at (0, n-1), not (0, n): the leaf branch reads
        # commits[hi], and n is one past the last index. Seeding it at n crashed
        # after most of the tree was resolved, leaving an interval unexamined.
        if not ident:
            nxt, deferred = (lo, mid), (mid, hi)
        else:
            nxt, deferred = (mid, hi), None
        frontier.insert(0, nxt)
        # The deferred half can be re-derived from a later split; queue it once.
        if deferred is not None and deferred not in queued:
            queued.add(deferred)
            frontier.append(deferred)

    st["boundaries"] = sorted(set(st["boundaries"]), key=lambda s: cs.index(s))
    st["probed"] = sorted(probed)
    STATE.write_text(json.dumps(st, indent=2))

    print("\n=== boundaries: the commit whose bundle differs from the one before ===")
    for b in st["boundaries"]:
        print(f"  {b}  {subj[b]}")
    print(f"\nprobed {len(probed)}/{n}   boundaries {len(st['boundaries'])}")


if __name__ == "__main__":
    main()