//! `girpr`'s own repair machinery, on crafted inputs.
//!
//! The artifact suite (`tests/artifact_bridge.rs`) exists to prove one thing:
//! that `girpr`'s output is what another repair tool computes. Everything
//! else — which tier the repair picked, which direction it wrote in, whether
//! it trimmed eagerly, whether it refused a RAM reservation — is **`girpr`'s
//! own business**, and it is tested here, where the inputs are kilobytes and
//! the assertions are exact.
//!
//! # Why the real binary
//!
//! The decisions are made by the tool the user runs, and several of them are
//! only visible on the **data plane**: the live and the settled byte figures,
//! `local_manifest`, the write accounting. Those are the binary's `--output
//! json` records, so these cases run the binary rather than the library —
//! which also means the flags under test (`--repair-mode`, `--max-ram-bytes`,
//! `--scan`, `--max-write-bytes`, `--include`) are exercised as the user
//! spells them.
//!
//! The server is the mock in `tests/common`: a local HoYoPlay + Sophon on
//! `127.0.0.1`, no external network. The fixtures are described rather than
//! hardcoded (`FixtureSpec`), so a shape is a **description** and the cost of
//! a case is the size of the description.
//!
//! # What is here
//!
//! | decision | case |
//! |---|---|
//! | D1 skip | an intact file is planned, skipped and never touched |
//! | D2 kept / copied / fetched | the three chunk outcomes and the exact fetch set |
//! | D3 tier | staged, ordered-by-mode and ordered-by-shortage agree on the file |
//! | D4 direction | a uniform up-shift, a uniform down-shift, a mixed shift, a tie |
//! | D5 trim | eager, deferred, and "the file does not shrink" |
//! | D6 growth | the file grows and only the growth is written |
//! | D7 mode | `inplace` / `temp` / `discard` agree on the bytes |
//! | D8 resume | the in-place path re-verifies and continues |
//! | D9 a malformed manifest | a hole in the coverage, a body that is not a zstd frame, a digest no payload can match, a size that lies |
//!
//! and the four facts the artifact driver used to assert on every capture:
//! the live and the settled figures agree, `local_manifest` says `ok` or
//! `unavailable`, the scope contains the run, and the trace log's download
//! records agree with the server's request log.

#[path = "common/mod.rs"]
mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use common::{
    ChunkSpec, FileSpec, FixtureSpec, Lies, MockOpts, MockServer, build_fixture_spec, lay_out,
    temp_dir,
};
use girpr_tempdir::TempDir;

// ---------------------------------------------------------------------------
// the harness
// ---------------------------------------------------------------------------

/// One `girpr` invocation's result, with its NDJSON records indexed by name.
struct Run {
    code: i32,
    records: BTreeMap<String, serde_json::Value>,
    stdout: String,
    stderr: String,
    /// The trace log the run wrote.
    log: String,
}

impl Run {
    fn record(&self, event: &str) -> &serde_json::Value {
        self.records
            .get(event)
            .unwrap_or_else(|| panic!("no {event} record in:\n{}", self.stdout))
    }
    fn num(&self, event: &str, key: &str) -> u64 {
        self.record(event)[key]
            .as_u64()
            .unwrap_or_else(|| panic!("{event}.{key} is not a number in {:?}", self.record(event)))
    }
    fn word(&self, event: &str, key: &str) -> String {
        self.record(event)[key]
            .as_str()
            .unwrap_or_else(|| panic!("{event}.{key} is not a string in {:?}", self.record(event)))
            .to_string()
    }
}

/// A scratch install and the mock that serves it.
struct Rig {
    mock: MockServer,
    dir: TempDir,
    /// Somewhere for the trace logs, **outside** the install: the install's
    /// tree is what the scope assertions compare, and a log file appearing in
    /// it would look like the run added a file.
    logs: TempDir,
    /// How many runs this rig has made, so each gets its own trace log.
    runs: std::sync::atomic::AtomicU64,
}

impl Rig {
    /// A rig over the given fixture, with the mock's default answers.
    async fn start(spec: &FixtureSpec) -> Self {
        Self::start_with(spec, MockOpts::default()).await
    }

    async fn start_with(spec: &FixtureSpec, opts: MockOpts) -> Self {
        let mock = MockServer::start(build_fixture_spec(spec), opts).await;
        let dir = temp_dir("machinery");
        // The description's bytes on disk are the install's starting state.
        for (rel, bytes) in &spec.disk {
            common::write_game_file(&dir, rel, bytes);
        }
        Rig {
            mock,
            dir,
            logs: temp_dir("machinery_log"),
            runs: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// The scratch install's root.
    fn game(&self) -> PathBuf {
        self.dir.to_path_buf()
    }

    /// Write a file into the scratch install.
    fn write(&self, rel: &str, bytes: &[u8]) {
        common::write_game_file(&self.dir, rel, bytes);
    }

    /// Read a file out of the scratch install.
    fn read(&self, rel: &str) -> Vec<u8> {
        common::read_game_file(&self.dir, rel).unwrap_or_else(|| panic!("{rel} is not there"))
    }

    /// Say that the install is at the local build, so the local manifest is
    /// fetched and the chunk reuse map is not empty.
    fn at_local_version(&self) {
        std::fs::write(
            self.dir.join("config.ini"),
            "[General]\ngame_version=9.9.8-test\n",
        )
        .expect("write config.ini");
    }

    /// Run the real binary against the rig's install.
    async fn run(&self, args: &[&str]) -> Run {
        let n = self.runs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let log = self.logs.join(format!("girpr-{n}.log"));
        let mut all: Vec<String> = vec![
            "--game-path".into(),
            self.game().display().to_string(),
            "--biz".into(),
            "hk4e_global".into(),
            "--hyp-base".into(),
            self.mock.hyp_base.clone(),
            "--sophon-base".into(),
            self.mock.sophon_base.clone(),
            "--io-threads".into(),
            "1".into(),
            "--output".into(),
            "json".into(),
            "--log-file".into(),
            log.display().to_string(),
            // The log file's own level, which is a knob of its own: the
            // chunk-level records this test reads are `trace`, and the file
            // defaults to `debug`. `--log-level` would only change the
            // console.
            "--log-file-level".into(),
            "trace".into(),
        ];
        all.extend(args.iter().map(|s| s.to_string()));
        let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_girpr"))
            .args(&all)
            .output()
            .await
            .expect("spawn girpr");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        let mut records = BTreeMap::new();
        for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
            let v: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("--output json line is not JSON ({e}): {line}"));
            // Last one wins: `PROGRESS` ticks repeatedly and `SUMMARY` is
            // terminal, so the newest record of a name describes the run.
            if let Some(event) = v["event"].as_str() {
                records.insert(event.to_string(), v);
            }
        }
        Run {
            code: out.status.code().expect("process must exit normally"),
            records,
            stdout,
            stderr,
            log: std::fs::read_to_string(&log).unwrap_or_default(),
        }
    }
}

/// The chunk ids a trace log records as **downloaded**.
///
/// Only the two download records are matched, never the bare id: a reused
/// chunk is logged too (`reuse slice md5 ok -> copy chunk=<id>`), so a
/// substring search for an id finds it in the "reused it" line and
/// false-positives.
fn downloaded_chunks(log: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in log.lines() {
        for marker in ["chunk downloaded ", "no local reuse candidate -> download "] {
            let Some(at) = line.find(marker) else {
                continue;
            };
            let rest = &line[at + marker.len()..];
            let Some(id) = rest.strip_prefix("chunk=") else {
                continue;
            };
            let id = id.split_whitespace().next().unwrap_or_default();
            if !id.is_empty() {
                out.insert(id.to_string());
            }
        }
    }
    out
}

/// Every file under the install as `rel -> (bytes, md5)`, `/`-separated.
///
/// MD5 rather than length alone, because "untouched" has to mean
/// *byte-identical*: a run that rewrote a file to the same length would
/// otherwise pass.
fn snapshot(game: &Path) -> BTreeMap<String, (u64, String)> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, (u64, String)>) {
        for entry in
            std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        {
            let entry = entry.expect("read dir entry");
            let path = entry.path();
            if entry.file_type().expect("file type").is_dir() {
                walk(&path, base, out);
                continue;
            }
            let rel = path
                .strip_prefix(base)
                .expect("under the base")
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = std::fs::read(&path).expect("read file");
            out.insert(
                rel,
                (bytes.len() as u64, format!("{:x}", md5::compute(&bytes))),
            );
        }
    }
    let mut out = BTreeMap::new();
    walk(game, game, &mut out);
    out
}

// ---------------------------------------------------------------------------
// the shapes
// ---------------------------------------------------------------------------

/// One chunk: `size` bytes of `fill`, at `offset`.
fn chunk(id: &str, fill: u8, size: usize, offset: i64) -> ChunkSpec {
    ChunkSpec::new(id, vec![fill; size], offset)
}

/// `n` chunks of `size` bytes, the `i`-th holding the byte `base + i`.
fn run_of(prefix: &str, n: usize, size: usize, base: u8) -> Vec<ChunkSpec> {
    (0..n)
        .map(|i| {
            chunk(
                &format!("{prefix}{i}"),
                base + i as u8,
                size,
                (i * size) as i64,
            )
        })
        .collect()
}

/// A uniform **downward** shift: every source sits **after** its
/// destination, so the ascending order keeps all of them and the descending
/// one keeps none.
///
/// ```text
///   old:  [0,64)=A   [64,128)=B
///   new:  [0,64)=B   [64,128)=C
/// ```
fn shift_down(size: usize) -> FixtureSpec {
    let a = 0x40u8;
    let old = vec![
        chunk("old_a", a, size, 0),
        chunk("old_b", a + 1, size, size as i64),
    ];
    let new = vec![
        chunk("new_b", a + 1, size, 0),
        chunk("new_c", a + 2, size, size as i64),
    ];
    FixtureSpec::new(vec![FileSpec::new("game/shift.dat", new)])
        .with_old(vec![FileSpec::new("game/shift.dat", old.clone())])
        .with_disk("game/shift.dat", lay_out(&old))
}

/// A uniform **upward** shift: every source sits **before** its
/// destination, so the descending order keeps all of them and the ascending
/// one keeps none.
///
/// ```text
///   old:  [0,64)=A   [64,128)=B
///   new:  [0,64)=C   [64,128)=A
/// ```
fn shift_up(size: usize) -> FixtureSpec {
    let a = 0x50u8;
    let old = vec![
        chunk("old_a", a, size, 0),
        chunk("old_b", a + 1, size, size as i64),
    ];
    let new = vec![
        chunk("new_c", a + 2, size, 0),
        chunk("new_a", a, size, size as i64),
    ];
    FixtureSpec::new(vec![FileSpec::new("game/shift.dat", new)])
        .with_old(vec![FileSpec::new("game/shift.dat", old.clone())])
        .with_disk("game/shift.dat", lay_out(&old))
}

/// A mixed shift: `n_down` chunks move **down** and `n_up` move **up**, so
/// neither write order keeps every source and the ordered tier has to fetch
/// the ones it cannot salvage.
///
/// The new file is the old one rotated by `n_up`: the first `n_down` chunks
/// come from *later* in the old file (their source is after their
/// destination) and the last `n_up` from *earlier* (their source is before).
///
/// With `n_down = 20` and `n_up = 3` the ratio is far more lopsided than a
/// real game file's, and the rule's answer is unambiguous.
fn mixed_shift(n_down: usize, n_up: usize, size: usize) -> FixtureSpec {
    let total = n_down + n_up;
    let old = run_of("old_", total, size, 0x60);
    let payloads: Vec<Vec<u8>> = old.iter().map(|c| c.payload.clone()).collect();
    let new: Vec<ChunkSpec> = (0..total)
        .map(|j| {
            let src = (n_up + j) % total;
            ChunkSpec::new(
                &format!("new_{j}"),
                payloads[src].clone(),
                (j * size) as i64,
            )
        })
        .collect();
    FixtureSpec::new(vec![FileSpec::new("game/mixed.dat", new)])
        .with_old(vec![FileSpec::new("game/mixed.dat", old.clone())])
        .with_disk("game/mixed.dat", lay_out(&old))
}

// ---------------------------------------------------------------------------
// D1 — the skip branch
// ---------------------------------------------------------------------------

/// An intact file is **planned**, **skipped**, and **not touched**.
///
/// The file is the target's own bytes, so the whole-file MD5 matches and the
/// repair has nothing to do. The load-bearing part is the negative: nothing
/// was fetched, and the bytes on disk are the bytes that were there before.
#[tokio::test]
async fn an_intact_file_is_planned_skipped_and_never_touched() {
    let spec = FixtureSpec::new(vec![FileSpec::new(
        "game/intact.dat",
        vec![chunk("c0", 0x41, 64, 0), chunk("c1", 0x42, 64, 64)],
    )]);
    let rig = Rig::start(&spec).await;
    let want = lay_out(&spec.new[0].chunks);
    rig.write("game/intact.dat", &want);

    let run = rig.run(&["--scan"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.num("summary", "total"), 1, "it is planned");
    assert_eq!(run.num("summary", "skipped"), 1);
    assert_eq!(run.num("summary", "repaired"), 0);
    assert_eq!(run.num("summary", "download_bytes"), 0);
    assert!(
        rig.mock.chunk_hits().is_empty(),
        "nothing may be downloaded: {:?}",
        rig.mock.chunk_hits()
    );
    // The file is byte-identical: not rewritten, not touched.
    assert_eq!(rig.read("game/intact.dat"), want);
    // And the scan agrees: no chunk is damaged.
    assert_eq!(run.num("summary", "damaged_chunks"), 0);
}

// ---------------------------------------------------------------------------
// D2 — kept, copied, fetched
// ---------------------------------------------------------------------------

/// The three chunk outcomes, and the exact set of chunk ids requested.
///
/// One file, three chunks:
/// - **kept** — already correct where it belongs, so it is not written;
/// - **copied** — its bytes live elsewhere in the file, so they are moved;
/// - **fetched** — no usable local source, so it comes off the network.
///
/// The fetch set is the assertion: exactly one chunk id is asked for.
#[tokio::test]
async fn the_chunk_outcomes_are_kept_copied_and_fetched() {
    // new:  [0,64)=K   [64,128)=M   [128,192)=F
    // old:  [0,64)=X   [64,128)=K   [192,256)=M
    //
    // `K` is already correct where it belongs (kept), `M`'s source is at 192
    // (copied), and `F` has no source at all (fetched).
    let size = 64usize;
    let old = vec![
        chunk("old_x", 0x71, size, 0),
        chunk("old_k", 0x72, size, size as i64),
        chunk("old_m", 0x73, size, 3 * size as i64),
    ];
    let new = vec![
        chunk("new_k", 0x72, size, 0),
        chunk("new_m", 0x73, size, size as i64),
        chunk("new_f", 0x74, size, 2 * size as i64),
    ];
    let spec = FixtureSpec::new(vec![FileSpec::new("game/three.dat", new)])
        .with_old(vec![FileSpec::new("game/three.dat", old.clone())]);
    let rig = Rig::start(&spec).await;
    rig.at_local_version();
    // The disk holds the old file, with the kept chunk already in place.
    rig.write("game/three.dat", &lay_out(&old));

    let run = rig.run(&["--scan", "--repair-mode", "inplace"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.num("summary", "repaired"), 1);
    assert_eq!(
        rig.mock.chunk_hits(),
        vec!["new_f".to_string()],
        "exactly the chunk with no local source is fetched"
    );
    // The file is the target's bytes.
    assert_eq!(rig.read("game/three.dat"), lay_out(&spec.new[0].chunks));
}

/// With no local manifest the reuse map is empty, so **every** chunk is
/// fetched — the other side of the same boundary.
///
/// The two cases together are the reuse claim: the fetch set is exactly the
/// set of chunks that had no usable local source.
#[tokio::test]
async fn without_a_local_manifest_every_chunk_is_fetched() {
    let spec = FixtureSpec::new(vec![FileSpec::new(
        "game/three.dat",
        vec![
            chunk("new_k", 0x72, 64, 0),
            chunk("new_m", 0x73, 64, 64),
            chunk("new_f", 0x74, 64, 128),
        ],
    )]);
    let rig = Rig::start(&spec).await;
    rig.write("game/three.dat", &[0u8; 192]);

    let run = rig.run(&["--scan"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(
        rig.mock.chunk_hits(),
        vec![
            "new_k".to_string(),
            "new_m".to_string(),
            "new_f".to_string()
        ],
        "with no reuse map every chunk downloads"
    );
    assert_eq!(rig.read("game/three.dat"), lay_out(&spec.new[0].chunks));
}

// ---------------------------------------------------------------------------
// D3 — the tier
// ---------------------------------------------------------------------------

/// The three tiers produce the **same file**.
///
/// - **staged** — the bytes that move are held in RAM, so every source
///   survives;
/// - **ordered by mode** — `--max-ram-bytes 0` stages nothing at all;
/// - **ordered by shortage** — a budget too small for the payload, so the
///   reservation is *refused*.
///
/// The tier is a choice of *how* to get there, so the file must not depend
/// on it.
#[tokio::test]
async fn the_three_tiers_produce_the_same_file() {
    // A uniform downward shift: the ascending order keeps every source, so
    // the ordered tier fetches the same single chunk the staged tier does.
    let spec = shift_down(64);
    let want = lay_out(&spec.new[0].chunks);

    // Staged: the default, with a budget the payload fits.
    let staged = Rig::start(&spec).await;
    staged.at_local_version();
    let run = staged.run(&["--max-ram-bytes", "1048576"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(staged.read("game/shift.dat"), want);
    assert_eq!(
        run.num("summary", "ram_bytes"),
        64,
        "the one chunk that has to move was staged"
    );

    // Ordered by mode: stage nothing at all.
    let by_mode = Rig::start(&spec).await;
    by_mode.at_local_version();
    let run = by_mode.run(&["--max-ram-bytes", "0"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(by_mode.read("game/shift.dat"), want);
    assert_eq!(run.num("summary", "ram_bytes"), 0, "nothing was staged");

    // Ordered by shortage: the reservation is refused.
    let by_shortage = Rig::start(&spec).await;
    by_shortage.at_local_version();
    let run = by_shortage.run(&["--max-ram-bytes", "16"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(by_shortage.read("game/shift.dat"), want);
    assert_eq!(run.num("summary", "ram_bytes"), 0, "nothing was staged");
    assert!(
        run.log
            .contains("the moving bytes do not fit the RAM budget"),
        "the refusal has to be said, not merely acted on\n{}",
        run.log
    );
}

/// A refused reservation is a **shortage**, and `--max-ram-bytes 0` is a
/// **mode**. They end up in the same tier and say different things.
#[tokio::test]
async fn a_refused_reservation_is_not_the_same_thing_as_staging_nothing() {
    let spec = shift_down(64);

    let by_mode = Rig::start(&spec).await;
    by_mode.at_local_version();
    let run = by_mode.run(&["--max-ram-bytes", "0"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(
        !run.log
            .contains("the moving bytes do not fit the RAM budget"),
        "the mode is not a shortage\n{}",
        run.log
    );

    let by_shortage = Rig::start(&spec).await;
    by_shortage.at_local_version();
    let run = by_shortage.run(&["--max-ram-bytes", "16"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.log
            .contains("the moving bytes do not fit the RAM budget"),
        "the shortage has to be said\n{}",
        run.log
    );
}

// ---------------------------------------------------------------------------
// D4 — the direction
// ---------------------------------------------------------------------------

/// A uniform **upward** shift: the sources sit **before** their
/// destinations, so the descending write order keeps every one of them and
/// the ascending order destroys them.
///
/// The observable is the fetch set: the descending order fetches only the
/// chunk that has no source at all, while the ascending one has to fetch the
/// moved chunk as well.
#[tokio::test]
async fn a_uniform_upward_shift_keeps_every_source_in_the_descending_order() {
    let spec = shift_up(64);
    let rig = Rig::start(&spec).await;
    rig.at_local_version();
    let run = rig.run(&["--max-ram-bytes", "0"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.log.contains("ascending=false"),
        "the content moved later, so the descending order is the one that keeps it\n{}",
        run.log
    );
    assert_eq!(
        rig.mock.chunk_hits(),
        vec!["new_c".to_string()],
        "the descending order keeps the moved chunk's source alive"
    );
}

/// A uniform **downward** shift: the sources sit **after** their
/// destinations, so the ascending write order keeps every one of them.
#[tokio::test]
async fn a_uniform_downward_shift_keeps_every_source_in_the_ascending_order() {
    let spec = shift_down(64);
    let rig = Rig::start(&spec).await;
    rig.at_local_version();
    let run = rig.run(&["--max-ram-bytes", "0"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.log.contains("ascending=true"),
        "the content moved earlier, so the ascending order is the one that keeps it\n{}",
        run.log
    );
    assert_eq!(
        rig.mock.chunk_hits(),
        vec!["new_c".to_string()],
        "the ascending order keeps the moved chunk's source alive"
    );
}

/// A mixed shift: the ordered tier keeps the **larger half** and fetches the
/// rest.
///
/// The fixture is deliberately lopsided — 20 chunks move down and 3 move up
/// — so the rule's answer is unambiguous and the number is not a matter of
/// taste.
#[tokio::test]
async fn a_mixed_shift_keeps_the_larger_half() {
    let n_down = 20usize;
    let n_up = 3usize;
    let size = 64usize;
    let spec = mixed_shift(n_down, n_up, size);
    let rig = Rig::start(&spec).await;
    rig.at_local_version();

    let run = rig.run(&["--max-ram-bytes", "0"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.log
            .contains(&format!("salvaged={n_down} of={}", n_down + n_up)),
        "the ascending order keeps the {n_down} that moved down, and not the \
         {n_up} that moved up\n{}",
        run.log
    );
    let asked = rig.mock.chunk_hits();
    assert_eq!(
        asked.len(),
        n_up,
        "the ordered tier fetches exactly the {n_up} chunks whose source the \
         write order destroys, and keeps the other {n_down}: {asked:?}"
    );
    // The file is still the target's bytes.
    assert_eq!(rig.read("game/mixed.dat"), lay_out(&spec.new[0].chunks));
}

/// A tie goes to the **ascending** order.
///
/// Two chunks, each the other's source: neither direction keeps more than
/// one, so the rule's tie-break decides, and it is ascending.
#[tokio::test]
async fn a_tie_goes_to_the_ascending_order() {
    let size = 64usize;
    let old = vec![
        chunk("old_a", 0x41, size, 0),
        chunk("old_b", 0x42, size, size as i64),
    ];
    let new = vec![
        chunk("new_b", 0x42, size, 0),
        chunk("new_a", 0x41, size, size as i64),
    ];
    let spec = FixtureSpec::new(vec![FileSpec::new("game/tie.dat", new)])
        .with_old(vec![FileSpec::new("game/tie.dat", old.clone())])
        .with_disk("game/tie.dat", lay_out(&old));
    let rig = Rig::start(&spec).await;
    rig.at_local_version();

    let run = rig.run(&["--max-ram-bytes", "0"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.log.contains("ascending=true"),
        "a tie goes to the ascending order\n{}",
        run.log
    );
    assert_eq!(rig.read("game/tie.dat"), lay_out(&spec.new[0].chunks));
}

// ---------------------------------------------------------------------------
// D5 — the trim
// ---------------------------------------------------------------------------

/// The file on disk is **longer** than the target and no source reaches past
/// the target's end, so the dead tail is released **before** the first
/// write.
///
/// The trim's outcome is not in the output contract — the file ends at the
/// target size either way — so the log is where the outcome is visible, and
/// the log is the documented place it is written.
#[tokio::test]
async fn the_dead_tail_is_released_before_the_first_write() {
    let size = 64usize;
    let old = vec![
        chunk("old_a", 0x41, size, 0),
        chunk("old_b", 0x42, size, size as i64),
    ];
    let new = vec![
        chunk("new_a", 0x41, size, 0),
        chunk("new_b", 0x42, size, size as i64),
    ];
    // The disk file carries a third, dead chunk the manifest never mentions.
    let mut on_disk = lay_out(&old);
    on_disk.extend_from_slice(&vec![0x5Au8; size]);
    let spec = FixtureSpec::new(vec![FileSpec::new("game/shrink.dat", new)])
        .with_old(vec![FileSpec::new("game/shrink.dat", old.clone())])
        .with_disk("game/shrink.dat", on_disk.clone());

    let rig = Rig::start(&spec).await;
    rig.at_local_version();
    let run = rig.run(&["--max-ram-bytes", "0"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.log.contains("trimming the dead tail"),
        "the dead tail is released before the first write\n{}",
        run.log
    );
    assert!(
        !run.log
            .contains("a reuse candidate reaches past the target's end"),
        "and it is not deferred as well\n{}",
        run.log
    );
    // The file ends at the target's size, with the tail gone.
    assert_eq!(rig.read("game/shrink.dat"), lay_out(&spec.new[0].chunks));
}

/// A source reaches past the target's end, so the trim **waits** for the
/// end: the tail is not dead weight, and the file grows during the repair.
#[tokio::test]
async fn a_source_past_the_end_defers_the_trim() {
    let size = 64usize;
    // The old file is three chunks; the new one is the last two of them,
    // moved up. The source of `new_c` sits at 128, past the target's end.
    let old = vec![
        chunk("old_a", 0x41, size, 0),
        chunk("old_b", 0x42, size, size as i64),
        chunk("old_c", 0x43, size, 2 * size as i64),
    ];
    let new = vec![
        chunk("new_b", 0x42, size, 0),
        chunk("new_c", 0x43, size, size as i64),
    ];
    let spec = FixtureSpec::new(vec![FileSpec::new("game/shrink.dat", new)])
        .with_old(vec![FileSpec::new("game/shrink.dat", old.clone())])
        .with_disk("game/shrink.dat", lay_out(&old));

    let rig = Rig::start(&spec).await;
    rig.at_local_version();
    let run = rig.run(&["--max-ram-bytes", "0"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.log
            .contains("a reuse candidate reaches past the target's end"),
        "the trim waits for the end\n{}",
        run.log
    );
    assert!(
        !run.log.contains("trimming the dead tail"),
        "and it is not released eagerly as well\n{}",
        run.log
    );
    assert_eq!(rig.read("game/shrink.dat"), lay_out(&spec.new[0].chunks));
}

/// A file that does not shrink is **not trimmed** at all.
#[tokio::test]
async fn a_file_that_does_not_shrink_is_not_trimmed() {
    let size = 64usize;
    let old = vec![
        chunk("old_a", 0x41, size, 0),
        chunk("old_b", 0x42, size, size as i64),
    ];
    let new = vec![
        chunk("new_a", 0x41, size, 0),
        chunk("new_b", 0x43, size, size as i64),
    ];
    let spec = FixtureSpec::new(vec![FileSpec::new("game/same.dat", new)])
        .with_old(vec![FileSpec::new("game/same.dat", old.clone())])
        .with_disk("game/same.dat", lay_out(&old));

    let rig = Rig::start(&spec).await;
    rig.at_local_version();
    let run = rig.run(&["--max-ram-bytes", "0"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(
        !run.log.contains("trimming the dead tail")
            && !run
                .log
                .contains("a reuse candidate reaches past the target's end"),
        "a file that does not shrink has no dead tail\n{}",
        run.log
    );
}

// ---------------------------------------------------------------------------
// D6 — the growth
// ---------------------------------------------------------------------------

/// The file on disk is **shorter** than the target: the repair writes only
/// the growth.
///
/// The first chunk is already correct where it belongs, so it is kept and
/// not written; the second has to be fetched and written. The write figure
/// is the growth, not the file's size.
#[tokio::test]
async fn a_file_that_grows_writes_only_the_growth() {
    let size = 64usize;
    let old = vec![chunk("old_a", 0x41, size, 0)];
    let new = vec![
        chunk("new_a", 0x41, size, 0),
        chunk("new_b", 0x42, size, size as i64),
    ];
    let spec = FixtureSpec::new(vec![FileSpec::new("game/grow.dat", new)])
        .with_old(vec![FileSpec::new("game/grow.dat", old.clone())])
        .with_disk("game/grow.dat", lay_out(&old));

    let rig = Rig::start(&spec).await;
    rig.at_local_version();
    let run = rig
        .run(&["--max-write-bytes", "1048576", "--repair-mode", "inplace"])
        .await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(
        run.num("summary", "write_bytes"),
        size as u64,
        "only the growth is written"
    );
    assert_eq!(
        run.num("summary", "kept_bytes"),
        size as u64,
        "the chunk that was already right is kept"
    );
    assert_eq!(rig.read("game/grow.dat"), lay_out(&spec.new[0].chunks));
}

// ---------------------------------------------------------------------------
// D7 — the mode
// ---------------------------------------------------------------------------

/// The three modes produce the **same bytes**.
///
/// `inplace` writes where the file lies, `temp` writes beside it and renames
/// over it, and `discard` deletes the target and writes it from scratch. The
/// mode is a choice of *how*, not of *what*.
#[tokio::test]
async fn the_three_modes_produce_the_same_bytes() {
    let size = 64usize;
    let old = vec![
        chunk("old_a", 0x41, size, 0),
        chunk("old_b", 0x42, size, size as i64),
    ];
    let new = vec![
        chunk("new_a", 0x43, size, 0),
        chunk("new_b", 0x44, size, size as i64),
    ];
    let spec = FixtureSpec::new(vec![FileSpec::new("game/mode.dat", new)])
        .with_old(vec![FileSpec::new("game/mode.dat", old.clone())])
        .with_disk("game/mode.dat", lay_out(&old));
    let want = lay_out(&spec.new[0].chunks);

    for mode in ["inplace", "temp", "discard"] {
        let rig = Rig::start(&spec).await;
        rig.at_local_version();
        let run = rig.run(&["--repair-mode", mode]).await;
        assert_eq!(run.code, 0, "{mode}: {}\n{}", run.stdout, run.stderr);
        assert_eq!(
            rig.read("game/mode.dat"),
            want,
            "{mode}: the mode is a choice of how, not of what"
        );
    }
}

/// The in-place path leaves **no** temp file behind; the temp path leaves
/// none either, because the temp file is renamed over the target.
#[tokio::test]
async fn no_mode_leaves_a_temp_file_behind() {
    let size = 64usize;
    let old = vec![chunk("old_a", 0x41, size, 0)];
    let new = vec![chunk("new_b", 0x42, size, 0)];
    let spec = FixtureSpec::new(vec![FileSpec::new("game/mode.dat", new)])
        .with_old(vec![FileSpec::new("game/mode.dat", old.clone())])
        .with_disk("game/mode.dat", lay_out(&old));

    for mode in ["inplace", "temp", "discard"] {
        let rig = Rig::start(&spec).await;
        rig.at_local_version();
        let run = rig.run(&["--repair-mode", mode]).await;
        assert_eq!(run.code, 0, "{mode}: {}\n{}", run.stdout, run.stderr);
        let leftovers: Vec<String> = std::fs::read_dir(rig.game().join("game"))
            .expect("game/")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with("_tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{mode}: {leftovers:?}");
    }
}

// ---------------------------------------------------------------------------
// D8 — the resume
// ---------------------------------------------------------------------------

/// The in-place path **re-verifies and continues**: a second run over a
/// damaged file rewrites only the damaged chunk.
///
/// The damage is one chunk's bytes, flipped in the file the first run
/// produced. The second run finds it, rewrites it, and leaves every other
/// byte alone.
#[tokio::test]
async fn the_in_place_path_re_verifies_and_continues() {
    let size = 64usize;
    let old = vec![
        chunk("old_a", 0x41, size, 0),
        chunk("old_b", 0x42, size, size as i64),
    ];
    let new = vec![
        chunk("new_a", 0x43, size, 0),
        chunk("new_b", 0x44, size, size as i64),
    ];
    let spec = FixtureSpec::new(vec![FileSpec::new("game/resume.dat", new)])
        .with_old(vec![FileSpec::new("game/resume.dat", old.clone())])
        .with_disk("game/resume.dat", lay_out(&old));
    let want = lay_out(&spec.new[0].chunks);

    let rig = Rig::start(&spec).await;
    rig.at_local_version();
    let first = rig
        .run(&[
            "--scan",
            "--max-write-bytes",
            "1048576",
            "--repair-mode",
            "inplace",
        ])
        .await;
    assert_eq!(first.code, 0, "{}\n{}", first.stdout, first.stderr);
    assert_eq!(rig.read("game/resume.dat"), want);

    // Damage one chunk's bytes.
    let mut damaged = rig.read("game/resume.dat");
    damaged[size] ^= 0xff;
    rig.write("game/resume.dat", &damaged);

    let second = rig
        .run(&[
            "--scan",
            "--max-write-bytes",
            "1048576",
            "--repair-mode",
            "inplace",
        ])
        .await;
    assert_eq!(second.code, 0, "{}\n{}", second.stdout, second.stderr);
    assert_eq!(
        second.num("summary", "damaged_chunks"),
        1,
        "the second run finds exactly the damaged chunk"
    );
    assert_eq!(
        second.num("summary", "write_bytes"),
        size as u64,
        "and rewrites only it"
    );
    assert_eq!(rig.read("game/resume.dat"), want, "the file is restored");
}

// ---------------------------------------------------------------------------
// D9 — a malformed manifest
// ---------------------------------------------------------------------------

/// A hole in the chunk coverage is **zero-filled**, and the file still
/// verifies.
///
/// The manifest claims two chunks and leaves a gap between them, so the gap
/// is a region no chunk writes: without the fill it would keep the old
/// file's bytes, and the whole-file MD5 would reject the result.
#[tokio::test]
async fn a_hole_in_the_coverage_is_zero_filled_and_the_file_verifies() {
    let size = 64usize;
    // [0,64) and [128,192): the gap [64,128) belongs to no chunk.
    let new = vec![
        chunk("new_a", 0x41, size, 0),
        chunk("new_b", 0x42, size, 2 * size as i64),
    ];
    let spec = FixtureSpec::new(vec![FileSpec::new("game/hole.dat", new)]);
    let want = lay_out(&spec.new[0].chunks);
    // The disk file has stale bytes in the hole.
    let mut on_disk = vec![0x5Au8; 3 * size];
    on_disk[..size].copy_from_slice(&want[..size]);
    on_disk[2 * size..].copy_from_slice(&want[2 * size..]);

    let rig = Rig::start(&spec).await;
    rig.write("game/hole.dat", &on_disk);

    let run = rig.run(&["--scan"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.num("summary", "repaired"), 1);
    assert_eq!(
        rig.read("game/hole.dat"),
        want,
        "the hole is zero-filled, so the file's MD5 matches"
    );
}

/// A chunk served with a body that is **not a zstd frame** fails the file,
/// and the run says so.
///
/// The decode is the gate: a body that is not a frame has to be refused
/// rather than landed in the install.
#[tokio::test]
async fn a_chunk_that_is_not_a_zstd_frame_fails_the_file() {
    let spec = FixtureSpec::new(vec![FileSpec::new(
        "game/bad.dat",
        vec![chunk("bad", 0x41, 64, 0)],
    )])
    .with_lies(Lies {
        not_zstd: vec!["bad".to_string()],
        ..Default::default()
    });
    let rig = Rig::start(&spec).await;

    let run = rig.run(&["--fail-fast"]).await;
    assert_eq!(run.code, 3, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.num("summary", "failed"), 1);
    assert_eq!(run.num("summary", "repaired"), 0);
}

/// A chunk whose declared digest **no payload can match** is refused.
///
/// The manifest's `uncompressed_md5` is the digest gate, and a digest that
/// nothing can produce has to fail the chunk rather than land the bytes.
#[tokio::test]
async fn a_chunk_whose_declared_digest_no_payload_can_match_is_refused() {
    let spec = FixtureSpec::new(vec![FileSpec::new(
        "game/bad.dat",
        vec![chunk("bad", 0x41, 64, 0)],
    )])
    .with_lies(Lies {
        wrong_chunk_md5: vec!["bad".to_string()],
        ..Default::default()
    });
    let rig = Rig::start(&spec).await;

    let run = rig.run(&["--fail-fast"]).await;
    assert_eq!(run.code, 3, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.num("summary", "failed"), 1);
}

/// A manifest whose `compressed_size` **lies** is still repaired: the size is
/// a hint, and the bytes that cross the wire are the ones that count.
#[tokio::test]
async fn a_manifest_that_lies_about_the_size_is_still_repaired() {
    let size = 64usize;
    let new = vec![chunk("new_a", 0x41, size, 0)];
    let spec = FixtureSpec::new(vec![FileSpec::new("game/sized.dat", new)]).with_lies(Lies {
        wrong_chunk_size: vec!["new_a".to_string()],
        ..Default::default()
    });
    let rig = Rig::start(&spec).await;

    let run = rig.run(&["--max-download-bytes", "1048576"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.num("summary", "repaired"), 1);
    assert_eq!(
        rig.read("game/sized.dat"),
        lay_out(&spec.new[0].chunks),
        "the file is the target's bytes"
    );
    // The declared size is a hint; the reported figure is the real one.
    assert!(
        run.num("summary", "download_bytes") < 100,
        "the reported bytes are the ones that crossed the wire, not the \
         declared size: {}",
        run.num("summary", "download_bytes")
    );
}

// ---------------------------------------------------------------------------
// the facts the artifact driver used to assert on every capture
// ---------------------------------------------------------------------------

/// The **pair**: the live figure (what the run spent) and the settled one
/// (what it achieved) are the same number in a clean run.
///
/// Nothing was fetched twice and nothing was written twice, so the run's
/// two figures have to agree — in both directions.
#[tokio::test]
async fn the_live_and_the_settled_figures_agree_on_a_clean_run() {
    let size = 64usize;
    let old = vec![
        chunk("old_a", 0x41, size, 0),
        chunk("old_b", 0x42, size, size as i64),
    ];
    let new = vec![
        chunk("new_a", 0x43, size, 0),
        chunk("new_b", 0x44, size, size as i64),
    ];
    let spec = FixtureSpec::new(vec![FileSpec::new("game/pair.dat", new)])
        .with_old(vec![FileSpec::new("game/pair.dat", old.clone())])
        .with_disk("game/pair.dat", lay_out(&old));

    let rig = Rig::start(&spec).await;
    rig.at_local_version();
    let run = rig
        .run(&[
            "--max-download-bytes",
            "1048576",
            "--max-write-bytes",
            "1048576",
        ])
        .await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(
        run.num("summary", "download_bytes_spent"),
        run.num("summary", "download_bytes"),
        "a clean run fetches nothing twice"
    );
    assert_eq!(
        run.num("summary", "write_bytes_spent"),
        run.num("summary", "write_bytes"),
        "a clean run writes nothing twice"
    );
}

/// `local_manifest` says **`ok`** when the local build's manifest loaded, and
/// **`unavailable`** when it did not.
///
/// This is the one degradation that is otherwise invisible: with no local
/// build the reuse map is empty, every chunk downloads, and the run still
/// exits `0`. The key is what tells the two apart.
#[tokio::test]
async fn the_local_manifest_key_says_ok_and_unavailable() {
    let size = 64usize;
    let new = vec![chunk("c0", 0x41, size, 0)];
    let old = vec![chunk("old_c0", 0x41, size, 0)];

    // The local build is served: the key says `ok`.
    let with_local = FixtureSpec::new(vec![FileSpec::new("game/one.dat", new.clone())])
        .with_old(vec![FileSpec::new("game/one.dat", old.clone())]);
    let ok = Rig::start(&with_local).await;
    ok.at_local_version();
    let run = ok.run(&[]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.word("summary", "local_manifest"), "ok");

    // The local build is **not** served: the key says `unavailable`, and the
    // run is still green — it just downloaded everything.
    let without_local = FixtureSpec::new(vec![FileSpec::new("game/one.dat", new)]);
    let unavailable = Rig::start(&without_local).await;
    unavailable.at_local_version();
    let run = unavailable.run(&[]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.word("summary", "local_manifest"), "unavailable");
}

/// The scope **contains** the run: a path outside `--include` is not added,
/// not removed and not changed.
///
/// The manifest holds three files and the scope names one of them, so the
/// other two — and a file on disk that the manifest does not mention — must
/// come out of the run untouched.
#[tokio::test]
async fn the_scope_contains_the_run() {
    let size = 64usize;
    let spec = FixtureSpec::new(vec![
        FileSpec::new("game/one.dat", vec![chunk("one", 0x41, size, 0)]),
        FileSpec::new("game/two.dat", vec![chunk("two", 0x42, size, 0)]),
        FileSpec::new("game/three.dat", vec![chunk("three", 0x43, size, 0)]),
    ]);
    let rig = Rig::start(&spec).await;
    // Everything on disk is stale, and there is a file the manifest does not
    // mention at all.
    for rel in ["game/one.dat", "game/two.dat", "game/three.dat"] {
        rig.write(rel, &vec![0x5Au8; size]);
    }
    rig.write("game/orphan.dat", &vec![0x5Bu8; size]);
    let before = snapshot(&rig.game());

    let run = rig.run(&["--include", "game/one.dat"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.num("summary", "scoped"), 1);
    assert_eq!(
        run.num("summary", "total"),
        1,
        "only the selected file is planned"
    );

    let after = snapshot(&rig.game());
    let outside: Vec<&String> = after
        .keys()
        .filter(|k| k.as_str() != "game/one.dat")
        .filter(|k| before.get(*k) != after.get(*k))
        .collect();
    assert!(
        outside.is_empty(),
        "nothing outside the scope may be added, removed or changed: {outside:?}"
    );
    // The selected file was repaired; the rest were not.
    assert_eq!(rig.read("game/one.dat"), lay_out(&spec.new[0].chunks));
    assert_eq!(rig.read("game/two.dat"), vec![0x5Au8; size]);
    assert_eq!(rig.read("game/orphan.dat"), vec![0x5Bu8; size]);
}

/// A path with a **space** in it survives the `--include` glob.
///
/// The glob is compiled and matched against the `/`-separated rel path, so a
/// space is a character in a path and not a separator in a pattern.
#[tokio::test]
async fn a_path_with_a_space_survives_the_include_glob() {
    let size = 64usize;
    let spec = FixtureSpec::new(vec![FileSpec::new(
        "game/with space.dat",
        vec![chunk("spaced", 0x41, size, 0)],
    )]);
    let rig = Rig::start(&spec).await;
    rig.write("game/with space.dat", &vec![0x5Au8; size]);

    let run = rig.run(&["--include", "game/with space.dat"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.num("summary", "total"), 1, "the glob matched the path");
    assert_eq!(
        rig.read("game/with space.dat"),
        lay_out(&spec.new[0].chunks),
        "and the file was repaired"
    );
}

/// The trace log's download records agree with the server's request log.
///
/// `src/sophon.rs` documents `chunk downloaded chunk=<id>` as the supported
/// way to see which chunks were requested. This is the check that the
/// documented last resort actually works — and it is the check that catches a
/// reuse path that silently started downloading.
#[tokio::test]
async fn the_trace_logs_download_records_agree_with_the_servers_request_log() {
    let size = 64usize;
    let old = vec![
        chunk("old_a", 0x41, size, 0),
        chunk("old_b", 0x42, size, size as i64),
    ];
    let new = vec![
        chunk("new_a", 0x43, size, 0),
        chunk("new_b", 0x44, size, size as i64),
    ];
    let spec = FixtureSpec::new(vec![FileSpec::new("game/log.dat", new)])
        .with_old(vec![FileSpec::new("game/log.dat", old.clone())])
        .with_disk("game/log.dat", lay_out(&old));

    let rig = Rig::start(&spec).await;
    rig.at_local_version();
    let run = rig.run(&["--max-ram-bytes", "0"]).await;
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);

    let asked: BTreeSet<String> = rig.mock.chunk_hits().into_iter().collect();
    let logged = downloaded_chunks(&run.log);
    assert_eq!(
        logged, asked,
        "the trace log's download records must agree with the server's request log"
    );
    assert!(!asked.is_empty(), "the case has to download something");
}
