//! Gives the `skald` binary's main thread 8 MiB of stack on Windows, as Linux and macOS give it:
//! Windows gives 1 MiB, which the CLI's debug build outgrows.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target = |key: &str| std::env::var(key).unwrap_or_default();
    if target("CARGO_CFG_TARGET_OS") == "windows" && target("CARGO_CFG_TARGET_ENV") == "msvc" {
        println!("cargo:rustc-link-arg-bin=skald=/STACK:8388608");
    }
}
