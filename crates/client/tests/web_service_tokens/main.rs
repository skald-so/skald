//! Service agent tokens in the web app's Access section, with `skald` on the machines: an admin of a
//! space sees a row per live token of the space's service agents, which a person creates with
//! `skald create-service-agent`, and revokes it after a confirmation, after which the server
//! refuses the token at its next request. A member who is no admin sees none. Against a
//! `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind the
//! recording relay and the front of the web app's harness (`server/tests/common/spa.rs`), on which
//! the machines reach it too; a test plays each person in headless Chrome. The CLI as the tests of
//! service agents use it (`service_agents/main.rs`); the web app's Access section as the tests of
//! sessions on the web use it (`server/tests/web_sessions/access.rs`).
//!
//! The web app, as these tests use it, beside those: for each space where the account is an admin,
//! Access has a row per live token of its service agents, which names the agent, as `ci` and
//! `deploy-bot`, and says the token's permissions, as `send` and `receive`, its channels, by ID, by
//! the first 8 characters of an ID, or by their number, as "1 channel", when it was created, its
//! last use and when it expires, as the days left or its date. Each such row has a button named
//! "Revoke …", which opens a dialog, role `dialog` or `alertdialog`, and revokes nothing until its
//! button whose name says "revoke" is clicked; the token's row then goes. No row of a space where
//! the account is no admin names a token, and Access shows no failure, no element of role `alert`.
//! The page `/access` opens the section. At 360 px the page does not scroll sideways.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

// The person's browser, the web app's harness, and its Access section.
#[path = "../../../server/tests/web_sessions/access.rs"]
mod access;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../../../server/tests/common/spa.rs"]
mod spa;

// The server, the relay and the machines of the tests of signing in.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

// The service agents' kit of the tests of service agents, and the people and spaces of the tests
// of roster revisions, with the device key tests' search for keys they need.
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../service_agents/kit.rs"]
mod kit;
#[path = "../roster_revisions/world.rs"]
mod world;

mod stage;

mod listed;
mod revoked;
