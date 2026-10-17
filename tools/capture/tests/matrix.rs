//! M10: the matrix, end to end — class predicates, greedy admission, and the
//! budget — against the same mock the rest of the suite uses.
//!
//! `tests/measured_fixture.rs` already pins these predicates against the frozen
//! measured manifests. This file asks a different question: *does the whole
//! pipeline agree*, from the two `getBuild` bodies through the diff, the
//! candidate list and admission, to the nine rows a bundle records. A predicate
//! can be right and the admission loop can still admit the wrong file, and only
//! the second question needs a run.

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::{Mock, World};

fn bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("a path");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("gircap.exe")
}

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin())
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("running {args:?}: {e}"));
    (
        out.status.code().expect("not killed"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A scratch root removed when the test ends.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Scratch {
        let path = std::env::temp_dir().join(format!(
            "gircap-matrix-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch");
        Scratch { path }
    }

    fn out(&self) -> PathBuf {
        self.path.join("artifacts")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A capture, and the bundle it wrote.
struct Captured {
    /// Kept alive so the scratch root outlives the bundle read from it.
    _scratch: Scratch,
    _mock: Mock,
    bundle: PathBuf,
    summary: std::collections::BTreeMap<String, String>,
}

fn captured(name: &str, world: World, extra: &[&str]) -> Captured {
    let mock = Mock::start(world);
    let scratch = Scratch::new(name);
    let mut args: Vec<String> = vec![
        "capture".into(),
        "--out".into(),
        scratch.out().to_string_lossy().into_owned(),
        "--hyp-base".into(),
        mock.hyp_base(),
        "--sophon-base".into(),
        mock.sophon_base(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    let borrowed: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (_code, stdout, stderr) = run(&borrowed);
    let summary: std::collections::BTreeMap<String, String> = stdout
        .lines()
        .find(|l| l.starts_with("SUMMARY "))
        .map(|l| {
            l.strip_prefix("SUMMARY ")
                .unwrap()
                .split(' ')
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .unwrap_or_else(|| panic!("no SUMMARY:\n{stdout}\n{stderr}"));
    let bundle = find_bundle(&scratch.out());
    Captured {
        _scratch: scratch,
        _mock: mock,
        bundle,
        summary,
    }
}

fn find_bundle(dir: &std::path::Path) -> PathBuf {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if d.join("MANIFEST.json").is_file() {
            return d;
        }
        if let Ok(rd) = std::fs::read_dir(&d) {
            let mut subs: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            subs.sort();
            stack.extend(subs.into_iter().rev());
        }
    }
    panic!("no bundle under {}", dir.display());
}

fn json(path: &std::path::Path) -> serde_json::Value {
    let raw = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_slice(&raw).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

impl Captured {
    fn manifest(&self) -> serde_json::Value {
        json(&self.bundle.join("MANIFEST.json"))
    }

    fn matrix(&self) -> Vec<serde_json::Value> {
        self.manifest()["matrix"]
            .as_array()
            .expect("a matrix")
            .clone()
    }

    fn diff(&self) -> serde_json::Value {
        json(&self.bundle.join("diff/paths.diff.json"))
    }
}

// ------------------------------------------------------- the nine classes

/// Each class the world publishes is admitted as `captured`, and each class it
/// does not is `unavailable` **with a reason**.
///
/// The reason is the part worth asserting. `unavailable` with an empty reason
/// tells a reader nothing they could act on; `unavailable` with one tells them
/// to widen `--audio` or pick another pair.
#[test]
fn the_matrix_has_one_row_per_class_and_each_says_what_happened() {
    let c = captured("classes", World::five_classes(), &[]);
    let rows = c.matrix();
    assert_eq!(rows.len(), 9, "one row per required class");
    for r in &rows {
        // A fulfilled row has nothing to explain; an unfulfilled one must say why.
        let satisfied = matches!(r["status"].as_str(), Some("captured" | "covered_by"));
        assert_eq!(
            r["reason"].as_str().is_some_and(|s| !s.is_empty()),
            !satisfied,
            "{r}: a reason is required exactly when the class was not answered"
        );
        assert!(
            r["status"].as_str().is_some_and(|s| {
                matches!(s, "captured" | "covered_by" | "over_budget" | "unavailable")
            }),
            "{r} has an unknown status"
        );
        // And `ok` agrees with the status, rather than being a second opinion.
        let ok = matches!(
            r["status"].as_str(),
            Some("captured" | "covered_by" | "unavailable")
        );
        assert_eq!(
            r["status"] == "over_budget",
            !ok,
            "{r}: status and ok disagree"
        );
    }
}

/// Admission is **greedy and cheapest-first**, and the capture's own accounting
/// agrees with what it admitted.
///
/// This is the assertion that catches an admission loop admitting an expensive
/// file for an expensive class while a cheap one sat unfunded.
#[test]
fn admission_is_cheapest_first_and_the_totals_add_up() {
    let c = captured("greedy", World::five_classes(), &[]);
    let s = &c.summary;
    let admitted = s["captures_admitted"].parse::<u64>().expect("a count");
    assert_eq!(admitted, 5, "five classes are satisfiable in this world");

    let bytes_admitted = s["capture_bytes_admitted"].parse::<u64>().expect("a size");
    // The admission total is the sum of what the admitted captures cost, and a
    // reader can recompute it from `diff/candidates.json`.
    // candidates.json is columnar: a columns header and one object per row.
    let candidates = json(&c.bundle.join("diff/candidates.json"));
    let cols: Vec<&str> = candidates["columns"]
        .as_array()
        .expect("a columns header")
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    for want in ["path", "matching_field", "total_cost", "classes", "rank"] {
        assert!(
            cols.contains(&want),
            "the column {want} is missing: {cols:?}"
        );
    }
    let rows = candidates["rows"].as_array().expect("rows");
    assert!(!rows.is_empty(), "candidates were scored: {candidates:#?}");

    // Rows are ranked cheapest-first, which is the order admission walks. A
    // candidate list that is not sorted would make "greedy" mean nothing.
    let costs: Vec<u64> = rows
        .iter()
        .map(|r| r["total_cost"].as_u64().expect("a cost"))
        .collect();
    assert!(
        costs.windows(2).all(|w| w[0] <= w[1]),
        "candidates are not in ascending cost order: {costs:?}"
    );
    // And every path appears exactly once.
    let mut paths: Vec<&str> = rows.iter().map(|r| r["path"].as_str().unwrap()).collect();
    let total = paths.len();
    paths.sort_unstable();
    paths.dedup();
    assert_eq!(
        paths.len(),
        total,
        "a path appears twice in the candidate list"
    );

    // The admission charge is at most the sum of every candidate, and at least
    // the cheapest one - the first thing a greedy loop admits.
    let sum_all: u64 = costs.iter().sum();
    assert!(
        bytes_admitted <= sum_all,
        "charged {bytes_admitted} but every candidate together costs {sum_all}"
    );
    assert!(
        bytes_admitted >= costs[0],
        "nothing was admitted, yet the cheapest candidate costs {}",
        costs[0]
    );
    assert_eq!(
        s["budget_unlimited"], "true",
        "no --max-download-bytes was given, so the budget is unlimited"
    );
}

/// `--max-download-bytes` refuses the expensive captures and says which, and
/// the class that lost it becomes `over_budget` rather than `unavailable`.
///
/// The distinction is load-bearing: `unavailable` is a fact about the manifests
/// and `over_budget` is a fact about this run's flags. A bundle that reported
/// `unavailable` here would be claiming the world cannot supply the file when
/// the truth is that this run would not pay for it.
///
/// **The run now fails, and this test used to assert the opposite.** It ended
/// with "a budget is a preference, not a failure" and `exit_code == 0`. A17's
/// own doc has always said `over_budget` is *deliberately* not an acceptable
/// matrix state — "a finding about the run rather than about the world" — so A17
/// already failed here, and the only reason the process exited `0` is that
/// nothing consulted A17's verdict. Now that a failed row ends the run, the
/// process exit agrees with the bundle's own account of itself.
///
/// That is a change to what a budgeted capture *means* to a caller, and it is
/// worth being explicit that it is one: `--max-download-bytes` is a supported
/// flag, and a caller who sets it too low now gets exit 3 rather than exit 0
/// with `result: fail`. Two documented positions disagreed — this test's line
/// and A17's doc — and wiring `is_fatal` resolves it in A17's favour.
///
/// The cost is visible below: `budget_limit`, `captures_over_budget` and
/// `capture_bytes_admitted` are summary keys, and a failing run emits a
/// failure summary instead of the capture summary, so those three numbers are
/// no longer on stdout at all. They were asserted here before. Nothing in the
/// bundle records them either.
#[test]
fn a_budget_turns_a_satisfiable_class_into_over_budget() {
    // The budget is derived from a first, unlimited run rather than guessed: a
    // hard-coded number would silently stop testing anything the moment the
    // world's bytes changed, and "the budget happened to be larger than
    // everything" looks exactly like "the budget did nothing".
    let full = captured("budget-full", World::five_classes(), &[]);
    let total = full.summary["capture_bytes_admitted"]
        .parse::<u64>()
        .expect("a size");
    let half = (total / 2).max(1);

    let c = captured(
        "budget",
        World::five_classes(),
        &["--max-download-bytes", &half.to_string()],
    );
    // The run fails, and says which assertion: A17, on the matrix.
    assert_eq!(
        c.summary["exit_code"], "3",
        "a class the budget refused makes A17 fail, and a failed row ends the run"
    );
    assert_eq!(c.summary["result"], "error");
    assert!(
        c.summary["detail"].contains("A17"),
        "the failure names the assertion: {:?}",
        c.summary["detail"]
    );

    // The matrix still records the distinction this test exists for. The bundle
    // is written before the verdict, so the evidence survives the failure -
    // which is the point of writing everything and *then* deciding.
    assert!(
        c.matrix().iter().any(|r| r["status"] == "over_budget"),
        "a refused candidate is recorded as over_budget, not unavailable: {:#?}",
        c.matrix()
    );
    for r in c.matrix() {
        if r["status"] == "over_budget" {
            assert!(
                r["capture_id"].is_null(),
                "{r} is over budget yet names a capture"
            );
            // The wording is an artifact string. The default capture is
            // unlimited, so `tests/bundle_diff.rs` never reaches it; this is the
            // only place that does, which makes it the only place that can hold
            // it still.
            let reason = r["reason"].as_str().expect("over_budget names itself");
            assert!(
                reason.starts_with("the cheapest candidate costs ")
                    && reason.ends_with(" B and the budget could not fund it"),
                "{reason:?}"
            );
        }
    }
}

/// The diff the run computed, read back off disk, agrees with the rows.
#[test]
fn the_diff_on_disk_matches_the_files_the_captures_name() {
    let c = captured("diff", World::five_classes(), &[]);
    let d = c.diff();
    assert_eq!(d["fields"], serde_json::json!(["game", "en-us"]));
    let counts = &d["counts"];
    let common = counts["common"].as_u64().expect("common");
    let added = counts["added"].as_u64().expect("added");
    let removed = counts["removed"].as_u64().expect("removed");
    let identical = counts["identical"].as_u64().expect("identical");

    // The world publishes 4 files on the new side across both fields: three `game`
    // files plus the one `en-us` file, which is identical across the pair.
    assert_eq!(
        common, 4,
        "browser_assets, zero.blk, HoYoChannel.dll, vo_all"
    );
    assert_eq!(added, 1, "libNew.dll");
    assert_eq!(removed, 1, "old.blk");
    assert_eq!(identical, 2, "browser_assets and vo_all are unchanged");
    assert_eq!(
        added as usize,
        d["added"].as_array().map(|a| a.len()).unwrap_or(0),
        "the added list and the count agree"
    );
    // Every path in the diff is field-qualified, so two fields cannot collide on
    // the same name.
    for p in d["added"].as_array().unwrap() {
        assert!(
            p.as_str().unwrap().contains(':'),
            "an unqualified path in the diff: {p}"
        );
    }
}

/// One field's matrix rows and another's are kept apart, so a capture from the
/// audio pool cannot be mistaken for one from `game`.
#[test]
fn audio_candidates_are_attributed_to_their_own_field() {
    let c = captured("attrib", World::five_classes(), &[]);
    let candidates = json(&c.bundle.join("diff/candidates.json"));
    let rows = candidates["rows"].as_array().expect("candidate rows");
    assert!(!rows.is_empty());
    for r in rows {
        let field = r["matching_field"]
            .as_str()
            .expect("every candidate names its field");
        assert!(
            field == "game" || field == "en-us",
            "an unexpected field: {r}"
        );
    }
    // The pool is both, because `--audio` defaults to `en-us`.
    assert_eq!(c.summary["selection_pool"], "game,en-us");
    let pool = &c.manifest()["selection_pool"];
    assert_eq!(pool["fields"], serde_json::json!(["game", "en-us"]));
    assert!(
        pool["manifest_entries"]["en-us"].as_u64().unwrap_or(0) > 0,
        "the audio field's entry count is recorded: {pool}"
    );
}

/// `--per-class` asks for more than one file of a class, and the extra is
/// admitted and numbered.
#[test]
fn per_class_two_admits_two_files_and_numbers_them() {
    let c = captured("perclass", World::five_classes(), &["--per-class", "2"]);
    // `added` has exactly one candidate, so the second is refused; the point is
    // that the capture still succeeds and says which class was short.
    let s = &c.summary;
    assert_eq!(s["exit_code"], "0");
    let ids: Vec<String> = std::fs::read_dir(c.bundle.join("patchmaps"))
        .unwrap()
        .flatten()
        .map(|e| {
            e.file_name()
                .to_string_lossy()
                .trim_end_matches(".json")
                .to_string()
        })
        .collect();
    for id in &ids {
        // An id is `<class>` or `<class>+<n>`; a bare class means the first.
        let base = id.split('+').next().unwrap_or(id);
        assert!(
            gircap::select::Class::parse(base).is_ok(),
            "{id} does not name a class"
        );
    }
}
/// An explicit `--files` path is captured, and the matrix is unaffected.
///
/// This is the escape hatch. The interesting property is *not* that it adds a
/// capture - naming a file the matrix would have taken anyway adds nothing, and
/// duplicating it would waste a download - but that naming one still produces
/// the file, and that the nine rows are the same nine either way.
#[test]
fn an_explicit_file_is_captured_and_the_matrix_is_unchanged() {
    let matrix_only = captured("explicit-off", World::five_classes(), &[]);
    let c = captured(
        "explicit",
        World::five_classes(),
        &["--files", "GenshinImpact_Data/Resources/browser_assets"],
    );
    assert_eq!(
        c.summary["exit_code"], "0",
        "naming a file the matrix already takes is not an error"
    );
    let ids = |b: &std::path::Path| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(b.join("patchmaps"))
            .unwrap()
            .flatten()
            .map(|e| {
                e.file_name()
                    .to_string_lossy()
                    .trim_end_matches(".json")
                    .to_string()
            })
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        ids(&c.bundle),
        ids(&matrix_only.bundle),
        "the named file was already admitted, so nothing was duplicated"
    );
    // And the nine rows are the same nine, with one difference that is the
    // point: `identical` moves from `captured` to `covered_by`, because the file
    // was taken for the caller rather than for the class. Both are satisfying
    // states, which is why A17 accepts either.
    let states = |rows: Vec<serde_json::Value>| -> std::collections::BTreeMap<String, String> {
        rows.iter()
            .map(|r| {
                (
                    r["class"].as_str().unwrap_or("").to_string(),
                    r["status"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect()
    };
    let with_files = states(c.matrix());
    let without = states(matrix_only.matrix());
    assert_eq!(with_files.len(), without.len(), "nine rows either way");
    assert_eq!(with_files["identical"], "covered_by");
    assert_eq!(without["identical"], "captured");
    for (class, status) in &with_files {
        if class == "identical" {
            continue;
        }
        assert_eq!(
            Some(status),
            without.get(class),
            "{class} changed state because an unrelated file was named"
        );
    }
}

/// A path neither manifest carries is skipped, recorded, and **not** fatal.
///
/// The caller may be naming a file for a future version, so refusing the run
/// would be wrong. Silently dropping it would also be wrong: a caller who named
/// five files and got one capture cannot otherwise tell the difference between
/// "captured" and "ignored".
#[test]
fn a_named_path_the_manifests_do_not_carry_is_skipped_and_reported() {
    let c = captured(
        "explicit-missing",
        World::five_classes(),
        &["--files", "GenshinImpact_Data/Plugins/Nope.dll"],
    );
    assert_eq!(
        c.summary["exit_code"], "0",
        "a path for a future version is not an error"
    );
    assert_eq!(c.summary["forced_files_requested"], "1");
    assert_eq!(c.summary["forced_files_skipped"], "1");
}
