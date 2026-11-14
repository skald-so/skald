//! Stateless HPKE, natively and in WASM: a single-shot HPKE seal and open in base mode of the
//! suite of `docs/protocol/README.md`, "Scheme `hpke`", with any `info` and `aad`, and the `hpke`
//! payload of that section on top of it: sealed to one device, signed by the sending device, opened
//! with checks 1 and 2.

#[path = "../common/mod.rs"]
mod common;
mod fixtures;
mod messages;
mod single_shot;
