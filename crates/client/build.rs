//! Gives the `skald` binary's main thread 8 MiB of stack on Windows, as Linux and macOS give it:
//! Windows gives 1 MiB, which the CLI's debug build outgrows. Embeds the schema migrations of
//! `migrations/` for `state.rs`, and rebuilds the client when that directory changes.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=migrations");
    let target = |key: &str| env::var(key).unwrap_or_default();
    if target("CARGO_CFG_TARGET_OS") == "windows" && target("CARGO_CFG_TARGET_ENV") == "msvc" {
        println!("cargo:rustc-link-arg-bin=skald=/STACK:8388608");
    }
    migrations();
}

/// Writes `$OUT_DIR/migrations.rs`: an array of the `.sql` files of `migrations/`, sorted by name,
/// each as `include_str!` of its path.
fn migrations() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("Cargo sets CARGO_MANIFEST_DIR");
    let mut files: Vec<PathBuf> = fs::read_dir(Path::new(&manifest).join("migrations"))
        .expect("migrations/ is readable")
        .map(|entry| entry.expect("migrations/ is readable").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .collect();
    files.sort();
    let mut array = String::from("&[\n");
    for file in &files {
        let path = file.to_str().expect("a migration's path is UTF-8");
        array += &format!("    include_str!({path:?}),\n");
    }
    array += "]\n";
    let out = env::var("OUT_DIR").expect("Cargo sets OUT_DIR");
    fs::write(Path::new(&out).join("migrations.rs"), array).expect("OUT_DIR is writable");
}
