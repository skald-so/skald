//! A member agent's state directory: sealed under a key derived from its key secret, so the
//! directory alone opens nothing, with nothing in the key storage; and once the agent has been in
//! a group, a run without its earlier state refuses.

use std::fs;

use serde_json::Value;

use crate::gaps::{bob_in, bob_sends};
use crate::keys;
use crate::member::{Bot, Runner, STATE, assert_not_shown, shown_once};
use crate::service_kit::{KEY_VARIABLE, TOKEN_VARIABLE};

/// What Ada, Bob and the agent say; none is anywhere else.
const ADA_SAYS: &str = "member state test: ada asks for the nightly build";
const AGENT_SAYS: &str = "member state test: deploy-bot built the nightly";
const BOB_SAYS: &str = "member state test: bob asks for a second build";
const COPY_SAYS: &str = "member state test: a copy of the state speaks";
const EMPTY_SAYS: &str = "member state test: a run without the state speaks";
const AFTER_SAYS: &str = "member state test: deploy-bot keeps going";

/// The file the client keeps its secrets in when its key storage is the file, as the tests' runs
/// set it.
const SECRETS_FILE: &str = "secrets.json";

/// Fails unless the key storage of `runner`, its state directory's secrets file, holds no secret.
#[track_caller]
fn assert_key_storage_empty(runner: &Runner) {
    let file = runner.state.join(SECRETS_FILE);
    let Ok(text) = fs::read_to_string(&file) else {
        return;
    };
    let held: Value =
        serde_json::from_str(&text).unwrap_or_else(|err| panic!("{}: {err}", file.display()));
    assert!(
        held.as_object().is_some_and(serde_json::Map::is_empty),
        "the key storage holds secrets: {}",
        held.as_object()
            .map(|secrets| secrets.keys().cloned().collect::<Vec<_>>().join(", "))
            .unwrap_or_default()
    );
}

/// Whether `bytes` hold `value` as it is.
fn holds_as_is(bytes: &[u8], value: &[u8]) -> bool {
    bytes.windows(value.len()).any(|window| window == value)
}

/// With the agent in the group, Ada's message reaches it and it says a line. No file of its state
/// directory holds its key secret, its token, its device's private keys or either text, its key
/// storage holds no secret, and on Unix only its user reaches the directory. Bob's device then says
/// a line the agent has not read. Copies of the directory on a machine with the token alone and on
/// one with neither secret fail at `skald recv` and `skald send`, show Bob's line nowhere and send
/// nothing, and Bob's device reads nothing more from the agent. A copy on a machine with both
/// secrets shows Bob's line once.
#[test]
fn the_agent_s_state_directory_opens_only_with_its_key_secret() {
    let mut bot = Bot::created(
        [3_135_730_001, 3_135_730_002],
        ["ada-member-sealed-standin", "bob-member-sealed-standin"],
    );
    bot.brought_in();
    let channel = bot.channel();
    bot.ada_reaches_the_agent(ADA_SAYS);
    bot.says(AGENT_SAYS);

    let files = bot.runner.files();
    assert!(!files.is_empty(), "the agent keeps no state");
    let plain = [
        ("key secret", bot.secrets.key.as_bytes()),
        ("token", bot.secrets.token.as_bytes()),
        ("text it read", ADA_SAYS.as_bytes()),
        ("text it said", AGENT_SAYS.as_bytes()),
    ];
    for file in &files {
        let bytes = fs::read(file).unwrap_or_else(|err| panic!("{}: {err}", file.display()));
        for (what, value) in plain {
            assert!(
                !holds_as_is(&bytes, value),
                "{} holds the {what}",
                file.display()
            );
        }
        for seed in [&bot.key.signing, &bot.key.hpke] {
            assert!(
                !keys::holds(&bytes, seed),
                "{} holds a private key of the agent's device",
                file.display()
            );
        }
    }
    assert_key_storage_empty(&bot.runner);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = (fs::metadata(&bot.runner.state))
            .expect("the state directory")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "the state directory's mode is {mode:o}");
    }

    bot.bob_takes_in();
    let (bob_from, _, bob_device) = bob_in(&bot.talk);
    bob_sends(&mut bot.talk, 1, BOB_SAYS);
    let read_before = bot.bob_read_from_the_agent().len();
    let mark = bot.talk.front.sent().len();
    let token_alone = [(TOKEN_VARIABLE, bot.secrets.token.as_str())];

    for vars in [&token_alone[..], &[]] {
        let copy = Runner::copy_of(&bot.runner, vars);

        let got = copy.recv();
        let sent = copy.send(channel, COPY_SAYS);

        assert!(
            !got.success,
            "a copy without the key secret received: {got:?}"
        );
        assert!(
            !sent.success,
            "a copy without the key secret sent: {sent:?}"
        );
        assert_not_shown(&got, BOB_SAYS);
        assert_not_shown(&sent, BOB_SAYS);
    }

    assert_eq!(
        bot.talk.front.sent().len(),
        mark,
        "copies without the key secret sent"
    );
    bot.bob_takes_in();
    assert_eq!(
        bot.bob_read_from_the_agent().len(),
        read_before,
        "Bob's device read more from the agent"
    );
    let both = [
        (KEY_VARIABLE, bot.secrets.key.as_str()),
        (TOKEN_VARIABLE, bot.secrets.token.as_str()),
    ];
    let copy = Runner::copy_of(&bot.runner, &both);
    let got = copy.received();
    shown_once(&got, BOB_SAYS, (channel, &bob_from, bob_device));
}

/// With the agent in the group, `skald recv` and `skald send` on a machine with both secrets and an
/// empty state directory fail, saying `state`, and send nothing: Bob's device gets no Commit and no
/// message from the agent's device, and still holds it in the group. The agent's run with its state
/// then sends, and Bob's device reads that message with the counter 1.
#[test]
fn a_run_without_the_agent_s_earlier_state_refuses() {
    let mut bot = Bot::created(
        [3_135_731_001, 3_135_731_002],
        ["ada-member-empty-standin", "bob-member-empty-standin"],
    );
    bot.brought_in();
    let channel = bot.channel();
    bot.bob_takes_in();
    let changes = bot.talk.played[0].changes_of(channel).len();
    let mark = bot.talk.front.sent().len();
    let empty = Runner::of(&bot.secrets);

    let got = empty.recv();
    let sent = empty.send(channel, EMPTY_SAYS);

    for ran in [&got, &sent] {
        assert!(!ran.success, "a run without the state ran: {ran:?}");
        assert!(
            ran.shown().to_lowercase().contains(STATE),
            "it does not say {STATE:?}: {ran:?}"
        );
    }
    assert_eq!(
        bot.talk.front.sent().len(),
        mark,
        "runs without the state sent"
    );
    bot.bob_takes_in();
    assert_eq!(
        bot.talk.played[0].changes_of(channel).len(),
        changes,
        "the channel's group changed after runs without the state"
    );
    assert!(
        bot.bob_read_from_the_agent().is_empty(),
        "Bob's device read a message from the agent"
    );
    assert!(bot.bob_holds_the_agent(), "the agent left the group");

    let msg_id = crate::gaps::msg_id_of(&bot.says(AFTER_SAYS));

    bot.bob_takes_in();
    let read = bot.bob_read_from_the_agent();
    let [(read_id, content)] = read.as_slice() else {
        panic!("not one message from the agent: {read:?}");
    };
    assert_eq!(*read_id, msg_id);
    assert_eq!(
        content.counter.map(std::num::NonZeroU64::get),
        Some(1),
        "{content:?}"
    );
}
