//! Linking on a local page (`docs/dev/architecture.md`, "Device linking"): the steps of a link code
//! that `skald sign-in` and `skald link` take at a terminal, taken instead on a local page that
//! `skald` opens in the browser, so that the person, not an agent that started the command, answers
//! them and sees the code, the confirmation and the recovery key. As in the tests of linking with a
//! link code (`link_codes/main.rs`): Ada's first machine is linked, her new machine signs in as no
//! first device and shows a code, and her first machine takes it; here both run as an agent, and a
//! test plays the person in headless Chrome on each page.
//!
//! The CLI, as these tests use it, beside what the tests of linking with a link code and of the
//! recovery key use:
//! - `--browser` of `skald sign-in` and of `skald link` opens the page. `skald` writes a file to
//!   its state directory, readable only by its user, and runs the program `BROWSER` names, the
//!   tests' stand-in, with one argument: that file, as its path or a `file:` URL. The file takes
//!   the browser, with `<meta http-equiv="refresh">`, to the page's URL,
//!   `http://<loopback address>:<port>/<path>?<query>`, whose query holds the page's secret. While
//!   the command runs, the page answers each request that carries the secret as that query, with
//!   the URL's own Host and the URL's own Origin or none, and refuses any other; every answer has
//!   `Cache-Control: no-store`. The file is gone once the command ends.
//! - On the page of `skald sign-in`: whether this is the account's first device, a text holding
//!   `first device` with a `Yes` and a `No` button. After `Yes`, the recovery key in its shown form
//!   and a button that is neither, which deletes the key as Enter does at a terminal; after `No`,
//!   the link code, a line holding `skald link`, a button naming a fresh, new or another code, and
//!   what each run of the code gave, as at a terminal. A waiting recovery key shows on the same
//!   page, with that button.
//! - On the page of `skald link`: the reminder, holding `own device`; a text field and its form, or
//!   the page's first button, which sends the code; then the new device's `name` and kind `client`
//!   with a `Yes` and a `No` button. After 3 failed runs in a row, it first asks, with a text
//!   holding `new code` and a `Yes` and a `No` button.
//! - Each command exits as it does at a terminal; its output holds no code, no confirmation and no
//!   recovery key. The line of a waiting recovery key also holds `--browser`.
//! - With `BROWSER` naming no program, the command says on one line that it has no browser to open
//!   and to run it at a terminal, and exits failing, sending nothing. With `BROWSER` naming a
//!   program that exits failing, it says on one line that the browser did not open and to run it at
//!   a terminal, and exits failing.
//! - As an agent, the command's output holds none of its warnings: the page shows them, and the
//!   person's next `skald recv` does.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod codes;
mod first_devices;
mod linking_through;
mod no_browser;
mod pages;
mod requests;
mod warnings;

// The modules the tests of the channel history share (`history_common/mod.rs`), the anchors objects
// of the tests of anchors, the helpers of the tests of linking with a link code, and the terminal
// and signing in there of the tests of the recovery key.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../anchors/objects.rs"]
mod anchors;
#[path = "../recovery_key/at_terminal.rs"]
mod at_terminal;
#[path = "../recovery_key/grants.rs"]
mod grants;
#[path = "../link_codes/linking.rs"]
mod linking;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
// The helpers of linking with a link code name the grants so.
use grants as terminal_grants;
