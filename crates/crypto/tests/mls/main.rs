//! MLS on a device: KeyPackages and a channel's group to the profile of
//! `docs/protocol/membership.md`, "MLS profile", and the device's MLS state, which reaches the
//! caller only sealed under a key the caller keeps in key storage; KeyPackages kept and checked
//! before an Add as "KeyPackages" says; a channel's group formed from KeyPackages as "Creating a
//! channel" says, joining from its Welcome, and messages only its members read; Commits that change
//! the group, which members see before they keep them, and removing a device; forward secrecy; a
//! channel seen from outside its group, from its GroupInfo and Commits; the history key chain and
//! signed message copies under the history keys. Every step loads the state the previous step
//! sealed, as one-shot mode does.

// The acceptance tests of the history key chain and entries, which fail until the code they test
// lands; the implementation removes the gates with the `acceptance` feature.
#[cfg(feature = "acceptance")]
#[path = "../rosters/cbor.rs"]
mod cbor;
mod channel_view;
mod channels;
mod commit_checks;
mod commits;
mod fixtures;
mod foreign;
mod foreign_member;
mod framing;
mod group_info;
#[cfg(feature = "acceptance")]
mod history;
#[cfg(feature = "acceptance")]
mod history_entries;
#[cfg(feature = "acceptance")]
mod history_objects;
#[cfg(feature = "acceptance")]
mod key_chain;
mod key_packages;
mod messages;
mod profile;
mod removing;
mod sealed;
mod secrecy;
mod steps;
mod welcome;
mod wire;
