//! The presence types of `skald-proto`: a value, its signed body, agent statuses and served queues,
//! encoded as `docs/protocol/agents.md`, "Presence", and `README.md`, "Encoding", say. They keep
//! fields of later versions and the signed body's bytes; malformed and truncated input is refused,
//! never a panic.

use skald_proto::{
    Address, AgentState, Cbor, ChannelId, Presence, PresenceBody, QueueId, SignedBody, Tier,
};

use crate::cbor::{Value, bytes, text};
use crate::devices::device_id;
use crate::fields::{
    assert_signed_round_trip, signed_cbor, truncated_and_longer, unknown, with, without,
};
use crate::fixtures::*;

fn address(text: &str) -> Address {
    text.parse().expect("an address")
}

/// Alice's body with every optional field and fields of later versions, in the body and in an
/// agent status.
fn rich() -> Body {
    let mut body = Body::of(ALICE, ChannelId::generate());
    body.agents[0].extend(unknown());
    body.unknown = unknown();
    body
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_presence_body_decodes_to_its_fields_and_encodes_back_byte_for_byte() {
    let queue_id = QueueId::generate();
    let body = Body {
        serves: vec![
            queue(queue_id, [FULL, READ_ONLY, OFF]),
            queue(QueueId::generate(), [OFF, APPROVE, FULL]),
        ],
        ..rich()
    };
    let encoded = body.cbor();
    let decoded = PresenceBody::from_cbor(&encoded).expect("the body decodes");

    assert_eq!(
        (
            decoded.device,
            decoded.channel,
            decoded.epoch,
            decoded.revision,
            decoded.expires
        ),
        (device_id(ALICE), body.channel, 1, 7, EXPIRES)
    );
    assert_eq!(decoded.agents.len(), 2);
    let working = &decoded.agents[0];
    assert_eq!(working.agent, address(AGENT));
    assert_eq!(working.state, AgentState::Busy);
    assert_eq!(working.since, SINCE);
    assert_eq!(working.note.as_deref(), Some("reviewing PR 12"));
    assert_eq!(
        working.runtime.as_ref().map(ToString::to_string).as_deref(),
        Some("claude-code")
    );
    let idle = &decoded.agents[1];
    assert_eq!(
        (&idle.agent, &idle.state, idle.since),
        (&address(OTHER_AGENT), &AgentState::Offline, SINCE)
    );
    assert_eq!(idle.note, None);
    assert!(idle.runtime.is_none());

    assert_eq!(decoded.serves.len(), 2);
    assert_eq!(decoded.serves[0].queue, queue_id);
    let tiers = &decoded.serves[0].tiers;
    assert_eq!(
        (&tiers.own, &tiers.peer, &tiers.service),
        (&Tier::Full, &Tier::ReadOnly, &Tier::Off)
    );
    let tiers = &decoded.serves[1].tiers;
    assert_eq!(
        (&tiers.own, &tiers.peer, &tiers.service),
        (&Tier::Off, &Tier::Approve, &Tier::Full)
    );

    assert_eq!(
        decoded.to_cbor(),
        encoded,
        "byte for byte, unknown fields included"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn each_agent_state_and_tier_decodes_to_its_own_value() {
    let channel = ChannelId::generate();
    for (code, state) in [
        (AVAILABLE, AgentState::Available),
        (BUSY, AgentState::Busy),
        (WAITING, AgentState::Waiting),
        (OFFLINE, AgentState::Offline),
    ] {
        let body = Body {
            agents: vec![status(AGENT, code)],
            ..Body::of(ALICE, channel)
        };
        let decoded = body.decoded();
        assert_eq!(decoded.agents[0].state, state, "state {code}");
        assert_eq!(
            decoded.to_cbor(),
            body.cbor(),
            "state {code}, byte for byte"
        );
    }
    for (code, tier) in [
        (OFF, Tier::Off),
        (READ_ONLY, Tier::ReadOnly),
        (APPROVE, Tier::Approve),
        (FULL, Tier::Full),
    ] {
        let body = Body {
            serves: vec![queue(QueueId::generate(), [code, code, code])],
            ..Body::of(ALICE, channel)
        };
        let decoded = body.decoded();
        let tiers = &decoded.serves[0].tiers;
        assert_eq!(
            (&tiers.own, &tiers.peer, &tiers.service),
            (&tier, &tier, &tier),
            "tier {code}"
        );
        assert_eq!(decoded.to_cbor(), body.cbor(), "tier {code}, byte for byte");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_body_of_a_device_with_no_agents_and_no_queues_round_trips() {
    let body = Body {
        agents: vec![],
        serves: vec![],
        ..Body::of(ALICE, ChannelId::generate())
    };
    let decoded = body.decoded();
    assert!(decoded.agents.is_empty());
    assert!(decoded.serves.is_empty());
    assert_eq!(decoded.to_cbor(), body.cbor());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_presence_plaintext_keeps_its_body_bytes_and_signature_under_the_presence_label() {
    assert_eq!(<PresenceBody as SignedBody>::LABEL, LABEL);
    let body = rich();
    assert_signed_round_trip::<PresenceBody>(&body.cbor(), &body.decoded());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_presence_value_decodes_to_its_fields_and_round_trips_with_unknown_fields() {
    let channel = ChannelId::generate();
    let ct = vec![0xc7; BLOCK + 16];
    let encoded = value_cbor(device_id(BOB), channel, 3, &[0x5e; 12], &ct, unknown());
    let value = Presence::from_cbor(&encoded).expect("the value decodes");
    assert_eq!(
        (value.device, value.channel, value.epoch, value.nonce),
        (device_id(BOB), channel, 3, [0x5e; 12])
    );
    assert_eq!(value.ct, ct);
    assert_eq!(
        value.to_cbor(),
        encoded,
        "byte for byte, unknown fields included"
    );
}

/// Checks that `body`, described as `what`, decodes neither alone nor as a signed plaintext.
fn assert_body_refused(what: &str, body: &[u8]) {
    assert!(
        PresenceBody::from_cbor(body).is_err(),
        "body refused: {what}"
    );
    assert!(
        skald_proto::Signed::<PresenceBody>::from_cbor(&signed_cbor(body, &[0x5a; 64])).is_err(),
        "plaintext refused: {what}"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_presence_bodies_are_refused() {
    let body = Body::of(ALICE, ChannelId::generate());
    let fields = body.fields();
    for key in 1..=7 {
        assert_body_refused(&format!("without field {key}"), &without(&fields, key));
    }
    for (what, key, value) in [
        ("epoch 0", 3, Value::Uint(0)),
        ("revision 0", 4, Value::Uint(0)),
        ("a device of 15 bytes", 1, bytes(&[1; 15])),
        ("a channel of 17 bytes", 2, bytes(&[1; 17])),
        ("expires as text", 5, text("soon")),
        ("agents not an array", 6, Value::Map(status(AGENT, BUSY))),
        ("serves not an array", 7, Value::Uint(1)),
    ] {
        assert_body_refused(what, &with(&fields, key, value));
    }

    let mut statuses: Vec<(String, Vec<(i64, Value)>)> = Vec::new();
    for key in 1..=3 {
        let status = status(AGENT, BUSY)
            .into_iter()
            .filter(|(k, _)| *k != key)
            .collect();
        statuses.push((format!("an agent status without field {key}"), status));
    }
    for (what, key, value) in [
        ("state 0", 2, Value::Uint(0)),
        ("state 5", 2, Value::Uint(5)),
        ("an agent that is no address", 1, text("claude-api")),
        ("an empty note", 4, text("")),
        ("a note of 129 bytes", 4, text(&"n".repeat(129))),
        ("a runtime that is no text", 5, bytes(b"claude-code")),
    ] {
        let mut status: Vec<_> = status(AGENT, BUSY)
            .into_iter()
            .filter(|(k, _)| *k != key)
            .collect();
        status.push((key, value));
        statuses.push((what.to_owned(), status));
    }
    for (what, status) in statuses {
        let changed = Body {
            agents: vec![status],
            ..body.clone()
        };
        assert_body_refused(&what, &changed.cbor());
    }

    let id = QueueId::generate();
    for (what, served) in [
        ("tier 0", queue(id, [0, FULL, FULL])),
        ("tier 5", queue(id, [FULL, FULL, 5])),
        (
            "two tiers",
            Value::Array(vec![
                bytes(id.as_bytes()),
                Value::Array(vec![Value::Uint(FULL); 2]),
            ]),
        ),
        (
            "four tiers",
            Value::Array(vec![
                bytes(id.as_bytes()),
                Value::Array(vec![Value::Uint(FULL); 4]),
            ]),
        ),
        (
            "a queue of 15 bytes",
            Value::Array(vec![
                bytes(&[1; 15]),
                Value::Array(vec![Value::Uint(FULL); 3]),
            ]),
        ),
        (
            "a queue without tiers",
            Value::Array(vec![bytes(id.as_bytes())]),
        ),
    ] {
        let changed = Body {
            serves: vec![served],
            ..body.clone()
        };
        assert_body_refused(what, &changed.cbor());
    }

    let longest = Body {
        agents: vec![status_with(AGENT, BUSY, &"n".repeat(128), "claude-code")],
        ..body.clone()
    };
    assert_eq!(
        longest.decoded().agents[0].note.as_deref().map(str::len),
        Some(128),
        "a note of 128 bytes decodes"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_presence_values_are_refused() {
    let fields = vec![
        (1, bytes(device_id(ALICE).as_bytes())),
        (2, bytes(ChannelId::generate().as_bytes())),
        (3, Value::Uint(1)),
        (4, bytes(&[0x5e; 12])),
        (5, bytes(&[0xc7; BLOCK + 16])),
    ];
    assert!(
        Presence::from_cbor(&Value::Map(fields.clone()).encode()).is_ok(),
        "the value as made decodes"
    );
    for key in 1..=5 {
        assert!(
            Presence::from_cbor(&without(&fields, key)).is_err(),
            "refused: without field {key}"
        );
    }
    for (what, key, value) in [
        ("epoch 0", 3, Value::Uint(0)),
        ("a nonce of 11 bytes", 4, bytes(&[0x5e; 11])),
        ("a nonce of 13 bytes", 4, bytes(&[0x5e; 13])),
        ("a device of 15 bytes", 1, bytes(&[1; 15])),
        ("ct as text", 5, text("ciphertext")),
    ] {
        assert!(
            Presence::from_cbor(&with(&fields, key, value)).is_err(),
            "refused: {what}"
        );
    }
    assert!(
        Presence::from_cbor(&Value::Array(vec![]).encode()).is_err(),
        "refused: not a map"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn truncated_values_bodies_and_plaintexts_are_refused_without_a_panic() {
    let body = rich();
    let value = value_cbor(
        device_id(ALICE),
        body.channel,
        1,
        &[0x5e; 12],
        &[0xc7; 40],
        unknown(),
    );
    for input in truncated_and_longer(&value) {
        assert!(
            Presence::from_cbor(&input).is_err(),
            "value refused: {input:02x?}"
        );
    }
    for input in truncated_and_longer(&body.cbor()) {
        assert!(
            PresenceBody::from_cbor(&input).is_err(),
            "body refused: {input:02x?}"
        );
    }
    for input in truncated_and_longer(&body.signed(ALICE, LABEL)) {
        assert!(
            skald_proto::Signed::<PresenceBody>::from_cbor(&input).is_err(),
            "plaintext refused: {input:02x?}"
        );
    }
}
