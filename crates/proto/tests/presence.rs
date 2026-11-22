//! Presence values: `docs/protocol/agents.md`, "Presence". The acceptance tests of presence are in
//! `crates/crypto/tests/presence/`.

mod common;

use common::{alice, alice_device, channel, hex, to_hex};
use skald_proto::{
    AgentState, AgentStatus, Cbor, Presence, PresenceBody, QueueId, ServedQueue, Tier, Tiers,
    Unknown,
};

const EXPIRES: u64 = 1_800_000_900;
const SINCE: u64 = 1_800_000_000;

fn queue() -> QueueId {
    "019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5eb".parse().unwrap()
}

/// A body with an agent in `state` with a note and a runtime, and a queue with `tier` for each
/// trust level.
fn body(state: AgentState, tier: Tier) -> PresenceBody {
    PresenceBody {
        device: alice_device(),
        channel: channel(),
        epoch: 2,
        revision: 3,
        expires: EXPIRES,
        agents: vec![AgentStatus {
            agent: alice(),
            state,
            since: SINCE,
            note: Some("reviewing PR 12".to_owned()),
            runtime: Some("claude-code".parse().unwrap()),
            unknown: Unknown::default(),
        }],
        serves: vec![ServedQueue {
            queue: queue(),
            tiers: Tiers {
                own: tier,
                peer: tier,
                service: tier,
            },
        }],
        unknown: Unknown::default(),
    }
}

/// The encoded [`body`], `state` and `tier` as their codes.
fn body_hex(state: u8, tier: u8) -> Vec<u8> {
    hex(&format!(
        "a7 0150{device} 0250{channel} 0302 0403 051a{EXPIRES:08x}
         0681 a5 0178{agent_len:02x}{agent} 02{state:02x} 031a{SINCE:08x}
              046f{note} 056b{runtime}
         0781 82 50{queue} 83{tier:02x}{tier:02x}{tier:02x}",
        device = to_hex(alice_device().as_bytes()),
        channel = to_hex(channel().as_bytes()),
        agent_len = alice().to_string().len(),
        agent = to_hex(alice().to_string().as_bytes()),
        note = to_hex(b"reviewing PR 12"),
        runtime = to_hex(b"claude-code"),
        queue = to_hex(queue().as_bytes()),
    ))
}

#[test]
fn a_body_is_its_map_with_each_state_and_tier_as_its_code() {
    for (code, state, tier) in [
        (1, AgentState::Available, Tier::Off),
        (2, AgentState::Busy, Tier::ReadOnly),
        (3, AgentState::Waiting, Tier::Approve),
        (4, AgentState::Offline, Tier::Full),
    ] {
        assert_eq!(body(state, tier).to_cbor(), body_hex(code, code), "{code}");
        assert_eq!(
            PresenceBody::from_cbor(&body_hex(code, code)),
            Ok(body(state, tier)),
            "{code}"
        );
        assert_eq!(state.code(), code);
        assert_eq!(AgentState::of_code(u64::from(code)), Some(state));
    }
    assert_eq!(AgentState::of_code(0), None);
    assert_eq!(AgentState::of_code(5), None);
}

#[test]
fn a_value_is_its_map_and_its_associated_data_the_label_device_channel_and_epoch() {
    let value = Presence {
        device: alice_device(),
        channel: channel(),
        epoch: 2,
        nonce: [0x6e; 12],
        ct: vec![0xc7; 3],
        unknown: Unknown::default(),
    };
    let device = to_hex(alice_device().as_bytes());
    let channel_hex = to_hex(channel().as_bytes());
    let encoded = hex(&format!(
        "a5 0150{device} 0250{channel_hex} 0302 044c{nonce} 0543c7c7c7",
        nonce = "6e".repeat(12),
    ));
    assert_eq!(value.to_cbor(), encoded);
    assert_eq!(Presence::from_cbor(&encoded), Ok(value));
    assert_eq!(
        Presence::associated_data(alice_device(), channel(), 2),
        hex(&format!(
            "84 6e{} 50{device} 50{channel_hex} 02",
            to_hex(b"skald presence")
        ))
    );
}
