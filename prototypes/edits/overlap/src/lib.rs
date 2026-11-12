//! Edit overlaps between machines, found by a server that sees only HMAC fingerprints.
//!
//! A [`Client`] fingerprints its machine's [edit set](git::edit_set), its [hunks] and its
//! [symbols] with the space key; the [`Server`] stand-in, without the key, compares the
//! fingerprints of all machines. The [`harness`] runs the scenarios on local clones of a test
//! repository. On an overlap, a client gets the other machine's changes over [`messages`], which
//! bypass the server, and checks them with `git merge-tree` ([`check`]). [`replay`] measures the
//! fingerprints on the merges of real repositories, and [`noise`] classes the files behind its
//! warnings and conflicts. Scenarios and findings: `docs/dev/edit-prototype.md`.

pub mod check;
pub mod client;
pub mod fingerprint;
pub mod git;
pub mod harness;
pub mod hunks;
pub mod messages;
pub mod noise;
pub mod replay;
pub mod server;
pub mod symbols;

pub use check::{Check, Conflict};
pub use client::{Client, Edits, Lines, Symbols};
pub use fingerprint::{Fingerprint, Scope, SpaceKey};
pub use messages::{Message, Messages};
pub use server::{MachineId, Overlap, Publication, Server};

/// An error of the prototype or of a `git` command it runs.
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T, E = Error> = std::result::Result<T, E>;
