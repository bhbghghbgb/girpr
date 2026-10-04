//! The dry-run plan, as a byte-exact golden string.
//!
//! Stage 4 makes `sync` lazy: the same planner `compare` uses stops it reading
//! files no verdict needs. This file exists so that "lazy" can never quietly
//! become "a different plan". The planner decides only *what to read*, and this
//! asserts that from the outside, on stdout, exactly as a user sees it.
//!
//! It runs the real binary rather than the library. That matters for two
//! reasons. A reconstructed string from `cmd_sync`'s internals would not cover
//! the `RENAME` line, which is printed by the rename pass rather than the plan
//! printer — and the rename pass is the part of `sync` most exposed to this
//! change, because `plan_pairs` runs *before* it. And it pins the real output
//! order and the real SUMMARY, so reformatting the printer cannot quietly pass.
//!
//! Everything here is insensitive-mode unless a test says otherwise, which is the
//! mode with the rename pass.

mod common;

use common::{TempRoot, opts, sync_mtime, wfile};
use std::path::Path;
use std::process::Command;

/// Run `girsync sync --dry-run` and return its stdout.
///
/// Only stdout: tracing goes to stderr and a log file, so stdout is exactly the
/// plan a user reads.
fn dry_run(src: &Path, dst: &Path, extra: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_girsync"))
        .arg("sync")
        .arg("--src")
        .arg(src)
        .arg("--dst")
        .arg(dst)
        .arg("--dry-run")
        .args(extra)
        .output()
        .expect("spawn girsync");
    assert!(
        out.status.success(),
        "dry run failed: {}\n{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    String::from_utf8(out.stdout).expect("stdout is utf-8")
}

/// The fixture, in one place, because `sync_dry_run_matches` asserts the plan this
/// layout produces and `dry_run_changes_nothing` asserts it changed nothing — a
/// golden string is only meaningful if the fixture that produced it is pinned.
///
/// Every plan line the printer can emit is represented on purpose:
///
/// | path            | state                        | plan line        |
/// | --------------- | ---------------------------- | ---------------- |
/// | `same.txt`      | identical, stat-equal        | *none*           |
/// | `differs.txt`   | different size               | `COPY`           |
/// | `Data.txt`      | same bytes, other casing     | `RENAME`         |
/// | `Case.txt`      | other casing, same size, **different bytes** | `RENAME` + `COPY` |
/// | `onlysrc.txt`   | src-only                     | `COPY`           |
/// | `onlydst.txt`   | dst-only file                | `DELETE`         |
/// | `extradir/`     | dst-only file + two dirs     | `DELETE` + 2 × `RMDIR` |
/// | `fixdir/`       | dir on src, file on dst      | `FIX-DIR` + `COPY`|
/// | `conflict.txt`  | file on src, dir on dst      | `COPY`           |
/// | `newdir/`       | src-only directory           | `MKDIR` + `COPY` |
///
/// Two of these carry the stage. `differs.txt` is the one laziness must stop
/// reading — its sizes differ, so stat settles it — and the plan must not notice.
/// `Case.txt` is the one that catches the ordering: see its own note.
fn fixture(t: &TempRoot) -> (std::path::PathBuf, std::path::PathBuf) {
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");

    // Equal on both sides: contributes nothing to the plan, which is the point.
    wfile(&src, "same.txt", b"identical");
    wfile(&dst, "same.txt", b"identical");
    sync_mtime(&src.join("same.txt"), &dst.join("same.txt"));

    // Different sizes, so stat alone settles it and no digest is needed.
    wfile(&src, "differs.txt", b"src-content-that-is-longer");
    wfile(&dst, "differs.txt", b"dst");

    // Casing only, same bytes: a rename and not a copy.
    wfile(&src, "Data.txt", b"payload");
    wfile(&dst, "data.txt", b"payload");
    sync_mtime(&src.join("Data.txt"), &dst.join("data.txt"));

    // Casing *and* content, at the same size so stat cannot settle it.
    //
    // This is the pair that guards the planner's case flag. `plan_pairs` runs
    // before the rename pass, so it has to pair by lowercase here or this pair
    // looks one-sided and neither side gets a digest; the rename then collapses
    // the keys, `hashes_differ` falls silent on dst's absent digest, and the plan
    // comes out as a bare RENAME — copying nothing and leaving dst with the wrong
    // bytes. Every other case-only fixture in the suite uses identical bytes,
    // which is exactly why that mistake passes everything else.
    wfile(&src, "Case.txt", b"payload");
    wfile(&dst, "case.txt", b"PAYLOAD");
    sync_mtime(&src.join("Case.txt"), &dst.join("case.txt"));

    // One-sided, both directions, file and directory.
    wfile(&src, "onlysrc.txt", b"only here");
    wfile(&dst, "onlydst.txt", b"only there");
    wfile(&dst, "extradir/nested/deep.bin", b"x");

    // A directory on src standing where dst has a file: the `FIX-DIR` case.
    wfile(&src, "fixdir/inner.txt", b"f");
    wfile(&dst, "fixdir", b"a blocking file");

    // A file on src standing where dst has a directory: resolved the other way.
    wfile(&src, "conflict.txt", b"a file");
    std::fs::create_dir_all(dst.join("conflict.txt")).unwrap();

    // A src-only directory, so MKDIR and a nested COPY both appear.
    wfile(&src, "newdir/nested.txt", b"nested");

    (src, dst)
}

#[test]
fn sync_dry_run_matches() {
    let t = TempRoot::new("golden");
    let (src, dst) = fixture(&t);
    let got = dry_run(&src, &dst, &[]);
    assert_eq!(got, GOLDEN, "\n--- actual ---\n{got}\n---");
}

/// The same plan under `--case-sensitive`, where there is no rename pass at all.
///
/// Kept separate from the golden string above on purpose: it is a second golden,
/// not a variation to be tolerated. Stage 4 passes `common.case_sensitive` to
/// `plan_pairs` but keeps `diff_maps`' `case_sensitive: true` — the diff is always
/// taken post-rename. That asymmetry is correct and invisible, so the only way to
/// keep it correct is to pin both modes.
#[test]
fn sync_dry_run_case_sensitive_matches() {
    let t = TempRoot::new("golden_cs");
    let (src, dst) = fixture(&t);
    let got = dry_run(&src, &dst, &["--case-sensitive"]);
    assert_eq!(got, GOLDEN_CASE_SENSITIVE, "\n--- actual ---\n{got}\n---");
}

/// A dry run must print a plan and change nothing — not the filesystem, not
/// either cache. This is the promise stage 4 is most able to break, because
/// laziness stops both sides from writing the backfill a real run would.
#[test]
fn dry_run_changes_nothing() {
    let t = TempRoot::new("golden_untouched");
    let (src, dst) = fixture(&t);
    // Warm both caches so there is something a stray write could damage.
    for dir in [&src, &dst] {
        girsync::cmd_update(
            girsync::UpdateOpts {
                dir: dir.clone(),
                common: opts(),
            },
            &common::log(),
        )
        .unwrap();
    }
    let before = snapshot(&src);
    let dst_before = snapshot(&dst);

    let _ = dry_run(&src, &dst, &[]);

    assert_eq!(snapshot(&src), before, "src is byte-identical");
    assert_eq!(
        snapshot(&dst),
        dst_before,
        "dst, cache included, is byte-identical"
    );
    assert!(
        !snapshot(&src).is_empty(),
        "the snapshot is not vacuously empty"
    );
}

/// Every byte under `dir`, cache included.
///
/// The cache is deliberately *not* filtered out. A dry run's real promise is that
/// neither cache changes, and the plan being identical says nothing about that —
/// a stray prune, a stat-only row or a `meta` rewrite would leave every plan line
/// untouched.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = walk(dir)
        .into_iter()
        .map(|(rel, p)| (rel, std::fs::read(p).unwrap_or_default()))
        .collect();
    out.sort();
    out
}

fn walk(dir: &Path) -> Vec<(String, std::path::PathBuf)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((
                    p.strip_prefix(dir).unwrap().to_string_lossy().into_owned(),
                    p,
                ));
            }
        }
    }
    out
}

/// Captured from the eager `sync`, before stage 4. Anything stage 4 changes here is
/// a plan regression, not a plan improvement — laziness may remove reads, never a
/// line.
const GOLDEN: &str = "\
RENAME case.txt -> Case.txt
RENAME data.txt -> Data.txt
MKDIR newdir
FIX-DIR fixdir
DELETE extradir/nested/deep.bin
DELETE onlydst.txt
COPY Case.txt
COPY conflict.txt
COPY differs.txt
COPY fixdir/inner.txt
COPY newdir/nested.txt
COPY onlysrc.txt
RMDIR extradir/nested
RMDIR extradir
SUMMARY renamed=2 mkdir=1 copied=6 deleted=2 rmdir=2 missing_only=false keep_extra=false dry_run=true
";

/// Same fixture, `--case-sensitive`. Note there is no rename pass in this mode, so
/// each case-only path becomes a copy plus a delete — and `Case.txt` differs in
/// bytes while `Data.txt` does not, which is visible here only as two more lines
/// of the same shape. In this mode neither pair is ever compared, so neither needs
/// a digest, which is precisely why passing `true` to `plan_pairs` would go
/// unnoticed if the insensitive golden above did not exist.
const GOLDEN_CASE_SENSITIVE: &str = "\
MKDIR newdir
FIX-DIR fixdir
DELETE case.txt
DELETE data.txt
DELETE extradir/nested/deep.bin
DELETE onlydst.txt
COPY Case.txt
COPY Data.txt
COPY conflict.txt
COPY differs.txt
COPY fixdir/inner.txt
COPY newdir/nested.txt
COPY onlysrc.txt
RMDIR extradir/nested
RMDIR extradir
SUMMARY renamed=0 mkdir=1 copied=7 deleted=4 rmdir=2 missing_only=false keep_extra=false dry_run=true
";
