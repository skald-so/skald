//! Signing a device key with a GitHub SSH key (`docs/protocol/membership.md`, "Device certificate"
//! and "Linking a device"; `docs/threat-model.md`, "An agent on the person's own device").
//!
//! - A `client` device that issues its own certificate, as the account's first device or with the
//!   recovery key, first makes its `ssh`: the GitHub user is the login of the account's GitHub
//!   sign-in method, or one an option names; the device takes the user's ID and SSH keys from
//!   GitHub's API and signs with a key on that list that ssh-agent holds or that is in the person's
//!   SSH directory, through OpenSSH's `ssh-keygen`. Options name the key or turn signing off.
//! - It says which key of which GitHub user signed, and that members of its spaces see that user.
//! - Without `ssh-keygen`, a GitHub user, a listed key or GitHub, the certificate goes without
//!   `ssh`, the command says why, and the sign-in goes on. The signature is written to the local
//!   state with the certificate before it is sent, so a resend keeps one body.
//! - As an agent, or with no terminal, it signs only with a key ssh-agent holds, so nothing asks
//!   for a passphrase.
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, or Google sign-in
//! through the OIDC stand-in, on PostgreSQL, behind the recording relay; a test plays the person
//! on the verification page in headless Chrome, and at a terminal in a pseudo-terminal, or a
//! pseudo-console on Windows (`recovery_key/terminal.rs`).
//!
//! GitHub's API is a GitHub stand-in of its own ([`Api`]), apart from the one the server signs in
//! with, reached through the hidden setting [`GITHUB_API`] (`github_checks/main.rs`). It gives
//! Ada's GitHub user, by the login she signs in with, and the SSH keys the tests list for it.
//!
//! The machines' `PATH` holds nothing but what a test puts there: [`with_ssh_keygen`] puts
//! OpenSSH's `ssh-keygen` of the machine running the tests there. The tests make SSH keys with that
//! `ssh-keygen` in the machine's home, `<home>/.ssh` being the person's SSH directory, and, on
//! Unix, run an ssh-agent of their own (`ssh_agent.rs`) whose socket only the machine's runs get
//! through `SSH_AUTH_SOCK`; on Windows, the machine's runs reach none: `SSH_AUTH_SOCK` names a pipe
//! nothing serves (`sign_in/machine.rs`). Nothing touches the real home, SSH directory or ssh-agent of the
//! person running the tests.
//!
//! The CLI, as these tests use it, beside `skald sign-in` (`device_keys/main.rs`,
//! `recovery_linking/main.rs`) and `skald members <space>` (`github_checks/main.rs`):
//! - `skald sign-in --ssh-key <file>` signs with the key of that private key file; `--no-ssh`
//!   signs with none; `--github <login>` takes that GitHub user in place of the account's GitHub
//!   sign-in method's;
//! - a run that signs says so on a line holding the key's SHA256 fingerprint, as `ssh-keygen -l`
//!   shows it, and the GitHub user's login as a word, and, on a line holding that login, that
//!   `members` see it;
//! - a run whose certificate goes without `ssh` because of a missing `ssh-keygen`, GitHub user,
//!   listed key or GitHub says so on a line holding `SSH key`, in any case, and the reason:
//!   `ssh-keygen` when it is missing; `GitHub` without a GitHub user and when GitHub cannot be
//!   reached or answers with an error; the GitHub user's login as a word without a listed key it
//!   can sign with;
//! - `ssh-keygen` asks for a key file's passphrase with a line holding `passphrase`, in any case.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[cfg(unix)]
mod agent;
mod passphrase;
mod recovery;
mod resend;
mod signing;
#[cfg(unix)]
mod ssh_agent;
mod ssh_kit;
mod without;

pub use ssh_kit::*;

// The modules the tests of the channel history share (`history_common/mod.rs`), and the terminal,
// signing in there and linking with the recovery key of the tests of the recovery key and of
// linking with it.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../recovery_key/at_terminal.rs"]
mod at_terminal;
#[path = "../recovery_key/grants.rs"]
mod grants;
#[path = "../recovery_linking/recovering.rs"]
mod linking;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
