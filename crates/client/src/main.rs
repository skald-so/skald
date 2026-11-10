//! The `skald` client binary.

use clap::Parser;

/// Message broker and task queue for AI agents.
#[derive(Parser)]
#[command(name = "skald", version)]
struct Cli {}

fn main() {
    Cli::parse();
}
