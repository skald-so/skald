//! The delivery frames of `docs/protocol/delivery.md`, "Frames": every frame encodes to the
//! spec's deterministic CBOR and decodes back byte for byte; the envelope in `send` and `message`
//! keeps the bytes it came in; a frame of an unknown type decodes as unknown, so its reader ignores
//! it; malformed and truncated frames are an error, never a panic; `docs/protocol/test-vectors.md`
//! publishes the hex of each frame type.

#[path = "../common/mod.rs"]
mod common;
mod fixtures;
mod misaddressed;
mod published;
mod refused;
mod round_trips;
