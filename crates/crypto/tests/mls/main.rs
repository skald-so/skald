//! MLS on a device: KeyPackages and a channel's group to the profile of
//! `docs/protocol/membership.md`, "MLS profile", and the device's MLS state, which reaches the
//! caller only sealed under a key the caller keeps in key storage; KeyPackages kept and checked
//! before an Add as "KeyPackages" says; a channel's group formed from KeyPackages as "Creating a
//! channel" says, joining from its Welcome, and messages only its members read. Every step loads
//! the state the previous step sealed, as one-shot mode does.

mod channels;
mod fixtures;
mod foreign;
mod framing;
mod key_packages;
mod messages;
mod profile;
mod sealed;
mod steps;
mod welcome;
mod wire;
