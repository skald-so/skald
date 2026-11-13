//! MLS on a device: KeyPackages and a channel's group to the profile of
//! `docs/protocol/membership.md`, "MLS profile", and the device's MLS state, which reaches the
//! caller only sealed under a key the caller keeps in key storage. Every step loads the state the
//! previous step sealed, as one-shot mode does.

mod fixtures;
mod profile;
mod sealed;
mod wire;
