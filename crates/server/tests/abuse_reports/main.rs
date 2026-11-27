//! Report intake through the server API, with local sign-in and synthetic reason categories.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../erasure_common/mod.rs"]
mod erasure_common;
use erasure_common::*;

#[path = "../mailbox_https/https.rs"]
mod https;
#[path = "../service_tokens/tokens.rs"]
mod tokens;

mod intake;
mod reporting;
mod retention;
