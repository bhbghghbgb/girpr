//! Convert a legacy sled cache directory into a redb cache file.
//!
//! Usage: `sled2redb <sled-dir> <redb-file> [--force]`
//! Exit `0` on success, `3` on failure (matches girsync's fatal code).

use std::path::PathBuf;

fn usage() -> ! {
    eprintln!("usage: sled2redb <sled-dir> <redb-file> [--force]");
    std::process::exit(3);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let sled_dir: PathBuf = match args.next() {
        Some(a) if !a.starts_with("--") => a.into(),
        _ => usage(),
    };
    let redb_path: PathBuf = match args.next() {
        Some(a) if !a.starts_with("--") => a.into(),
        _ => usage(),
    };
    let mut force = false;
    for rest in args {
        if rest == "--force" {
            force = true;
        } else {
            usage();
        }
    }
    match girsync::convert::sled_to_redb(&sled_dir, &redb_path, force) {
        Ok(c) => {
            println!(
                "converted {} -> {} files={} dirs={} hashes={}",
                sled_dir.display(),
                redb_path.display(),
                c.files,
                c.dirs,
                c.hashes
            );
        }
        Err(e) => {
            eprintln!("FATAL {:#}", e);
            std::process::exit(3);
        }
    }
}
