//! Small, self-contained measurements of the file-system operations the test
//! suite performs: fsync of a copied file, redb commits and opens, and the
//! cost of creating a fresh cache.
//!
//! Usage: iot-bench <scratch-dir>
//!
//! Run it once with the scratch directory on the disk under test (for the
//! numbers in `test-io-report.md`, the `E:` HDD) and once on a RAM disk, and
//! compare the `per_op` figures.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use redb::{Database, Durability, TableDefinition};

const ENTRIES: TableDefinition<&str, &[u8]> = TableDefinition::new("entries");
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn report(name: &str, total_ms: f64, ops: usize) {
    println!(
        "{name}: total={total_ms:.2}ms ops={ops} per_op={:.3}ms",
        total_ms / ops as f64
    );
}

/// Flush and pause before a phase. Without this the next phase's first fsync
/// is charged for the previous phase's writeback, which made the commit
/// figures swing by more than 100 ms between runs on the HDD.
fn settle(root: &Path) {
    let p = root.join(".settle");
    let mut f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&p)
        .unwrap();
    f.write_all(b"x").unwrap();
    f.sync_all().unwrap();
    drop(f);
    std::thread::sleep(std::time::Duration::from_millis(1000));
}

fn file_ops(root: &Path) {
    settle(root);
    let dir = root.join("files");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    // 200 copies of a 512-byte file, each with sync_all() -- what
    // tools/sync/src/commands/sync/apply.rs::copy_one does per copied file.
    let t = Instant::now();
    for i in 0..200 {
        let p = dir.join(format!("sync{i}.bin"));
        let mut f = File::create(&p).unwrap();
        f.write_all(&[b'x'; 512]).unwrap();
        f.sync_all().unwrap();
    }
    report("create+write512+sync_all", ms(t.elapsed()), 200);

    // The same without the fsync, to isolate its cost.
    let t = Instant::now();
    for i in 0..200 {
        let p = dir.join(format!("nosync{i}.bin"));
        let mut f = File::create(&p).unwrap();
        f.write_all(&[b'x'; 512]).unwrap();
    }
    report("create+write512", ms(t.elapsed()), 200);

    // Re-opening to set the modified time, as copy_one does after the copy.
    let t = Instant::now();
    for i in 0..200 {
        let p = dir.join(format!("sync{i}.bin"));
        let f = OpenOptions::new().write(true).open(&p).unwrap();
        f.set_modified(std::time::SystemTime::now()).unwrap();
    }
    report("reopen+set_modified", ms(t.elapsed()), 200);

    // Removing the 400 files one at a time, then the directory -- what a
    // TempDir drop costs.
    let t = Instant::now();
    let mut n = 0;
    for e in fs::read_dir(&dir).unwrap() {
        fs::remove_file(e.unwrap().path()).unwrap();
        n += 1;
    }
    report("remove_file", ms(t.elapsed()), n);
    let t = Instant::now();
    fs::remove_dir(&dir).unwrap();
    report("remove_dir", ms(t.elapsed()), 1);
}

/// Ten commits of ten rows, then ten writable opens that each insert meta --
/// the two things `CacheDb` does on every run.
fn redb_ops(root: &Path, durability: Durability, label: &str) {
    settle(root);
    let path = root.join(format!("cache-{label}.redb"));
    let _ = fs::remove_file(&path);
    let db = Database::create(&path).unwrap();
    let t = Instant::now();
    for round in 0..10 {
        let mut tx = db.begin_write().unwrap();
        tx.set_durability(durability).unwrap();
        {
            let mut t = tx.open_table(ENTRIES).unwrap();
            for i in 0..10 {
                let key = format!("row{round}_{i}");
                t.insert(key.as_str(), &[b'v'; 64][..]).unwrap();
            }
        }
        tx.commit().unwrap();
    }
    report(
        &format!("redb 10 commits x10 rows [{label}]"),
        ms(t.elapsed()),
        10,
    );
    drop(db);

    // Opening an existing file, then the meta commit that `init_meta`
    // performs on both sides of every run.
    let t = Instant::now();
    for _ in 0..10 {
        let db = Database::create(&path).unwrap();
        let mut tx = db.begin_write().unwrap();
        tx.set_durability(durability).unwrap();
        {
            let _ = tx.open_table(ENTRIES).unwrap();
            let mut m = tx.open_table(META).unwrap();
            m.insert("meta", &[b'm'; 8][..]).unwrap();
        }
        tx.commit().unwrap();
        drop(db);
    }
    report(
        &format!("redb open+meta commit [{label}]"),
        ms(t.elapsed()),
        10,
    );
    let _ = fs::remove_file(&path);
}

/// Split the open cost into open-only, open + an empty commit, and
/// open + a meta insert + commit.
fn redb_open_bench(root: &Path, mode: &str) {
    settle(root);
    let path = root.join(format!("open-{mode}.redb"));
    let _ = fs::remove_file(&path);
    {
        let db = Database::create(&path).unwrap();
        let tx = db.begin_write().unwrap();
        {
            let _ = tx.open_table(ENTRIES).unwrap();
            let mut m = tx.open_table(META).unwrap();
            m.insert("meta", &[b'm'; 8][..]).unwrap();
        }
        tx.commit().unwrap();
    }
    let t = Instant::now();
    for _ in 0..10 {
        let db = Database::create(&path).unwrap();
        match mode {
            "no-commit" => {}
            "empty-commit" => {
                let tx = db.begin_write().unwrap();
                {
                    let _ = tx.open_table(ENTRIES).unwrap();
                    let _ = tx.open_table(META).unwrap();
                }
                tx.commit().unwrap();
            }
            "insert-commit" => {
                let tx = db.begin_write().unwrap();
                {
                    let _ = tx.open_table(ENTRIES).unwrap();
                    let mut m = tx.open_table(META).unwrap();
                    m.insert("meta", &[b'm'; 8][..]).unwrap();
                }
                tx.commit().unwrap();
            }
            _ => unreachable!(),
        }
        drop(db);
    }
    report(&format!("redb open [{mode}]"), ms(t.elapsed()), 10);
    let _ = fs::remove_file(&path);
}

/// Creating a brand-new cache, as every test that builds a fresh pair does.
/// `durable` chooses the commit durability; the open cost is the same either
/// way, which is the point of measuring both.
fn fresh_create(root: &Path, durable: bool) {
    settle(root);
    let label = if durable { "durable" } else { "none" };
    let t = Instant::now();
    for i in 0..10 {
        let path = root.join(format!("fresh-{label}-{i}.redb"));
        let _ = fs::remove_file(&path);
        let db = Database::create(&path).unwrap();
        let mut tx = db.begin_write().unwrap();
        if !durable {
            tx.set_durability(Durability::None).unwrap();
        }
        {
            let _ = tx.open_table(ENTRIES).unwrap();
            let mut m = tx.open_table(META).unwrap();
            m.insert("meta", &[b'm'; 8][..]).unwrap();
        }
        tx.commit().unwrap();
        drop(db);
        let _ = fs::remove_file(&path);
    }
    report(
        &format!("redb fresh create (durable={durable})"),
        ms(t.elapsed()),
        10,
    );
}

/// What a 1 MB cache backup/snapshot copy or a fresh 1 MB file costs,
/// independent of redb.
fn raw_file(root: &Path) {
    settle(root);
    let t = Instant::now();
    for i in 0..10 {
        let p = root.join(format!("raw{i}.bin"));
        let f = File::create(&p).unwrap();
        f.set_len(1_048_576).unwrap();
        f.sync_all().unwrap();
        drop(f);
        let _ = fs::remove_file(&p);
    }
    report("file create 1MB + sync_all", ms(t.elapsed()), 10);
}

/// The whole-file copies `backup_db`/`snapshot_old` perform.
fn copy_1mb(root: &Path) {
    settle(root);
    let src = root.join("copy-src.bin");
    let f = File::create(&src).unwrap();
    f.set_len(1_048_576).unwrap();
    f.sync_all().unwrap();
    drop(f);

    let t = Instant::now();
    for i in 0..10 {
        fs::copy(&src, root.join(format!("copy{i}.bin"))).unwrap();
    }
    report("copy 1MB", ms(t.elapsed()), 10);

    for i in 0..10 {
        let _ = fs::remove_file(root.join(format!("copy{i}.bin")));
    }
    let _ = fs::remove_file(&src);
}

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).expect("scratch dir"));
    fs::create_dir_all(&root).unwrap();
    println!("scratch: {}", root.display());
    file_ops(&root);
    redb_ops(&root, Durability::Immediate, "immediate");
    redb_open_bench(&root, "no-commit");
    redb_open_bench(&root, "empty-commit");
    redb_open_bench(&root, "insert-commit");
    fresh_create(&root, true);
    fresh_create(&root, false);
    raw_file(&root);
    copy_1mb(&root);
    redb_ops(&root, Durability::None, "none");
    let _ = fs::remove_file(root.join(".settle"));
}
