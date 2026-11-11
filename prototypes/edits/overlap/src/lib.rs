//! Edit overlaps between machines, found by a server that sees only HMAC fingerprints.
//!
//! A [`Client`] fingerprints its machine's [edit set](git::edit_set) with the space key; the
//! [`Server`] stand-in, without the key, compares the fingerprints of all machines. The
//! [`harness`] runs the scenarios on local clones of a test repository. Scenarios and findings:
//! `docs/dev/edit-prototype.md`.

pub mod client;
pub mod fingerprint;
pub mod git;
pub mod harness;
pub mod server;

pub use client::{Client, Edits};
pub use fingerprint::{Fingerprint, Scope, SpaceKey};
pub use server::{MachineId, Overlap, Publication, Server};

/// An error of the prototype or of a `git` command it runs.
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T, E = Error> = std::result::Result<T, E>;
