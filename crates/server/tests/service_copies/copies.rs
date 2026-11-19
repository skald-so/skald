//! What the tests share: tokens of the bot with their own grants, the space's grants as a member
//! gets them, a channel outside the bot's grants, and alice's laptop sending the bot `hpke`
//! envelopes.

use skald_proto::ChannelId;

use crate::api::Person;
use crate::common::Response;
use crate::common::device_flow::{json, text};
use crate::common::devices::unb64;
use crate::tokens::{Created, DAY, Stage, bearer, call, grant};
use crate::wire::{Answer, Socket, assert_nowhere, envelope, envelopes_of, header, now};

/// The problem of an envelope to a device that cannot receive in its channel.
pub const UNKNOWN_RECIPIENT: &str = "unknown-recipient";

/// The space's grants that its members get.
pub fn grants_path(stage: &Stage) -> String {
    format!("/spaces/{}/service-grants", stage.space())
}

/// The grant of the bot for `channels`, by alice's laptop, expiring `offset` seconds after a day
/// from now, so that each grant of a test differs from the others.
pub fn grant_for(stage: &Stage, channels: &[ChannelId], offset: u64) -> Vec<u8> {
    let alice = &stage.f.team.alice;
    grant(stage.bot.id, channels, &alice.laptop, now() + DAY + offset)
}

/// A token of the bot with `grant` and `permissions`, from alice's session.
#[track_caller]
pub fn token_with(stage: &Stage, grant: &[u8], permissions: &[&str]) -> Created {
    stage.create_as(&stage.f.team.alice, stage.bot.id, grant, permissions)
}

/// Moves the expiry of the token `id` into the past in the database.
pub fn expire(stage: &Stage, id: &str) {
    stage.db().execute(&format!(
        "UPDATE service_tokens SET expires_at = now() - interval '1 second' WHERE id = '{id}'"
    ));
}

/// `GET /spaces/<acme>/service-grants` with `headers`.
pub fn grants_response(stage: &Stage, headers: &[(&str, &str)]) -> Response {
    call(stage.base(), "get", &grants_path(stage), headers, None)
}

/// The grants `person`'s device session gets, each its CBOR bytes, sorted.
#[track_caller]
pub fn grants_of(stage: &Stage, person: &Person) -> Vec<Vec<u8>> {
    let bearer = bearer(person.session.token());
    let response = grants_response(stage, &[("authorization", &bearer)]);
    assert_eq!(response.status, 200, "the grants: {}", text(&response));
    let body = json(&response);
    let mut grants: Vec<Vec<u8>> = body["grants"]
        .as_array()
        .unwrap_or_else(|| panic!("no grants in {body}"))
        .iter()
        .map(|grant| unb64(grant.as_str().unwrap_or_else(|| panic!("a grant: {grant}"))))
        .collect();
    grants.sort();
    grants
}

/// `grants`, sorted.
pub fn sorted(grants: &[&[u8]]) -> Vec<Vec<u8>> {
    let mut grants: Vec<Vec<u8>> = grants.iter().map(|grant| grant.to_vec()).collect();
    grants.sort();
    grants
}

/// Stores the next revision of `acme`, with no service agent entry.
pub fn drop_entries(stage: &mut Stage) {
    stage.f.revise(|body| body.service_agents.clear());
}

/// The WebSocket of alice's laptop.
pub fn alice(stage: &Stage) -> Socket {
    Socket::of(&stage.f.team.flow, &stage.f.team.alice)
}

/// Sends a new `hpke` envelope from alice's laptop on `socket` to the bot in `channel`; checks
/// that the server refuses it with `unknown-recipient` and holds nothing of it. `what` names the
/// case.
#[track_caller]
pub fn assert_unknown_recipient(
    stage: &Stage,
    socket: &mut Socket,
    channel: ChannelId,
    what: &str,
) {
    let header = header(stage.space(), channel, stage.bot.id);
    let sent = envelope(&header);
    socket.send(&sent);
    match socket.answer() {
        Answer::Refused(key, problem) => {
            assert_eq!(key.msg_id, header.msg_id, "{what}: the refusal's msg_id");
            assert_eq!(problem, UNKNOWN_RECIPIENT, "{what}: the problem");
        }
        Answer::Stored(stored) => panic!("{what}: stored, not refused: {stored:?}"),
    }
    assert_nowhere(stage.db(), &sent);
}

/// Sends a new `hpke` envelope from alice's laptop on `socket` to the bot in `channel`; checks
/// that the server stores it in the bot's mailbox, whose envelopes are then `before` and it. Gives
/// the bot's envelopes.
#[track_caller]
pub fn assert_stored(
    stage: &Stage,
    socket: &mut Socket,
    channel: ChannelId,
    before: &[Vec<u8>],
    what: &str,
) -> Vec<Vec<u8>> {
    let header = header(stage.space(), channel, stage.bot.id);
    let sent = envelope(&header);
    socket.send(&sent);
    let answer = socket.answer();
    assert!(
        matches!(answer, Answer::Stored(_)),
        "{what}: stored, not {answer:?}"
    );
    let mut expected = before.to_vec();
    expected.push(sent);
    assert_eq!(bot_envelopes(stage), expected, "{what}: the bot's mailbox");
    expected
}

/// The envelopes of the bot's mailbox, in order.
pub fn bot_envelopes(stage: &Stage) -> Vec<Vec<u8>> {
    envelopes_of(stage.db(), stage.bot.id)
}
