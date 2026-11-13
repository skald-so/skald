//! MLS on a device: KeyPackages and a channel's group to the profile of
//! `docs/protocol/membership.md`, "MLS profile", and the device's MLS state, which reaches the
//! caller only sealed under a key the caller keeps in key storage; a channel's group formed from
//! KeyPackages as "Creating a channel" says, joining from its Welcome, and messages only its
//! members read. Every step loads the state the previous step sealed, as one-shot mode does.

// The acceptance tests of channel groups build with the `acceptance` feature until the code they
// test exists.
#[cfg(feature = "acceptance")]
mod channels;
mod fixtures;
#[cfg(feature = "acceptance")]
mod foreign;
#[cfg(feature = "acceptance")]
mod framing;
#[cfg(feature = "acceptance")]
mod messages;
mod profile;
mod sealed;
#[cfg(feature = "acceptance")]
mod steps;
#[cfg(feature = "acceptance")]
mod welcome;
mod wire;
