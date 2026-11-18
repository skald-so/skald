//! Linking a new device with a link code (`docs/protocol/membership.md`, "Linking a device", "Link
//! code" and "Device checks"). The new device, signed in as no first device at a terminal, shows a
//! code; its person types the code into `skald link` on a linked `client` device of the account at
//! a terminal and confirms the new device's name and kind. The two run CPace through the server's
//! relay, which never sees the code; the linked device issues the new device's certificate and a
//! link object, and the new device stores its certificate, pins the master key, starts from the
//! link's anchors, pins the personal space and gets into the account's channels. The server refuses
//! a certificate over the devices limit. Against a `skald-server` with GitHub sign-in through the
//! stand-in, behind the relay of the sign-in tests, which the tests use as a proxy that sees and
//! changes what goes through. Messages, the history and played devices are as in the tests of the
//! channel history.
//!
//! The CLI, as these tests use it, besides what the tests of signing in, spaces, channels, devices
//! and the history use:
//! - `skald sign-in --server <URL> --first-device no` at a terminal, on an account with a master
//!   key: once the device is registered, it shows a code, 6 characters of the base32 alphabet in two
//!   groups of three joined by `-`, such as `K7Q-M2X`, and a line holding `skald link`, which says
//!   where to type it, then waits. Each run of the code that fails it reports on a line holding
//!   `fail`, then opens the next one with a new hello; after the third, it says on one line that
//!   the code failed 3 times and that someone else may have tried. A code ends 5 minutes after it
//!   was shown, saying so on a line holding `expired`; `SKALD_LINK_CODE_LIFETIME`, in seconds,
//!   shortens that for the tests. Enter asks for a fresh code, which ends the current one; no other
//!   fresh code is shown. Once a run succeeds it exits with success.
//! - The same through a pipe, or `--agent <name>`, shows no code and sends no hello: it says the
//!   device waits for linking, as before.
//! - `skald link`, on a device whose certificate is valid, at a terminal: it says on a line holding
//!   `own device` to enter only a code shown on the person's own device, and takes a code. Entry
//!   ignores case, spaces and `-`, and reads `0`, `1` and `8` as `O`, `I` and `B`; any other entry
//!   that is not 6 characters of the alphabet it refuses, without a run, and exits failing. It runs
//!   the entry once against the account's latest hello, shows the new device's `name` and its kind,
//!   `client`, and takes `yes` or `no`. On `no` it ends the run and exits failing; on `yes` it issues
//!   the certificate, sends the grant, and exits with success once the new device's certificate is
//!   on the server, failing once the run ends without it. After 3 failed runs in a row, its next
//!   runs first ask, on a line holding `new code`, whether the new device shows a new one, and take
//!   a code only after `yes`. Through a pipe, or with `--agent <name>`, it refuses and sends
//!   nothing.
//! - A device of the account, and a device of a member of a space with the account, warns of each
//!   device added to the account since it last checked, on a warning line holding the device's ID.
//! - A new device whose certificate the server refuses as over the devices limit ends the run, says
//!   the account has as many devices as it may, and stores the certificate at a later run.
//!
//! The server's API, as these tests use it besides that of the link code relay and the anchors:
//! `PUT /devices/{device}/cert` of `{"cert": <bytes>}`, the new device's complete certificate.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod attempts;
mod channels;
mod confirming;
mod crashes;
mod grants;
mod limit;
mod linked;
mod linking;
mod proxy;
mod refusals;
mod terminal;

// The modules the tests of the channel history share (`history_common/mod.rs`), the helpers of the
// tests of the channel history on new devices and of reading the history, and the anchors objects
// of the tests of anchors.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../anchors/objects.rs"]
mod anchors;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../history_reading/reading.rs"]
mod reading;
