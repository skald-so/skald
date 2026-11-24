//! Real Codex in a disposable home on the loopback Responses stub.

#![allow(dead_code)]

mod capture;
mod delivery_cases;
mod executor;
mod permission;
mod session;

#[path = "../codex_hosts/app_server.rs"]
mod app_server;
#[path = "../claude_code_e2e/claude.rs"]
mod claude;
#[path = "../codex_hooks/codex.rs"]
mod codex;
#[path = "../claude_code_e2e/delivery.rs"]
mod delivery;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../claude_code_hooks/hooks.rs"]
mod hooks;
#[path = "../codex_hosts/host.rs"]
mod host;
#[path = "../claude_code_e2e/stub.rs"]
mod stub;
#[path = "../recovery_key/terminal.rs"]
mod terminal;

include!("../claude_code_hooks/modules.rs");
