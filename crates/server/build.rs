//! Rebuilds the server when `migrations/` changes: `sqlx::migrate!` embeds it.

fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
