//! MLS on a device: KeyPackages and a channel's group to the profile of
//! `docs/protocol/membership.md`, "MLS profile", and the device's MLS state, which reaches the
//! caller only sealed under a key the caller keeps in key storage. Every step loads the state the
//! previous step sealed, as one-shot mode does.

// Written before the code they test: built only with the `acceptance` feature until that code
// lands, so that the rest of the crate's tests run meanwhile.
#![cfg(feature = "acceptance")]

mod fixtures;
mod profile;
mod sealed;
mod wire;
