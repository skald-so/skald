//! A kit entry, run by hand and not by `script/test`: a real agent CLI's session on this machine
//! gets a message through this build's MCP server.
//!
//! `cargo test -p skald-client --test local_agents -- --ignored --nocapture kit_serves`, with
//! `SKALD_TEST_DATABASE_URL` as for the other tests, starts the server and the GitHub stand-in as
//! the tests do and signs a machine's `skald` in as Ada, who creates `clienta`, bound to
//! `~/work/clientA` there. In that folder's `<agent>` it writes the project config of Claude Code,
//! `.mcp.json`, that starts this build's `skald --agent <agent> mcp` as that machine;
//! `SKALD_KIT_AGENT` names the agent, `claude` by default. It says the folder and waits, with the
//! server running, until stopped. Once a session there registers its agent, the person's
//! `skald send --to <owner>/<agent>` sends it a message, then each line typed into the entry's
//! input; the entry says what each send said.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufRead};
use std::thread;
use std::time::Duration;

use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};

use crate::client::MCP;
use crate::solo::{SEND, Solo, TO};
use crate::world::AGENT;

/// The agent the session runs as, unless `SKALD_KIT_AGENT` names another.
const DEFAULT_AGENT: &str = "claude";

/// The text the person sends the session's agent.
const TEXT: &str = "kit: a message from the person to the session's agent";

/// How often the entry looks for the session's registration.
const POLL: Duration = Duration::from_millis(500);

#[test]
#[ignore = "a kit entry, run by hand: it waits until stopped"]
fn kit_serves_a_session_until_stopped() {
    let agent = std::env::var("SKALD_KIT_AGENT").unwrap_or_else(|_| DEFAULT_AGENT.to_owned());
    let solo = Solo::new(4_657_099_001, "ada-runs-a-real-session");
    let dir = solo.dir(&agent);
    let command = solo.machine().command_in(&dir, &[AGENT, &agent, MCP]);
    let text = |value: &OsStr| value.to_string_lossy().into_owned();
    let env: Map<String, Value> = (command.get_envs())
        .filter_map(|(name, value)| Some((text(name), json!(text(value?)))))
        .collect();
    let server = json!({
        "command": text(command.get_program()),
        "args": command.get_args().map(text).collect::<Vec<_>>(),
        "env": env,
    });
    let config = json!({ "mcpServers": { "skald": server } });
    let written = serde_json::to_string_pretty(&config).expect("a config encodes");
    fs::write(dir.join(".mcp.json"), written).expect("write the project config");
    println!(
        "kit: start the session in {}; the server runs until stopped",
        dir.display()
    );

    let state = Connection::open(solo.machine().state_dir().join("state.sqlite3"))
        .expect("open the local state");
    while !registered(&state, &agent) {
        thread::sleep(POLL);
    }
    let to = solo.short(&agent);
    let send = |text: &str| {
        let sent = solo.person_runs(&[SEND, TO, &to, text]);
        println!("kit: the person's send to {to}: {sent:?}");
    };
    send(TEXT);
    println!("kit: type a line to send it to {to}");
    for line in io::stdin().lock().lines() {
        send(&line.expect("read the input"));
    }
    loop {
        thread::park();
    }
}

/// Whether the local state `state` holds the agent `agent` registered.
fn registered(state: &Connection, agent: &str) -> bool {
    state
        .query_row(
            "SELECT 1 FROM agent WHERE name = ?1",
            params![agent],
            |_| Ok(()),
        )
        .is_ok()
}
