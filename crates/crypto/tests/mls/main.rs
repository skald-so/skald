//! MLS on a device: KeyPackages and a channel's group to the profile of
//! `docs/protocol/membership.md`, "MLS profile", and the device's MLS state, which reaches the
//! caller only sealed under a key the caller keeps in key storage; KeyPackages kept and checked
//! before an Add as "KeyPackages" says; a channel's group formed from KeyPackages as "Creating a
//! channel" says, joining from its Welcome, and messages only its members read; Commits that change
//! the group, which members see before they keep them, and removing a device; forward secrecy; a
//! channel seen from outside its group, from its GroupInfo and Commits. Every step loads the state
//! the previous step sealed, as one-shot mode does.

mod channel_view;
mod channels;
mod commit_checks;
mod commits;
mod fixtures;
mod foreign;
mod foreign_member;
mod framing;
mod group_info;
mod key_packages;
mod messages;
mod profile;
mod removing;
mod sealed;
mod secrecy;
mod steps;
mod welcome;
mod wire;
