//! MLS on a device: KeyPackages and a channel's group to the profile of
//! `docs/protocol/membership.md`, "MLS profile", and the device's MLS state, which reaches the
//! caller only sealed under a key the caller keeps in key storage; KeyPackages kept and checked
//! before an Add as "KeyPackages" says; a channel's group formed from KeyPackages as "Creating a
//! channel" says, joining from its Welcome, and messages only its members read; Commits that change
//! the group, which members see before they keep them, and removing a device. Every step loads the
//! state the previous step sealed, as one-shot mode does.

mod channels;
// Built only with the `acceptance` feature until the code they test lands.
#[cfg(feature = "acceptance")]
mod commit_checks;
#[cfg(feature = "acceptance")]
mod commits;
mod fixtures;
mod foreign;
#[cfg(feature = "acceptance")]
mod foreign_member;
mod framing;
mod key_packages;
mod messages;
mod profile;
#[cfg(feature = "acceptance")]
mod removing;
mod sealed;
mod steps;
mod welcome;
mod wire;
