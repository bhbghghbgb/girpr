// girsync — dev-only one-way mirror src(old) -> dst(working dir) with sled hash cache.
//
// Layout: <folder>/girpr-cache is a sled DB directory.
// Backups: <folder>/girpr-cache-backup-<ts> and <folder>/girpr-cache-old-<ts> (dir copies, keep-all).
// Record-only inputs: any path whose final component starts with "girpr-cache".

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use glob::{MatchOptions, Pattern};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

const META_KEY: &str = "\0meta";
const CACHE_PREFIX: &str = "girpr-cache";
const SUPPORTED_HASHES: &[&str] = &["md5", "sha256"];

#[derive(Parser, Debug)]
#[command(name = "girsync", about = "Dev-only one-way folder mirror with hash cache")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Build/refresh the record for a folder (always hashes per --hash, prunes missing).
    Update {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long = "hash", default_values_t = vec!["md5".to_string()])]
        hash: Vec<String>,
        #[arg(long = "include")]
        include: Vec<String>,
        #[arg(long = "exclude")]
        exclude: Vec<String>,
        /// Case-sensitive path handling. Default false = insensitive with rename/abort rules.
        #[arg(long, default_value_t = false)]
        case_sensitive: bool,
        #[arg(long, default_value_t = 10)]
        max_depth: usize,
        #[arg(long, default_value_t = false)]
        ignore_cache: bool,
    },
    /// Compare two sides (each: folder root or girpr-cache* record dir).
    Compare {
        #[arg(long)]
        src: PathBuf,
        #[arg(long)]
        dst: PathBuf,
        #[arg(long = "hash", default_values_t = vec!["md5".to_string()])]
        hash: Vec<String>,
        #[arg(long, default_value_t = false)]
        no_fast: bool,
        #[arg(long = "include")]
        include: Vec<String>,
        #[arg(long = "exclude")]
        exclude: Vec<String>,
        #[arg(long, default_value_t = false)]
        case_sensitive: bool,
        #[arg(long, default_value_t = 10)]
        max_depth: usize,
        #[arg(long, default_value_t = false)]
        ignore_cache: bool,
    },
    /// Mirror src folder -> dst folder.
    Sync {
        #[arg(long)]
        src: PathBuf,
        #[arg(long)]
        dst: PathBuf,
        #[arg(long = "hash", default_values_t = vec!["md5".to_string()])]
        hash: Vec<String>,
        #[arg(long, default_value_t = false)]
        no_fast: bool,
        /// Only copy src-only (missing) files; skip content updates.
        #[arg(long, default_value_t = false)]
        missing_only: bool,
        /// Keep dst-only files (default deletes them).
        #[arg(long, default_value_t = false)]
        keep_extra: bool,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[arg(long, default_value_t = 4)]
        jobs: usize,
        #[arg(long = "include")]
        include: Vec<String>,
        #[arg(long = "exclude")]
        exclude: Vec<String>,
        #[arg(long, default_value_t = false)]
        case_sensitive: bool,
        #[arg(long, default_value_t = 10)]
        max_depth: usize,
        #[arg(long, default_value_t = false)]
        ignore_cache: bool,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Meta {
    version: u32,
    case_sensitive: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct FileRec {
    /// "file" or "dir"
    kind: String,
    size: u64,
    mtime_ns: i64,
    hashes: HashMap<String, String>,
}

#[derive(Clone, Debug)]
struct EffRec {
    kind: String,
    size: u64,
    mtime_ns: i64,
    hashes: HashMap<String, String>,
}

#[derive(Clone, Debug)]
struct LiveEnt {
    rel: String,
    abs: PathBuf,
    is_dir: bool,
    size: u64,
    mtime_ns: i64,
}

fn main() {
    let cli = Cli::parse();
    let code = match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("FATAL {:#}", e);
            3
        }
    };
    std::process::exit(code);
}

fn run(cli: Cli) -> Result<i32> {
    match cli.cmd {
        Cmd::Update {
            dir,
            hash,
            include,
            exclude,
            case_sensitive,
            max_depth,
            ignore_cache,
        } => cmd_update(dir, hash, include, exclude, case_sensitive, max_depth, ignore_cache),
        Cmd::Compare {
            src,
            dst,
            hash,
            no_fast,
            include,
            exclude,
            case_sensitive,
            max_depth,
            ignore_cache,
        } => cmd_compare(
            src,
            dst,
            hash,
            !no_fast,
            include,
            exclude,
            case_sensitive,
            max_depth,
            ignore_cache,
        ),
        Cmd::Sync {
            src,
            dst,
            hash,
            no_fast,
            missing_only,
            keep_extra,
            dry_run,
            jobs,
            include,
            exclude,
            case_sensitive,
            max_depth,
            ignore_cache,
        } => cmd_sync(
            src,
            dst,
            hash,
            !no_fast,
            missing_only,
            keep_extra,
            dry_run,
            jobs,
            include,
            exclude,
            case_sensitive,
            max_depth,
            ignore_cache,
        ),
    }
}

// ---------- generic helpers ----------

fn ts_now() -> String {
    chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()
}

fn unique_sibling(parent: &Path, prefix: String) -> PathBuf {
    let mut p = parent.join(&prefix);
    let mut n = 1;
    while p.exists() {
        n += 1;
        p = parent.join(format!("{}_{}", prefix, n));
    }
    p
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)
        .with_context(|| format!("create dir {}", dst.display()))?;
    for ent in std::fs::read_dir(src).with_context(|| format!("read dir {}", src.display()))? {
        let ent = ent?;
        let ft = ent.file_type()?;
        let d = dst.join(ent.file_name());
        if ft.is_dir() {
            copy_dir_all(&ent.path(), &d)?;
        } else {
            std::fs::copy(ent.path(), &d)
                .with_context(|| format!("copy {} -> {}", ent.path().display(), d.display()))?;
        }
    }
    Ok(())
}

/// Copy existing sled DB dir to <parent>/girpr-cache-backup-<ts>. Returns backup path if made.
fn backup_db(db_path: &Path) -> Result<Option<PathBuf>> {
    if !db_path.exists() {
        return Ok(None);
    }
    let parent = db_path.parent().unwrap_or_else(|| Path::new("."));
    let dest = unique_sibling(parent, format!("girpr-cache-backup-{}", ts_now()));
    copy_dir_all(db_path, &dest)?;
    println!("backup {} -> {}", db_path.display(), dest.display());
    Ok(Some(dest))
}

fn snapshot_old(db_path: &Path) -> Result<Option<PathBuf>> {
    if !db_path.exists() {
        return Ok(None);
    }
    let parent = db_path.parent().unwrap_or_else(|| Path::new("."));
    let dest = unique_sibling(parent, format!("girpr-cache-old-{}", ts_now()));
    copy_dir_all(db_path, &dest)?;
    println!("snapshot-old {} -> {}", db_path.display(), dest.display());
    Ok(Some(dest))
}

fn is_record_path(p: &Path) -> bool {
    p.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.starts_with(CACHE_PREFIX))
        .unwrap_or(false)
}

fn is_cache_rel(rel: &str) -> bool {
    // Covers girpr-cache, girpr-cache-backup-*, girpr-cache-old-* at root.
    rel == CACHE_PREFIX
        || rel.starts_with("girpr-cache/")
        || rel.starts_with("girpr-cache-")
        || rel.split('/').next().map(|f| f.starts_with(CACHE_PREFIX)).unwrap_or(false)
}

fn mtime_ns_of(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| (d.as_nanos().min(i64::MAX as u128)) as i64)
        .unwrap_or(0)
}

fn parse_hash_list(input: &[String]) -> Result<Vec<String>> {
    let mut v: Vec<String> = input.iter().map(|s| s.to_lowercase()).collect();
    v.retain(|s| !s.is_empty());
    if v.iter().any(|s| s == "none") {
        if v.len() != 1 {
            bail!("--hash none cannot be combined with other algorithms");
        }
        return Ok(vec![]);
    }
    if v.is_empty() {
        return Ok(vec![]);
    }
    for a in &v {
        if !SUPPORTED_HASHES.contains(&a.as_str()) {
            bail!(
                "unsupported hash '{}' (supported: {} or none)",
                a,
                SUPPORTED_HASHES.join(",")
            );
        }
    }
    v.dedup();
    Ok(v)
}

fn hash_file(path: &Path, algos: &[String]) -> Result<HashMap<String, String>> {
    use sha2::Digest;
    let mut md5ctx = if algos.contains(&"md5".to_string()) {
        Some(md5::Context::new())
    } else {
        None
    };
    let mut sha2ctx = if algos.contains(&"sha256".to_string()) {
        Some(sha2::Sha256::new())
    } else {
        None
    };
    if algos.is_empty() {
        return Ok(HashMap::new());
    }
    let mut f =
        std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut buf = vec![0u8; 512 * 1024];
    loop {
        let n: usize = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        if let Some(c) = md5ctx.as_mut() {
            c.consume(chunk);
        }
        if let Some(c) = sha2ctx.as_mut() {
            use sha2::Digest;
            c.update(chunk);
        }
    }
    let mut out = HashMap::new();
    if let Some(c) = md5ctx {
        out.insert("md5".into(), format!("{:x}", c.compute()));
    }
    if let Some(c) = sha2ctx {
        use sha2::Digest;
        out.insert("sha256".into(), hex::encode(c.finalize()));
    }
    return Ok(out);

    mod hex {
        pub fn encode(b: impl AsRef<[u8]>) -> String {
            b.as_ref().iter().map(|x| format!("{:02x}", x)).collect()
        }
    }
}

fn compile_patterns(list: &[String]) -> Result<Vec<Pattern>> {
    list.iter()
        .map(|s| Pattern::new(s).with_context(|| format!("bad glob '{}'", s)))
        .collect()
}

fn is_excluded(
    rel: &str,
    includes: &[Pattern],
    excludes: &[Pattern],
    case_sensitive: bool,
) -> bool {
    let opts = MatchOptions {
        case_sensitive,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    };
    if !includes.is_empty() && !includes.iter().any(|p| p.matches_with(rel, opts)) {
        return true;
    }
    if excludes.iter().any(|p| p.matches_with(rel, opts)) {
        return true;
    }
    false
}

fn walk_live(root: &Path, max_depth: usize) -> Result<Vec<LiveEnt>> {
    let mut out = Vec::new();
    let mut it = walkdir::WalkDir::new(root)
        .follow_links(true)
        .max_depth(max_depth)
        .into_iter()
        .filter_entry(|e| {
            // Prune cache dirs from traversal.
            let rel = e
                .path()
                .strip_prefix(root)
                .ok()
                .and_then(|p| p.to_str())
                .map(|s| s.replace('\\', "/"))
                .unwrap_or_default();
            if rel.is_empty() {
                return true;
            }
            !is_cache_rel(&rel)
        });
    loop {
        match it.next() {
            None => break,
            Some(Err(e)) => bail!("walk {}: {}", root.display(), e), // dangling link / loop / IO -> abort
            Some(Ok(ent)) => {
                if ent.depth() == 0 {
                    continue;
                }
                let rel = ent
                    .path()
                    .strip_prefix(root)
                    .ok()
                    .and_then(|p| p.to_str())
                    .with_context(|| {
                        format!("non-UTF8 path {}", ent.path().display())
                    })?
                    .replace('\\', "/");
                if rel.is_empty() || is_cache_rel(&rel) {
                    continue;
                }
                let ft = ent.file_type();
                if ft.is_dir() {
                    out.push(LiveEnt {
                        rel,
                        abs: ent.path().to_path_buf(),
                        is_dir: true,
                        size: 0,
                        mtime_ns: 0,
                    });
                } else if ft.is_file() {
                    let meta = std::fs::metadata(ent.path()).with_context(|| {
                        format!("stat {}", ent.path().display())
                    })?;
                    out.push(LiveEnt {
                        rel,
                        abs: ent.path().to_path_buf(),
                        is_dir: false,
                        size: meta.len(),
                        mtime_ns: mtime_ns_of(&meta),
                    });
                } else {
                    bail!("unsupported file type {}", ent.path().display());
                }
            }
        }
    }
    Ok(out)
}

/// Abort if any two live rels differ only by case (when !case_sensitive).
fn check_mixed_case(live: &[LiveEnt], label: &str, case_sensitive: bool) -> Result<()> {
    if case_sensitive {
        return Ok(());
    }
    let mut map: HashMap<String, Vec<&str>> = HashMap::new();
    for e in live {
        map.entry(e.rel.to_lowercase()).or_default().push(&e.rel);
    }
    for (_, v) in map {
        let uniq: HashSet<&&str> = v.iter().collect();
        if uniq.len() > 1 {
            let mut sorted: Vec<String> = uniq.into_iter().map(|s| s.to_string()).collect();
            sorted.sort();
            bail!("mixed-case collision in {}: {}", label, sorted.join(" vs "));
        }
    }
    Ok(())
}

// ---------- sled helpers ----------

fn open_db(db_path: &Path, case_sensitive: bool, ignore_cache: bool, backup_first: bool) -> Result<sled::Db> {
    if ignore_cache && db_path.exists() {
        if backup_first {
            backup_db(db_path)?;
        }
        std::fs::remove_dir_all(db_path)
            .with_context(|| format!("remove {}", db_path.display()))?;
        println!("ignore-cache: removed {}", db_path.display());
    } else if backup_first && db_path.exists() {
        backup_db(db_path)?;
    }
    let db = sled::open(db_path)
        .with_context(|| format!("open sled db {} (corrupt? use --ignore-cache)", db_path.display()))?;
    match db.get(META_KEY)? {
        None => {
            db.insert(META_KEY, serde_json::to_vec(&Meta { version: 1, case_sensitive })?)?;
            db.flush()?;
        }
        Some(v) => {
            let m: Meta = serde_json::from_slice(&v)
                .context("parse cache meta (corrupt? use --ignore-cache)")?;
            if m.version != 1 {
                bail!("unsupported cache version {} (use --ignore-cache to rebuild)", m.version);
            }
            // NOTE: case_sensitive is informational only. The cache stays usable
            // across modes: in insensitive mode a disk/cached casing difference is
            // fixed to the on-disk name (disk always governs), so the same record
            // remains valid for a later sensitive run. Only genuine conflicts
            // (two live/record paths differing only by case in insensitive mode)
            // abort, detected by the callers — never here.
            if m.case_sensitive != case_sensitive {
                db.insert(META_KEY, serde_json::to_vec(&Meta { version: 1, case_sensitive })?)?;
            }
        }
    }
    Ok(db)
}

fn load_all_records(db: &sled::Db) -> Result<HashMap<String, FileRec>> {
    let mut map = HashMap::new();
    for kv in db.iter() {
        let (k, v) = kv?;
        if k.as_ref() == META_KEY.as_bytes() {
            continue;
        }
        let rel = String::from_utf8(k.to_vec()).context("non-UTF8 key in cache")?;
        let rec: FileRec = serde_json::from_slice(&v).context("parse cache entry (corrupt?)")?;
        map.insert(rel, rec);
    }
    Ok(map)
}

fn put_rec(db: &sled::Db, rel: &str, rec: &FileRec) -> Result<()> {
    db.insert(rel.as_bytes(), serde_json::to_vec(rec)?)?;
    Ok(())
}

/// Build effective map for a folder, using embedded cache to short-circuit.
/// Writes back to cache unless dry_run. Prunes DB rows missing from disk.
#[allow(clippy::too_many_arguments)]
fn build_effective_folder(
    root: &Path,
    db: &sled::Db,
    algos: &[String],
    fast: bool,
    includes: &[Pattern],
    excludes: &[Pattern],
    case_sensitive: bool,
    max_depth: usize,
    force_hash: bool,
    dry_run: bool,
) -> Result<HashMap<String, EffRec>> {
    let live = walk_live(root, max_depth)?;
    check_mixed_case(&live, &root.display().to_string(), case_sensitive)?;
    let mut eff = HashMap::new();
    let mut live_set: HashSet<String> = HashSet::new();
    for e in &live {
        if is_excluded(&e.rel, includes, excludes, case_sensitive) {
            continue; // treated as not existing at all
        }
        live_set.insert(e.rel.clone());
    }
    // In insensitive mode the on-disk name governs: index cached keys by
    // lowercase so a disk/cached casing difference is fixed to the disk name
    // first (reusing the cached hashes when stat matches) instead of
    // erroring. Multiple stale alternates are just pruned below, not a conflict.
    let alt_recs: HashMap<String, FileRec> = if !case_sensitive {
        load_all_records(db)?
    } else {
        HashMap::new()
    };
    let mut alt_index: HashMap<String, Vec<String>> = HashMap::new();
    if !case_sensitive {
        for key in alt_recs.keys() {
            alt_index.entry(key.to_lowercase()).or_default().push(key.clone());
        }
    }
    for e in live {
        if is_excluded(&e.rel, includes, excludes, case_sensitive) {
            continue;
        }
        if e.is_dir {
            eff.insert(
                e.rel.clone(),
                EffRec {
                    kind: "dir".into(),
                    size: 0,
                    mtime_ns: 0,
                    hashes: HashMap::new(),
                },
            );
            if !dry_run {
                let cur: Option<FileRec> = db
                    .get(e.rel.as_bytes())?
                    .map(|v| serde_json::from_slice(&v))
                    .transpose()
                    .context("parse cache entry")?;
                if cur.map(|c| c.kind != "dir").unwrap_or(true) {
                    put_rec(
                        db,
                        &e.rel,
                        &FileRec {
                            kind: "dir".into(),
                            size: 0,
                            mtime_ns: 0,
                            hashes: HashMap::new(),
                        },
                    )?;
                }
            }
            continue;
        }
        // file
        // `adopted` = cache entry came from an alternate-cased key, so the
        // disk-cased key must be (re)written even on a fast-hit.
        let (cached, adopted): (Option<FileRec>, bool) = match db
            .get(e.rel.as_bytes())?
            .map(|v| serde_json::from_slice(&v))
            .transpose()
            .context("parse cache entry")?
        {
            Some(c) => (Some(c), false),
            // Exact-case miss in insensitive mode: adopt the single stale
            // alternate-cased entry as the cache candidate (disk name wins).
            None if !case_sensitive => {
                match alt_index.get(&e.rel.to_lowercase()) {
                    Some(alts) => {
                        let others: Vec<&String> =
                            alts.iter().filter(|k| *k != &e.rel).collect();
                        if others.len() == 1 {
                            let old = others[0];
                            let rec = alt_recs.get(old).cloned();
                            if !dry_run {
                                db.remove(old.as_bytes())?; // old casing dropped; new one upserted below
                            }
                            (rec, true)
                        } else {
                            (None, false) // zero (truly new) or several stale: prune handles leftovers
                        }
                    }
                    None => (None, false),
                }
            }
            None => (None, false),
        };
        let fresh = cached.as_ref().map(|c| c.size == e.size && c.mtime_ns == e.mtime_ns && c.kind == "file").unwrap_or(false);
        if fresh && !force_hash {
            let c = cached.as_ref().unwrap();
            let have_all = algos.iter().all(|a| c.hashes.contains_key(a));
            if have_all {
                if adopted && !dry_run {
                    put_rec(db, &e.rel, c)?;
                }
                eff.insert(
                    e.rel.clone(),
                    EffRec {
                        kind: "file".into(),
                        size: c.size,
                        mtime_ns: c.mtime_ns,
                        hashes: c.hashes.clone(),
                    },
                );
                continue;
            }
        }
        // stale or missing or --no-fast or missing algos: drop stale entry, rehash
        if !dry_run && cached.is_some() && !fresh {
            db.remove(e.rel.as_bytes())?; // preemptively drop whole path record
        }
        let have_all = cached.as_ref().map(|c| algos.iter().all(|a| c.hashes.contains_key(a))).unwrap_or(false);
        if !fast || !fresh || force_hash || !have_all {
            let hashes = hash_file(&e.abs, algos)
                .with_context(|| format!("hash {}", e.abs.display()))?;
            if !dry_run {
                put_rec(
                    db,
                    &e.rel,
                    &FileRec {
                        kind: "file".into(),
                        size: e.size,
                        mtime_ns: e.mtime_ns,
                        hashes: hashes.clone(),
                    },
                )?;
            }
            eff.insert(
                e.rel.clone(),
                EffRec {
                    kind: "file".into(),
                    size: e.size,
                    mtime_ns: e.mtime_ns,
                    hashes,
                },
            );
        } else {
            // fast hit path (fresh && fast && have_all handled above); fallback hash
            let c = cached.as_ref().unwrap();
            eff.insert(
                e.rel.clone(),
                EffRec {
                    kind: "file".into(),
                    size: c.size,
                    mtime_ns: c.mtime_ns,
                    hashes: c.hashes.clone(),
                },
            );
        }
    }
    // prune DB rows for files no longer on disk (or now excluded)
    if !dry_run {
        let existing = load_all_records(db)?;
        for rel in existing.keys() {
            if !live_set.contains(rel) {
                db.remove(rel.as_bytes())?;
            }
        }
        db.flush()?;
    }
    Ok(eff)
}

fn load_record_side(
    db_path: &Path,
    includes: &[Pattern],
    excludes: &[Pattern],
    case_sensitive: bool,
) -> Result<HashMap<String, EffRec>> {
    if !db_path.exists() {
        bail!("record {} not found", db_path.display());
    }
    let db = sled::open(db_path)
        .with_context(|| format!("open record {} (corrupt?)", db_path.display()))?;
    let all = load_all_records(&db)?;
    let mut out = HashMap::new();
    for (rel, r) in all {
        if is_cache_rel(&rel) {
            continue;
        }
        if is_excluded(&rel, includes, excludes, case_sensitive) {
            continue;
        }
        out.insert(
            rel,
            EffRec {
                kind: r.kind,
                size: r.size,
                mtime_ns: r.mtime_ns,
                hashes: r.hashes,
            },
        );
    }
    if !case_sensitive {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for k in out.keys() {
            map.entry(k.to_lowercase()).or_default().push(k.clone());
        }
        for (_, v) in map {
            let uniq: HashSet<&String> = v.iter().collect();
            if uniq.len() > 1 {
                bail!("mixed-case collision in record {}: {}", db_path.display(), v.join(" vs "));
            }
        }
    }
    Ok(out)
}

// ---------- diff ----------

#[derive(Default)]
struct Diff {
    missing: Vec<String>,
    extra: Vec<String>,
    changed: Vec<String>,
    type_conflict: Vec<String>,
    case_mismatch: Vec<(String, String)>,
}

fn diff_maps(
    src: &HashMap<String, EffRec>,
    dst: &HashMap<String, EffRec>,
    algos: &[String],
    case_sensitive: bool,
) -> Diff {
    let mut d = Diff::default();
    if case_sensitive {
        let mut keys: HashSet<&String> = HashSet::new();
        for k in src.keys().chain(dst.keys()) {
            keys.insert(k);
        }
        for k in keys {
            match (src.get(k), dst.get(k)) {
                (Some(_s), None) => {
                    d.missing.push(k.clone());
                }
                (None, Some(_)) => {
                    d.extra.push(k.clone());
                }
                (Some(s), Some(t)) => {
                    if s.kind != t.kind {
                        d.type_conflict.push(k.clone());
                    } else if s.kind == "dir" {
                        // presence only
                    } else if s.size != t.size || s.mtime_ns != t.mtime_ns || hashes_differ(&s.hashes, &t.hashes, algos) {
                        d.changed.push(k.clone());
                    }
                }
                (None, None) => {}
            }
        }
    } else {
        // lower -> original (mixed-case already validated absent within each side)
        let mut slow: HashMap<String, &String> = HashMap::new();
        let mut dlow: HashMap<String, &String> = HashMap::new();
        for k in src.keys() {
            slow.insert(k.to_lowercase(), k);
        }
        for k in dst.keys() {
            dlow.insert(k.to_lowercase(), k);
        }
        let mut keys: HashSet<String> = HashSet::new();
        for k in slow.keys().chain(dlow.keys()) {
            keys.insert(k.clone());
        }
        for lk in keys {
            match (slow.get(&lk), dlow.get(&lk)) {
                (Some(srel), None) => d.missing.push((*srel).clone()),
                (None, Some(trel)) => d.extra.push((*trel).clone()),
                (Some(srel), Some(trel)) => {
                    if *srel != *trel {
                        d.case_mismatch.push(((*srel).clone(), (*trel).clone()));
                    }
                    let s = &src[*srel];
                    let t = &dst[*trel];
                    if s.kind != t.kind {
                        d.type_conflict.push((*srel).clone());
                    } else if s.kind == "dir" {
                    } else if s.size != t.size || s.mtime_ns != t.mtime_ns || hashes_differ(&s.hashes, &t.hashes, algos) {
                        d.changed.push((*srel).clone());
                    }
                }
                (None, None) => {}
            }
        }
    }
    d.missing.sort();
    d.extra.sort();
    d.changed.sort();
    d.type_conflict.sort();
    d.case_mismatch.sort();
    d
}

fn hashes_differ(a: &HashMap<String, String>, b: &HashMap<String, String>, algos: &[String]) -> bool {
    for algo in algos {
        match (a.get(algo), b.get(algo)) {
            (Some(x), Some(y)) => {
                if x != y {
                    return true;
                }
            }
            // If either side lacks the hash (e.g. --hash none history), fall back to
            // size+mtime which the caller already compared; do not force differ here.
            _ => {}
        }
    }
    false
}

// ---------- commands ----------

fn cmd_update(
    dir: PathBuf,
    hash: Vec<String>,
    include: Vec<String>,
    exclude: Vec<String>,
    case_sensitive: bool,
    max_depth: usize,
    ignore_cache: bool,
) -> Result<i32> {
    let algos = parse_hash_list(&hash)?;
    let includes = compile_patterns(&include)?;
    let excludes = compile_patterns(&exclude)?;
    if !dir.is_dir() {
        bail!("--dir {} is not a directory", dir.display());
    }
    let db_path = dir.join(CACHE_PREFIX);
    let db = open_db(&db_path, case_sensitive, ignore_cache, true)?;
    let eff = build_effective_folder(
        &dir, &db, &algos, true, &includes, &excludes, case_sensitive, max_depth,
        true, // update mode: ensure hashes populated
        false,
    )?;
    let files = eff.values().filter(|r| r.kind == "file").count();
    let dirs = eff.values().filter(|r| r.kind == "dir").count();
    println!("update {} files={} dirs={} algos=[{}]", dir.display(), files, dirs, algos.join(","));
    Ok(0)
}

enum Side {
    Folder(PathBuf),
    Record(PathBuf),
}

fn classify(p: &Path) -> Side {
    if is_record_path(p) {
        Side::Record(p.to_path_buf())
    } else {
        Side::Folder(p.to_path_buf())
    }
}

fn load_side(
    side: &Side,
    algos: &[String],
    fast: bool,
    includes: &[Pattern],
    excludes: &[Pattern],
    case_sensitive: bool,
    max_depth: usize,
    ignore_cache: bool,
    force_hash: bool,
    dry_run: bool,
) -> Result<HashMap<String, EffRec>> {
    match side {
        Side::Record(dbp) => load_record_side(dbp, includes, excludes, case_sensitive),
        Side::Folder(root) => {
            if !root.is_dir() {
                bail!("folder {} not found", root.display());
            }
            let db_path = root.join(CACHE_PREFIX);
            if !db_path.exists() {
                // missing cache: just create it
                let db = open_db(&db_path, case_sensitive, false, false)?;
                let eff = build_effective_folder(
                    root, &db, algos, fast, includes, excludes, case_sensitive, max_depth,
                    force_hash, dry_run,
                )?;
                return Ok(eff);
            }
            match open_db(&db_path, case_sensitive, ignore_cache, !dry_run) {
                Ok(db) => build_effective_folder(
                    root, &db, algos, fast, includes, excludes, case_sensitive, max_depth,
                    force_hash, dry_run,
                ),
                Err(e) => Err(e.context(format!("cache for {} (use --ignore-cache to rebuild)", root.display()))),
            }
        }
    }
}

fn cmd_compare(
    src: PathBuf,
    dst: PathBuf,
    hash: Vec<String>,
    fast: bool,
    include: Vec<String>,
    exclude: Vec<String>,
    case_sensitive: bool,
    max_depth: usize,
    ignore_cache: bool,
) -> Result<i32> {
    let algos = parse_hash_list(&hash)?;
    let includes = compile_patterns(&include)?;
    let excludes = compile_patterns(&exclude)?;
    let s = classify(&src);
    let d = classify(&dst);
    let sm = load_side(&s, &algos, fast, &includes, &excludes, case_sensitive, max_depth, ignore_cache, false, false)?;
    let dm = load_side(&d, &algos, fast, &includes, &excludes, case_sensitive, max_depth, ignore_cache, false, false)?;
    let diff = diff_maps(&sm, &dm, &algos, case_sensitive);
    for r in &diff.missing {
        println!("MISSING {}", r);
    }
    for r in &diff.extra {
        println!("EXTRA {}", r);
    }
    for r in &diff.changed {
        println!("CHANGED {}", r);
    }
    for r in &diff.type_conflict {
        println!("TYPE-CONFLICT {}", r);
    }
    for (a, b) in &diff.case_mismatch {
        println!("CASE-MISMATCH {} <=> {}", a, b);
    }
    let total = diff.missing.len() + diff.extra.len() + diff.changed.len() + diff.type_conflict.len();
    println!(
        "SUMMARY missing={} extra={} changed={} type_conflict={} case_mismatch={} total_diff={}",
        diff.missing.len(),
        diff.extra.len(),
        diff.changed.len(),
        diff.type_conflict.len(),
        diff.case_mismatch.len(),
        total
    );
    Ok(if total == 0 && diff.case_mismatch.is_empty() { 0 } else { 4 })
}

#[allow(clippy::too_many_arguments)]
fn cmd_sync(
    src: PathBuf,
    dst: PathBuf,
    hash: Vec<String>,
    fast: bool,
    missing_only: bool,
    keep_extra: bool,
    dry_run: bool,
    jobs: usize,
    include: Vec<String>,
    exclude: Vec<String>,
    case_sensitive: bool,
    max_depth: usize,
    ignore_cache: bool,
) -> Result<i32> {
    let algos = parse_hash_list(&hash)?;
    let includes = compile_patterns(&include)?;
    let excludes = compile_patterns(&exclude)?;
    if is_record_path(&src) || is_record_path(&dst) {
        bail!("sync needs folder vs folder (record inputs are compare-only)");
    }
    if !src.is_dir() || !dst.is_dir() {
        bail!("src and dst must both be directories");
    }
    let canon_src = src.canonicalize().with_context(|| format!("canon {}", src.display()))?;
    let canon_dst = dst.canonicalize().with_context(|| format!("canon {}", dst.display()))?;
    if canon_src == canon_dst {
        bail!("src == dst");
    }
    if jobs == 0 {
        bail!("--jobs must be >= 1");
    }

    let src_db_path = src.join(CACHE_PREFIX);
    let dst_db_path = dst.join(CACHE_PREFIX);

    if !dry_run {
        // backups before modifying either DB
        if src_db_path.exists() {
            backup_db(&src_db_path)?;
        }
        if dst_db_path.exists() {
            backup_db(&dst_db_path)?;
            // old-state snapshot before any dst changes
            snapshot_old(&dst_db_path)?;
        }
        if ignore_cache {
            for p in [&src_db_path, &dst_db_path] {
                if p.exists() {
                    std::fs::remove_dir_all(p)
                        .with_context(|| format!("remove {}", p.display()))?;
                    println!("ignore-cache: removed {}", p.display());
                }
            }
        }
    }

    let src_db = if dry_run {
        // open without backup; avoid creating walls of writes later (writes skipped by flag)
        open_db(&src_db_path, case_sensitive, false, false).or_else(|_| {
            // dry-run on missing cache: use temp DB so we never create real one
            sled::Config::new().temporary(true).open().context("open temp db")
        })?
    } else {
        open_db(&src_db_path, case_sensitive, false, false)?
    };
    let dst_db = if dry_run {
        open_db(&dst_db_path, case_sensitive, false, false).or_else(|_| {
            sled::Config::new().temporary(true).open().context("open temp db")
        })?
    } else {
        open_db(&dst_db_path, case_sensitive, false, false)?
    };

    let sm = build_effective_folder(
        &src, &src_db, &algos, fast, &includes, &excludes, case_sensitive, max_depth,
        false, dry_run,
    )?;
    let mut dm = build_effective_folder(
        &dst, &dst_db, &algos, fast, &includes, &excludes, case_sensitive, max_depth,
        false, dry_run,
    )?;

    // case-insensitive rename pass (sync mode): rename dst to src casing first.
    let mut renamed = 0usize;
    if !case_sensitive {
        // map lower -> (src_rel, dst_rel)
        let mut slow: HashMap<String, &String> = HashMap::new();
        for k in sm.keys() {
            slow.insert(k.to_lowercase(), k);
        }
        let mut renames: Vec<(PathBuf, PathBuf, String, String)> = Vec::new();
        for (lk, srel) in &slow {
            // find dst key with same lower but different case
            if let Some(drel) = dm.keys().find(|k| k.to_lowercase() == *lk && *k != *srel) {
                renames.push((dst.join(drel), dst.join(*srel), (*drel).clone(), (*srel).clone()));
            }
        }
        for (from, to, drel, srel) in renames {
            println!("RENAME {} -> {}", drel, srel);
            // Keep the in-memory map in sync in both real and dry-run so the
            // post-rename diff sees aligned keys. FS + DB writes are real-only.
            if let Some(v) = dm.remove(&drel) {
                dm.insert(srel.clone(), v);
            }
            if !dry_run {
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("mkdir {}", parent.display()))?;
                }
                // two-step for Windows case-only rename
                let tmp = from.with_extension("girsync_case_tmp");
                std::fs::rename(&from, &tmp)
                    .with_context(|| format!("rename {} (case fix step1)", from.display()))?;
                std::fs::rename(&tmp, &to)
                    .with_context(|| format!("rename to {} (case fix step2)", to.display()))?;
                // move DB entry
                if let Ok(Some(raw)) = dst_db.get(drel.as_bytes()) {
                    dst_db.remove(drel.as_bytes())?;
                    dst_db.insert(srel.as_bytes(), raw)?;
                }
                renamed += 1;
            } else {
                renamed += 1;
            }
        }
    }

    let diff = diff_maps(&sm, &dm, &algos, true /* post-rename: exact keys */);
    // NOTE: after rename pass, keys align; recompute case-sensitive diff.

    let mut to_copy: Vec<String> = Vec::new();
    for r in &diff.missing {
        if sm.get(r).map(|e| e.kind == "file").unwrap_or(false) {
            to_copy.push(r.clone());
        }
    }
    if !missing_only {
        for r in &diff.changed {
            if sm.get(r).map(|e| e.kind == "file").unwrap_or(false) {
                to_copy.push(r.clone());
            }
        }
        // type-conflicts where src is file: remove dst then copy; where src is dir: remove file then mkdir
        for r in &diff.type_conflict {
            if sm.get(r).map(|e| e.kind == "file").unwrap_or(false) {
                to_copy.push(r.clone());
            }
        }
    }
    to_copy.sort();
    to_copy.dedup();

    let mut to_delete_files: Vec<String> = Vec::new();
    let mut to_fix_dirs: Vec<String> = Vec::new(); // dst type-conflict where src is dir
    if !keep_extra {
        for r in &diff.extra {
            if dm.get(r).map(|e| e.kind == "file").unwrap_or(false) {
                to_delete_files.push(r.clone());
            }
            // extra dirs are removed by the unknown-dirs pass below
        }
    }
    for r in &diff.type_conflict {
        match sm.get(r) {
            Some(s) if s.kind == "dir" => to_fix_dirs.push(r.clone()),
            _ => {
                if !keep_extra {
                    // dst file/dir vs src file handled via copy (dst dir removed first)
                }
            }
        }
    }

    // src dirs to ensure
    let mut to_mkdir: Vec<String> = Vec::new();
    for (rel, rec) in &sm {
        if rec.kind == "dir" && !dm.contains_key(rel) {
            to_mkdir.push(rel.clone());
        }
    }

    if dry_run {
        for r in &to_mkdir {
            println!("MKDIR {}", r);
        }
        for r in &to_copy {
            println!("COPY {}", r);
        }
        for r in &to_delete_files {
            println!("DELETE {}", r);
        }
        for r in &to_fix_dirs {
            println!("RMDIR-FILE {}", r);
        }
        println!(
            "SUMMARY renamed={} mkdir={} copy={} delete={} missing_only={} keep_extra={} dry_run=true",
            renamed,
            to_mkdir.len(),
            to_copy.len(),
            to_delete_files.len() + to_fix_dirs.len(),
            missing_only,
            keep_extra
        );
        return Ok(0);
    }

    // Apply: mkdirs
    for r in &to_mkdir {
        std::fs::create_dir_all(dst.join(r))
            .with_context(|| format!("mkdir {}", dst.join(r).display()))?;
        dst_db.insert(
            r.as_bytes(),
            serde_json::to_vec(&FileRec {
                kind: "dir".into(),
                size: 0,
                mtime_ns: 0,
                hashes: HashMap::new(),
            })?,
        )?;
    }
    // Fix type-conflicts where src is dir: remove dst file, mkdir
    for r in &to_fix_dirs {
        dst_db.remove(r.as_bytes())?; // delete entries before change
        let p = dst.join(r);
        if p.is_file() || p.is_symlink() {
            std::fs::remove_file(&p).with_context(|| format!("remove {}", p.display()))?;
        } else if p.is_dir() {
            std::fs::remove_dir_all(&p).with_context(|| format!("rmdir {}", p.display()))?;
        }
        std::fs::create_dir_all(&p).with_context(|| format!("mkdir {}", p.display()))?;
        println!("FIX-DIR {}", r);
    }

    // Delete entries before file changes (crash leaves truncated risk documented;
    // entries already dropped so resume re-copies).
    for r in to_copy.iter().chain(to_delete_files.iter()) {
        dst_db.remove(r.as_bytes())?;
    }
    dst_db.flush()?;

    // Delete extra files (+ prune unknown dirs afterwards)
    let mut deleted = 0usize;
    for r in &to_delete_files {
        let p = dst.join(r);
        // eff map only tracks files here for extra; dirs handled below
        if let Some(rec) = dm.get(r) {
            if rec.kind == "dir" {
                continue;
            }
        }
        if p.is_file() || p.is_symlink() {
            std::fs::remove_file(&p).with_context(|| format!("delete {}", p.display()))?;
            println!("DELETE {}", r);
            deleted += 1;
        } else if p.is_dir() {
            std::fs::remove_dir_all(&p).with_context(|| format!("rmdir {}", p.display()))?;
            println!("RMDIR {}", r);
            deleted += 1;
        } else if p.exists() {
            bail!("unsupported type {}", p.display());
        }
    }

    // Copy files in parallel (truncate + write in place, preserve mtime, verify).
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .context("build thread pool")?;
    let results = pool.install(|| {
        use rayon::prelude::*;
        to_copy.par_iter().map(|rel| copy_one(&src, &dst, rel, &algos)).collect::<Vec<_>>()
    });
    let mut copied = 0usize;
    let mut new_recs: Vec<(String, FileRec)> = Vec::new();
    for (rel, r) in to_copy.iter().zip(results) {
        let rec = r.with_context(|| format!("copy {}", rel))?;
        println!("COPY {}", rel);
        new_recs.push((rel.clone(), rec));
        copied += 1;
    }
    for (rel, rec) in new_recs {
        dst_db.insert(rel.as_bytes(), serde_json::to_vec(&rec)?)?;
    }

    // Remove dst dirs not in src (unknown empties + leftovers), deepest first.
    let mut removed_dirs = 0usize;
    let live_after = walk_live(&dst, max_depth)?;
    let mut src_dirs: HashSet<String> = sm
        .iter()
        .filter(|(_, v)| v.kind == "dir")
        .map(|(k, _)| k.clone())
        .collect();
    let _ = &mut src_dirs;
    let mut unknown_dirs: Vec<String> = Vec::new();
    for e in &live_after {
        if !e.is_dir {
            continue;
        }
        if is_excluded(&e.rel, &includes, &excludes, case_sensitive) {
            continue;
        }
        if !sm.contains_key(&e.rel) {
            unknown_dirs.push(e.rel.clone());
        }
    }
    unknown_dirs.sort_by_key(|s| std::cmp::Reverse(s.len()));
    for r in unknown_dirs {
        let p = dst.join(&r);
        if p.is_dir() {
            // dir may already be gone as child of removed parent
            if let Err(e) = std::fs::remove_dir_all(&p) {
                if p.exists() {
                    bail!("rmdir {}: {:#}", p.display(), e);
                }
            }
            dst_db.remove(r.as_bytes())?;
            println!("RMDIR {}", r);
            removed_dirs += 1;
        }
    }
    // final prune of anything else missing + flush
    {
        let live2 = walk_live(&dst, max_depth)?;
        let mut set: HashSet<String> = HashSet::new();
        for e in live2 {
            if !is_excluded(&e.rel, &includes, &excludes, case_sensitive) {
                set.insert(e.rel);
            }
        }
        for kv in dst_db.iter() {
            let (k, _) = kv?;
            if k.as_ref() == META_KEY.as_bytes() {
                continue;
            }
            let rel = String::from_utf8(k.to_vec()).context("non-UTF8 key")?;
            if !set.contains(&rel) {
                dst_db.remove(rel.as_bytes())?;
            }
        }
    }
    src_db.flush()?;
    dst_db.flush()?;

    println!(
        "SUMMARY renamed={} mkdir={} copied={} deleted={} rmdir={} missing_only={} keep_extra={}",
        renamed,
        to_mkdir.len(),
        copied,
        deleted,
        removed_dirs,
        missing_only,
        keep_extra
    );
    Ok(0)
}

fn copy_one(src_root: &Path, dst_root: &Path, rel: &str, algos: &[String]) -> Result<FileRec> {
    let s = src_root.join(rel);
    let d = dst_root.join(rel);
    // dst dir that is in the way of a file copy: remove first
    if d.is_dir() && !d.is_symlink() {
        std::fs::remove_dir_all(&d).with_context(|| format!("rmdir {}", d.display()))?;
    }
    if let Some(parent) = d.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let smeta = std::fs::metadata(&s).with_context(|| format!("stat {}", s.display()))?;
    let ssize = smeta.len();
    let smtime_sys = smeta.modified().ok();
    let smtime_ns = mtime_ns_of(&smeta);
    // stream copy (truncate + write in place)
    let mut fin =
        std::fs::File::open(&s).with_context(|| format!("open {}", s.display()))?;
    let fout =
        std::fs::File::create(&d).with_context(|| format!("create {}", d.display()))?;
    // File::create truncates existing. Write via &File (Write for &File).
    let mut fout = fout;
    std::io::copy(&mut fin, &mut fout).with_context(|| format!("copy {} -> {}", s.display(), d.display()))?;
    fout.sync_all().ok();
    drop(fout);
    // preserve mtime
    if let Some(st) = smtime_sys {
        let f = std::fs::OpenOptions::new().write(true).open(&d)?;
        f.set_modified(st).with_context(|| format!("set mtime {}", d.display()))?;
    }
    // verify: size + mtime + hash
    let dmeta = std::fs::metadata(&d).with_context(|| format!("stat {}", d.display()))?;
    if dmeta.len() != ssize {
        bail!("verify size {} (want {} got {})", d.display(), ssize, dmeta.len());
    }
    if algos.is_empty() {
        // size+mtime only
        let dmtime = mtime_ns_of(&dmeta);
        if dmtime != smtime_ns {
            bail!("verify mtime {}", d.display());
        }
        return Ok(FileRec {
            kind: "file".into(),
            size: ssize,
            mtime_ns: smtime_ns,
            hashes: HashMap::new(),
        });
    }
    let sh = hash_file(&s, algos).with_context(|| format!("hash {}", s.display()))?;
    let dh = hash_file(&d, algos).with_context(|| format!("hash {}", d.display()))?;
    for a in algos {
        if sh.get(a) != dh.get(a) {
            bail!("verify hash({}) {}", a, d.display());
        }
    }
    Ok(FileRec {
        kind: "file".into(),
        size: ssize,
        mtime_ns: smtime_ns,
        hashes: dh,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insensitive_mode_adopts_disk_casing_and_survives_mode_switch() {
        // Cache created sensitive with "a.txt"; disk renames to "A.txt".
        // Insensitive run must NOT error: disk governs, cache key becomes
        // "A.txt" (hashes reused), and a later sensitive run still works.
        let dir = std::env::temp_dir()
            .join(format!("girsync_test_casefix_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        let db_path = dir.join(CACHE_PREFIX);
        let algos = vec!["md5".to_string()];

        let db = open_db(&db_path, true, false, false).unwrap();
        let eff = build_effective_folder(
            &dir, &db, &algos, true, &[], &[], true, 10, true, false,
        )
        .unwrap();
        assert!(eff.contains_key("a.txt"));
        drop(db);

        // Case-only rename via intermediate (works on case-insensitive FS too).
        let tmp = dir.join("girsync_rename_tmp");
        std::fs::rename(dir.join("a.txt"), &tmp).unwrap();
        std::fs::rename(&tmp, dir.join("A.txt")).unwrap();

        // Previously this errored in open_db (meta mismatch). Must succeed now.
        let db = open_db(&db_path, false, false, false).unwrap();
        let eff = build_effective_folder(
            &dir, &db, &algos, true, &[], &[], false, 10, false, false,
        )
        .unwrap();
        assert!(eff.contains_key("A.txt"), "disk casing governs");
        let want = format!("{:x}", md5::compute(b"hello"));
        assert_eq!(eff["A.txt"].hashes.get("md5").unwrap(), &want);
        let keys: Vec<String> = load_all_records(&db).unwrap().keys().cloned().collect();
        assert!(keys.contains(&"A.txt".to_string()), "cache key fixed to disk");
        assert!(!keys.contains(&"a.txt".to_string()), "stale casing pruned");
        drop(db);

        // Same record must remain usable in a later sensitive run.
        let db = open_db(&db_path, true, false, false).unwrap();
        let eff = build_effective_folder(
            &dir, &db, &algos, true, &[], &[], true, 10, false, false,
        )
        .unwrap();
        assert!(eff.contains_key("A.txt"));
        assert_eq!(eff["A.txt"].hashes.get("md5").unwrap(), &want);
        drop(db);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hash_arg_none_exclusive() {
        assert!(parse_hash_list(&["none".to_string()]).unwrap().is_empty());
        assert!(parse_hash_list(&["NONE".to_string()]).unwrap().is_empty());
        assert!(parse_hash_list(&["md5".to_string(), "none".to_string()]).is_err());
        assert!(parse_hash_list(&["md5".to_string()]).unwrap() == vec!["md5".to_string()]);
    }

    #[test]
    fn exclude_wins_over_include() {
        let inc = compile_patterns(&["*.dat".to_string()]).unwrap();
        let exc = compile_patterns(&["secret*".to_string()]).unwrap();
        assert!(is_excluded("other.txt", &inc, &exc, true));
        assert!(!is_excluded("a.dat", &inc, &exc, true));
        assert!(is_excluded("secret.dat", &inc, &exc, true));
    }
}
