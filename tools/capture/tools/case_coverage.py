#!/usr/bin/env python3
"""The instrument behind `../docs/00-spec.md` section 4.1.

Standard library only, like every other script in this directory. It reads a
bundle and nothing else: no network, no writes, no `gircap` binary.

It is worth re-running after any change to the class set: the populations it
measures are the ones `docs/00-spec.md` section 4.1 publishes, and a class
added or a predicate moved makes them stale.

Three sections, one per question the findings ask:

1. **population** - how many paths satisfy each class, old set and proposed
   set, over the pool the capture selects (`game` + every selected audio
   field). The scoring is `gircap`'s own: an identity is
   `(uncompressed_md5, uncompressed_size)`, a chunk is *reused* when the old
   file carries that identity and *moved* when the old file's offset for it
   differs from the new chunk's.

2. **outcome** - for each capture in the bundle, what the *new* in-place
   repair would do with the file: which chunks are already correct where they
   belong (kept), which have their bytes elsewhere in the file (moved), which
   have no usable local source (fetched), how much RAM the staged tier would
   need, how many sources each write order would salvage, and whether the trim
   is eager or deferred. The fixture's bytes are hashed at each entry's target
   and source range.

3. **ranking** - the cheapest candidate per class, by `gircap`'s own key
   (`total_cost` = the compressed bytes of the old and the new file), and the
   overlap between the proposed classes.

Usage:

    python tools/capture/tools/case_coverage.py <bundle-dir>
    python tools/capture/tools/case_coverage.py <bundle-dir> --section population
"""

import argparse
import gzip
import hashlib
import json
import os
import sys
from collections import Counter, defaultdict

# The class sets, as predicates over the old and the new manifest entries.
#
# `Shape` in `src/select/classes.rs` is deliberately lossy ("the numbers a
# decision turns on and nothing else"); the axes below are what it carries.
# They are spelled out here rather than imported, because the script must be
# able to measure a class set the crate does not have — that is how a gap is
# found in the first place.
#
# A path is described by:
#   reused   - new chunks whose identity is in this same old file
#   moved    - of those, how many sit at a different offset in the old file
#   up       - of the moved ones, how many move *later* (source < destination)
#   down     - of the moved ones, how many move *earlier* (source > destination)
#   n        - new chunks
#   old/new  - the two sizes
#   trim_free - no old chunk reaches past the new size (the eager trim)

OLD_CLASSES = [
    "identical",
    "zero_reuse_single",
    "zero_reuse_multi",
    "partial_same_offset",
    "partial_moved_offset",
    "partial_size_changed",
    "added",
    "removed",
    "full_reuse_reordered",
]

PROPOSED_CLASSES = [
    "identical",
    "zero_reuse_single",
    "zero_reuse_multi",
    "kept_in_place",
    "moved_ascending",
    "moved_descending",
    "moved_mixed",
    "trim_eager",
    "trim_deferred",
    "added",
    "removed",
]


def identity(chunk):
    return (chunk["uncompressed_md5"].lower(), chunk["uncompressed_size"])


def read_manifest(bundle, tag, field):
    """The decoded manifest for one tag and field.

    **The dump is gzipped only when it is big.** `--gzip-threshold` decides,
    so a small bundle's dump is the plain `<field>.manifest.json` and a large
    one's is `<field>.manifest.json.gz`. The instrument reads both, or it only
    works on the reference bundle.
    """
    root = os.path.join(bundle, "manifests", tag)
    gz = os.path.join(root, f"{field}.manifest.json.gz")
    if os.path.isfile(gz):
        with gzip.open(gz, "rt", encoding="utf-8") as fh:
            return json.load(fh)
    plain = os.path.join(root, f"{field}.manifest.json")
    with open(plain, encoding="utf-8") as fh:
        return json.load(fh)


def fields_of(bundle):
    """The `matching_field`s the bundle carries, `game` first."""
    root = os.path.join(bundle, "manifests")
    out = []
    for tag in sorted(os.listdir(root)):
        for name in sorted(os.listdir(os.path.join(root, tag))):
            for suffix in (".manifest.json.gz", ".manifest.json"):
                if name.endswith(suffix):
                    out.append(name[: -len(suffix)])
    seen = []
    for f in out:
        if f not in seen:
            seen.append(f)
    if "game" in seen:
        seen.remove("game")
        seen.insert(0, "game")
    return seen


def describe(entry_new, entry_old, index):
    """One path's shape, in the terms both class sets are written in."""
    n = len(entry_new["chunks"])
    reused = moved = fetch = up = down = payload = 0
    max_end = 0
    for c in entry_new["chunks"]:
        offsets = index.get(identity(c))
        if offsets is None:
            fetch += 1
            continue
        reused += 1
        # The FIRST old chunk with that identity: the same rule
        # `repair::reuse::sources_for` uses, and the same one the patchmap's
        # `source_range` records.
        src = offsets[0]
        max_end = max(max_end, src + c["uncompressed_size"])
        if src != c["offset"]:
            moved += 1
            payload += c["uncompressed_size"]
            if src > c["offset"]:
                down += 1
            else:
                up += 1
    return dict(
        n=n,
        reused=reused,
        moved=moved,
        fetch=fetch,
        up=up,
        down=down,
        payload=payload,
        old=entry_old["size"],
        new=entry_new["size"],
        trim_free=max_end <= entry_new["size"],
    )


def old_class_of(row, side):
    if side == "identical":
        return {"identical"}
    if side == "added":
        return {"added"}
    if side == "removed":
        return {"removed"}
    out = set()
    partial = 0 < row["reused"] < row["n"]
    if row["reused"] == 0:
        out.add("zero_reuse_single" if row["n"] == 1 else "zero_reuse_multi")
    if partial and row["moved"] == 0 and row["old"] == row["new"]:
        out.add("partial_same_offset")
    if partial and row["moved"] > 0:
        out.add("partial_moved_offset")
    if partial and row["moved"] == 0 and row["old"] != row["new"]:
        out.add("partial_size_changed")
    if row["reused"] == row["n"] and row["moved"] > 0:
        out.add("full_reuse_reordered")
    return out


def proposed_class_of(row, side):
    if side == "identical":
        return {"identical"}
    if side == "added":
        return {"added"}
    if side == "removed":
        return {"removed"}
    out = set()
    partial = 0 < row["reused"] < row["n"]
    if row["reused"] == 0:
        # The crate's rule: `nchunks == 1` is the single-chunk class and
        # `nchunks > 1` the multi one, so an empty file is neither.
        if row["n"] == 1:
            out.add("zero_reuse_single")
        elif row["n"] > 1:
            out.add("zero_reuse_multi")
    if partial and row["moved"] == 0:
        out.add("kept_in_place")
    if row["moved"] > 0:
        if row["up"] == 0:
            out.add("moved_ascending")
        elif row["down"] == 0:
            out.add("moved_descending")
        else:
            out.add("moved_mixed")
    if partial and row["old"] > row["new"]:
        out.add("trim_eager" if row["trim_free"] else "trim_deferred")
    return out


def collect(bundle):
    """Every path of every field, described."""
    rows = []
    for field in fields_of(bundle):
        old = {e["file"]: e for e in read_manifest(bundle, "7.0.0", field)["entries"]}
        new = {e["file"]: e for e in read_manifest(bundle, "7.1.0", field)["entries"]}
        index = {}
        for e in old.values():
            m = defaultdict(list)
            for c in e["chunks"]:
                m[identity(c)].append(c["offset"])
            index[e["file"]] = m
        for path, en in new.items():
            eo = old.get(path)
            if eo is None:
                rows.append(
                    dict(
                        field=field,
                        path=path,
                        side="added",
                        cost=sum(c["compressed_size"] for c in en["chunks"]),
                        disk=en["size"],
                    )
                )
                continue
            if en["md5"] == eo["md5"]:
                rows.append(
                    dict(
                        field=field,
                        path=path,
                        side="identical",
                        cost=sum(c["compressed_size"] for c in eo["chunks"])
                        + sum(c["compressed_size"] for c in en["chunks"]),
                        disk=eo["size"] + en["size"],
                    )
                )
                continue
            row = describe(en, eo, index[path])
            row.update(
                field=field,
                path=path,
                side="changed",
                cost=sum(c["compressed_size"] for c in eo["chunks"])
                + sum(c["compressed_size"] for c in en["chunks"]),
                disk=eo["size"] + en["size"],
            )
            rows.append(row)
        for path, eo in old.items():
            if path not in new:
                rows.append(
                    dict(field=field, path=path, side="removed", cost=eo["size"], disk=eo["size"])
                )
    return rows


def population_counts(rows):
    """How many paths satisfy each class, per class set."""
    counts = {}
    for label, classifier, names in (
        ("current", old_class_of, OLD_CLASSES),
        ("proposed", proposed_class_of, PROPOSED_CLASSES),
    ):
        c = Counter()
        for r in rows:
            for name in classifier(r, r["side"]):
                c[name] += 1
        counts[label] = {name: c.get(name, 0) for name in names}
    counts["pool"] = len(rows)
    return counts


def section_population(rows):
    print("== population ==")
    print(f"pool: {len(rows)} paths\n")
    for label, classifier, names in (
        ("current", old_class_of, OLD_CLASSES),
        ("proposed", proposed_class_of, PROPOSED_CLASSES),
    ):
        print(f"-- {label} class set --")
        counts = Counter()
        for r in rows:
            for c in classifier(r, r["side"]):
                counts[c] += 1
        for name in names:
            print(f"  {name:<24} {counts.get(name, 0):>6}")
        print()


def section_ranking(rows):
    print("== ranking (the proposed set) ==")
    for r in rows:
        r["classes"] = proposed_class_of(r, r["side"])
    for name in PROPOSED_CLASSES:
        sel = [r for r in rows if name in r["classes"]]
        if not sel:
            print(f"{name:<24} (no candidate)")
            continue
        sel.sort(key=lambda r: (r["cost"], r["field"], r["path"]))
        b = sel[0]
        others = sorted(c for c in b["classes"] if c != name)
        print(
            f"{name:<24} n={len(sel):<6} {b['field']:<6} {b['path']:<62} "
            f"cost={b['cost']:>10} disk={b['disk']:>10}"
        )
        if b["side"] == "changed":
            print(
                f"{'':<24}   n={b['n']} reused={b['reused']} moved={b['moved']} "
                f"up={b['up']} down={b['down']} fetch={b['fetch']} "
                f"payload={b['payload']} old={b['old']} new={b['new']} "
                f"trim_free={b['trim_free']}"
            )
        print(f"{'':<24}   also satisfies: {others}")
    print()


def md5(data):
    return hashlib.md5(data).hexdigest()


def outcome_rows(bundle):
    """What the in-place repair would do with each capture's file.

    **The rules are the artifact's** (`assemble::repair_of` in the crate), not a
    second opinion, and the differences matter:

    - a chunk that is already correct where it belongs is **kept**, and it is
      not a candidate for either write order. The salvage counts run over the
      *damaged* chunks only, exactly as `repair::reuse::salvage_order` does.
    - a source that reaches past the end of the file is not there to be
      copied, so it salvages nothing in either order.
    - a tie keeps nothing at all, so there is no direction to name.

    A file that is absent from `game/` is the *created* case: every chunk is
    fetched and nothing is kept.
    """
    pmdir = os.path.join(bundle, "patchmaps")
    out = []
    for name in sorted(os.listdir(pmdir)):
        if not name.endswith(".json"):
            continue
        j = json.load(open(os.path.join(pmdir, name), encoding="utf-8"))
        cid = j["capture_id"]
        if not j.get("plan"):
            out.append(
                dict(
                    capture_id=cid,
                    path=j["path"],
                    note="no plan",
                    kept=0,
                    moved=0,
                    fetched=0,
                    payload=0,
                    keeps_ascending=0,
                    keeps_descending=0,
                    direction="n/a",
                    trim="none",
                    growth=0,
                )
            )
            continue
        game = os.path.join(bundle, "game", j["path"].replace("/", os.sep))
        new_size = j["new"]["size"]
        if not os.path.isfile(game):
            # The file is created: absent locally, so every chunk is fetched.
            out.append(
                dict(
                    capture_id=cid,
                    path=j["path"],
                    note="absent from game/: the file is created",
                    kept=0,
                    moved=0,
                    fetched=len(j["plan"]),
                    payload=0,
                    keeps_ascending=0,
                    keeps_descending=0,
                    direction="n/a",
                    trim="none",
                    growth=new_size,
                )
            )
            continue
        data = open(game, "rb").read()
        old_size = len(data)
        kept = moved = fetched = 0
        payload = 0
        asc = desc = 0
        up = down = 0
        max_src_end = 0
        for e in j["plan"]:
            t0, t1 = e["target_range"]["start"], e["target_range"]["end"]
            want = e["uncompressed_md5"]
            at_dest = md5(data[t0:t1]) == want
            src = e["source_range"]
            if src is None:
                fetched += 1
                continue
            s0 = src["start"]
            intact = md5(data[s0 : s0 + (t1 - t0)]) == want
            max_src_end = max(max_src_end, s0 + (t1 - t0))
            if at_dest:
                # Kept: already correct where it belongs. Not a candidate for
                # either write order - the salvage counts are over the damaged
                # chunks only.
                kept += 1
                continue
            if not intact:
                fetched += 1
                continue
            moved += 1
            payload += t1 - t0
            if s0 > t0:
                down += 1
            elif s0 < t0:
                up += 1
            # The ordered tier's rule, on the wrong chunks only, and only for
            # a source that is inside the file being read from.
            inside = s0 + (t1 - t0) <= old_size
            if inside:
                if s0 >= t0:
                    asc += 1
                if s0 <= t0:
                    desc += 1
        if new_size >= old_size:
            trim = "none"
        elif max_src_end <= new_size:
            trim = "eager"
        else:
            trim = "deferred"
        if asc == 0 and desc == 0:
            direction = "n/a"
        elif desc > asc:
            direction = "descending"
        else:
            direction = "ascending"
        out.append(
            dict(
                capture_id=cid,
                path=j["path"],
                note=None,
                kept=kept,
                moved=moved,
                fetched=fetched,
                payload=payload,
                keeps_ascending=asc,
                keeps_descending=desc,
                direction=direction,
                trim=trim,
                growth=max(0, new_size - old_size),
            )
        )
    return out


def section_outcome(rows):
    print("== outcome (what the in-place repair would do) ==")
    for r in rows:
        if r["note"]:
            print(f"{r['capture_id']:<24} ({r['note']}: {r['path']})")
            continue
        trim = {
            "eager": "eager (the dead tail is released before the first write)",
            "deferred": "deferred (a source reaches past the target's end)",
            "none": "none (the file does not shrink)",
        }[r["trim"]]
        print(
            f"{r['capture_id']:<24} kept={r['kept']} moved={r['moved']} "
            f"fetched={r['fetched']} payload={r['payload']:>9} "
            f"asc_keeps={r['keeps_ascending']} desc_keeps={r['keeps_descending']} "
            f"-> {r['direction']} | trim: {trim}"
        )
    print()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("bundle", help="a bundle directory (the one with MANIFEST.json)")
    ap.add_argument(
        "--section",
        choices=["population", "outcome", "ranking", "all"],
        default="all",
    )
    ap.add_argument(
        "--json",
        action="store_true",
        help="print the sections as JSON instead of as tables (for the gate)",
    )
    args = ap.parse_args()
    bundle = args.bundle
    if not os.path.isfile(os.path.join(bundle, "MANIFEST.json")):
        print(f"{bundle} is not a bundle (no MANIFEST.json)", file=sys.stderr)
        return 1
    doc = {}
    if args.section in ("population", "ranking", "all"):
        rows = collect(bundle)
        if args.section in ("population", "all"):
            doc["population"] = population_counts(rows)
            if not args.json:
                section_population(rows)
        if args.section in ("ranking", "all") and not args.json:
            section_ranking(rows)
    if args.section in ("outcome", "all"):
        out = outcome_rows(bundle)
        doc["outcome"] = out
        if not args.json:
            section_outcome(out)
    if args.json:
        print(json.dumps(doc, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
