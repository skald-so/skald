//! Channels: `skald mcp` declares Claude Code's channel capability and pushes each of its agent's
//! messages as one channel event, none while nothing waits: `own` text as its line, a message
//! below `own` only as the signal, whose `inbox` call then gives its text; within the hooks' limit.

use std::fs::OpenOptions;
use std::io::Write;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use skald_proto::MessageId;

use crate::client::Session;

use crate::cloud::WAIT;
use crate::fallbacks::{
    Bare, CHANNEL_CAPABILITY, QUIET, RESEND_SETTING, STATELESS, events, long_text, wait_events,
};
use crate::hooks::{
    Bob, CLAUDE_CODE, Event, HOOK, LIMIT, assert_cut, assert_own_line, assert_signal,
};
use crate::machine::Run;
use crate::receipts::{shows_no_read, shows_read_once};
use crate::solo::{API, WEB};
use crate::world::AGENT;

/// A resend timeout longer than any test waits: each message goes by its first path only.
const LONG: &str = "600";

/// web's `skald mcp` starts, as Claude Code starts it, and is initialized: its `initialize` result
/// declares the capability `claude/channel` under `experimental`.
#[test]
fn skald_mcp_declares_the_channel_capability() {
    let ada = Bare::new(4_919_501_001, "ada-channel-capability");
    let session = ada.channel(WEB, &[]);

    let capabilities = &session.initialized()["capabilities"];
    assert!(
        capabilities["experimental"][CHANNEL_CAPABILITY].is_object(),
        "no {CHANNEL_CAPABILITY}: {capabilities}"
    );
}

/// web's `skald mcp` starts, as Claude Code starts it, and Claude Code's `server/discover` of the
/// revision without `initialize`, 2026-07-28, reaches it: the result offers only revisions with
/// `initialize`, as Claude Code takes channel events only on such a session, and declares the
/// capability `claude/channel`.
#[test]
fn skald_mcp_offers_claude_code_only_revisions_with_initialize() {
    let ada = Bare::new(4_919_501_002, "ada-channel-revisions");
    let session = ada.discovered(WEB);

    let discovered = session.initialized();
    let versions = discovered["supportedVersions"]
        .as_array()
        .expect("versions");
    assert!(!versions.is_empty(), "no versions: {discovered}");
    assert!(
        !versions.iter().any(|version| version == STATELESS),
        "{STATELESS} offered: {discovered}"
    );
    assert!(
        discovered["capabilities"]["experimental"][CHANNEL_CAPABILITY].is_object(),
        "no {CHANNEL_CAPABILITY}: {discovered}"
    );
}

/// What api says to web; it is nowhere else.
const OWN_SAYS: &str = "channel test: api asks web to tag the release";

/// Sends the message from api and checks its first channel event.
fn own_event(ada: &Bare, session: &mut Session) -> (MessageId, String) {
    let msg_id = ada.solo.api_sends_web(OWN_SAYS);
    let first = wait_events(session, 1, WAIT).remove(0);
    assert_own_line(&first, OWN_SAYS, msg_id, &ada.solo.short(API));
    (msg_id, first)
}

/// web's `skald mcp` runs, no other path of web's: over [`QUIET`] it sends no channel event. api
/// sends web a message: one channel event follows, its text the message's line from api at `own`;
/// over [`QUIET`] more, no other event.
#[test]
fn an_own_message_goes_as_one_channel_event_and_none_while_nothing_waits() {
    let ada = Bare::new(4_919_502_001, "ada-channel-own-message");
    let mut session = ada.channel(WEB, &[(RESEND_SETTING, LONG)]);

    let idle = events(&mut session, 1, QUIET);
    assert!(idle.is_empty(), "events with nothing waiting: {idle:#?}");
    own_event(&ada, &mut session);
    let later = events(&mut session, 2, QUIET);
    assert_eq!(later.len(), 1, "more than one event: {later:#?}");
}

#[test]
fn one_stop_confirms_an_own_channel_message_after_the_assistant_entry_is_written() {
    let ada = Bare::new(4_919_505_001, "ada-channel-late-assistant");
    let mut session = ada.channel(WEB, &[(RESEND_SETTING, LONG)]);
    let (msg_id, event) = own_event(&ada, &mut session);

    let dir = ada.solo.dir(WEB);
    let input = Event::Stop.input(&dir);
    let parsed: Value = serde_json::from_str(&input).expect("Stop input");
    let mut file = OpenOptions::new()
        .create_new(true)
        .append(true)
        .open(parsed["transcript_path"].as_str().expect("transcript path"))
        .expect("create the transcript");
    let session_id = &parsed["session_id"];
    let channel_entry = json!({
        "type": "user",
        "sessionId": session_id,
        "timestamp": jiff::Timestamp::now().to_string(),
        "message": { "role": "user", "content": format!("<channel source=\"skald\">\n{event}\n</channel>") },
    });
    let reply = json!({
        "type": "assistant",
        "sessionId": session_id,
        "message": { "role": "assistant", "content": [{ "type": "text", "text": "Done." }] },
    })
    .to_string();
    let (incomplete, remainder) = reply.split_at(reply.len() - 1);
    write!(file, "{channel_entry}\n{incomplete}").expect("write the transcript before Stop");

    let mut command = ada
        .helped
        .command_in(&dir, &[], &[AGENT, WEB, HOOK, CLAUDE_CODE]);
    let mut stop = Run::spawn(command.stdin(Stdio::piped()));
    stop.answer(&input);
    // Claude can flush the final entry while Stop is still running or just after it returns.
    let flush_at = Instant::now() + Duration::from_millis(300);
    while !stop.exited() && Instant::now() < flush_at {
        thread::sleep(Duration::from_millis(10));
    }
    shows_no_read(ada.machine(), msg_id, "before the assistant's reply");
    writeln!(file, "{remainder}").expect("complete the assistant's reply");

    let stopped = stop.finish();
    assert!(
        stopped.success && stopped.stdout.is_empty() && stopped.stderr.is_empty(),
        "Stop: {stopped:?}"
    );
    shows_read_once(ada.machine(), msg_id);
}

/// What review says to web; it is nowhere else.
const PEER_SAYS: &str = "channel test: review asks web for the build log";

/// Bob joins `clienta`; web's `skald mcp` runs. review sends web a message from Bob's machine: one
/// channel event follows, the signal alone, with neither review's text, Bob, his device nor the
/// message's reference. web's `inbox` then shows review's text, and Bob's `skald recv` shows `read`
/// of it once.
#[test]
fn a_peer_s_message_goes_as_a_channel_event_of_the_signal_alone() {
    let ada = Bare::new(4_919_503_001, "ada-channel-peer-message");
    let bob = Bob::join(&ada.solo, 4_919_503_002, "bob-channel-peer-message");
    let mut session = ada.channel(WEB, &[(RESEND_SETTING, LONG)]);

    let msg_id = bob.sends(&ada.solo, WEB, PEER_SAYS);

    let sent = wait_events(&mut session, 1, WAIT);
    assert_signal(&sent[0], msg_id, PEER_SAYS, &bob);
    let shown = session.inbox();
    assert!(shown.contains(PEER_SAYS), "inbox: {shown}");
    shows_read_once(&bob.person.machine, msg_id);
    let later = events(&mut session, 2, QUIET);
    assert_eq!(later.len(), 1, "more than one event: {later:#?}");
}

/// What starts api's long message; it is nowhere else.
const LONG_HEAD: &str = "channel test: api's report for web, 12000 characters:";

/// web's `skald mcp` runs. api sends web a message of 12,000 characters: its channel event has at
/// most 10,000 characters, without the whole text, and as its last line one that names `inbox`
/// and the message's `#<ref>`.
#[test]
fn a_message_over_the_limit_goes_cut_with_how_to_read_it_whole() {
    let ada = Bare::new(4_919_504_001, "ada-channel-long-message");
    let mut session = ada.channel(WEB, &[(RESEND_SETTING, LONG)]);
    let long = long_text(LONG_HEAD, 12_000);

    let msg_id = ada.solo.api_sends_web(&long);

    let sent = wait_events(&mut session, 1, WAIT);
    let length = sent[0].chars().count();
    assert!(length <= LIMIT, "{length} characters, over the limit");
    assert_cut(&sent[0], &long, msg_id);
    assert!(
        sent[0].contains(&long[..1_000]),
        "not the long message's: {}",
        sent[0]
    );
}
