//! Messages in a channel's group: `docs/protocol/membership.md`, "MLS profile". Application
//! messages are PrivateMessages that only the group's members read, each giving its sender's
//! device ID from its credential; Commits are PublicMessages.

use skald_proto::ChannelId;

use crate::fixtures::{device_id, load};
use crate::framing::{self, APPLICATION, COMMIT, PRIVATE_MESSAGE, PUBLIC_MESSAGE};
use crate::steps::{add, channel_of, creator, receive, send, step, stored, with_key_packages};

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn each_member_decrypts_each_message_and_gets_its_sender() {
    let channel = ChannelId::generate();
    let seeds = [71, 72, 73];
    let mut states = channel_of(channel, seeds[0], &seeds[1..]);
    for round in 0..2 {
        for at in 0..seeds.len() {
            let plaintext = format!("message {round} from device {}", seeds[at]).into_bytes();
            let message = send(seeds[at], &mut states[at], channel, &plaintext);
            for other in (0..seeds.len()).filter(|&other| other != at) {
                assert_eq!(
                    receive(seeds[other], &mut states[other], channel, &message),
                    Some((device_id(seeds[at]), plaintext.clone())),
                    "device {} reads device {}",
                    seeds[other],
                    seeds[at]
                );
            }
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_that_was_not_added_decrypts_nothing() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 74, &[75]);
    let first = send(74, &mut states[0], channel, b"from the creator");
    let second = send(75, &mut states[1], channel, b"from the added device");

    // One device has only a KeyPackage; another formed a group of the same channel of its own.
    let (mut outsider, _) = with_key_packages(76, 1);
    let mut own_group = creator(77, channel);
    for message in [&first, &second] {
        assert_eq!(receive(76, &mut outsider, channel, message), None);
        assert_eq!(receive(77, &mut own_group, channel, message), None);
    }
    assert!(
        step(76, &mut outsider, |device| device
            .encrypt(channel, b"hello"))
        .is_err(),
        "no group to encrypt in"
    );
    assert_eq!(
        receive(75, &mut states[1], channel, &first),
        Some((device_id(74), b"from the creator".to_vec())),
        "the members still read it"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_replayed_message_is_refused() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 78, &[79, 80]);
    let message = send(78, &mut states[0], channel, b"once");
    let read = Some((device_id(78), b"once".to_vec()));

    assert_eq!(receive(79, &mut states[1], channel, &message), read);
    assert_eq!(
        receive(79, &mut states[1], channel, &message),
        None,
        "replayed after a reload"
    );
    assert_eq!(
        receive(80, &mut states[2], channel, &message),
        read,
        "another member reads it once"
    );
    let next = send(78, &mut states[0], channel, b"next");
    assert_eq!(
        receive(79, &mut states[1], channel, &next),
        Some((device_id(78), b"next".to_vec())),
        "the refused replay changed nothing"
    );
    assert_eq!(receive(79, &mut states[1], channel, &message), None);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_message_of_another_channels_group_is_refused() {
    let (channel, other) = (ChannelId::generate(), ChannelId::generate());
    let (mut member, packages) = with_key_packages(82, 2);
    let mut sender = creator(81, channel);
    step(81, &mut sender, |device| {
        device.create_group(other).expect("a second group");
    });
    let mut welcomes = Vec::new();
    for (group, package) in [(channel, &packages[0]), (other, &packages[1])] {
        add(81, &mut sender, group, &[package.as_slice()]);
        welcomes.push(stored(81, &mut sender, group));
    }
    for welcome in &welcomes {
        step(82, &mut member, |device| {
            device.join(welcome, |_| true).expect("the device joins");
        });
    }

    let message = send(81, &mut sender, other, b"for the other channel");
    assert_eq!(
        receive(82, &mut member, channel, &message),
        None,
        "refused as a message of this channel"
    );
    assert_eq!(
        receive(82, &mut member, other, &message),
        Some((device_id(81), b"for the other channel".to_vec())),
        "read in its own channel"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn application_messages_are_private_messages_and_commits_public_messages() {
    let channel = ChannelId::generate();
    let (mut joiner, packages) = with_key_packages(84, 1);
    let mut state = creator(83, channel);

    let commit = add(83, &mut state, channel, &[packages[0].as_slice()]);
    let framed = framing::framing(&commit);
    assert_eq!(
        framed.wire_format, PUBLIC_MESSAGE,
        "a Commit is a PublicMessage"
    );
    assert_eq!(framed.content_type, COMMIT);
    assert_eq!(
        framed.group_id,
        channel.as_bytes(),
        "the group ID is the channel ID"
    );
    assert_eq!(framed.epoch, 0);

    let welcome = stored(83, &mut state, channel);
    step(84, &mut joiner, |device| {
        device.join(&welcome, |_| true).expect("the device joins");
    });
    let plaintext = b"a plaintext the server never sees";
    let message = send(83, &mut state, channel, plaintext);
    let framed = framing::framing(&message);
    assert_eq!(
        framed.wire_format, PRIVATE_MESSAGE,
        "an application message is a PrivateMessage"
    );
    assert_eq!(framed.content_type, APPLICATION);
    assert_eq!(framed.group_id, channel.as_bytes());
    assert_eq!(framed.epoch, 1);
    assert!(
        !message
            .windows(plaintext.len())
            .any(|window| window == plaintext),
        "the plaintext is encrypted"
    );
    assert_eq!(
        receive(84, &mut joiner, channel, &message),
        Some((device_id(83), plaintext.to_vec()))
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_malformed_or_truncated_message_is_refused_without_a_panic_and_changes_nothing() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 85, &[86]);
    let message = send(85, &mut states[0], channel, b"intact");

    let mut inputs: Vec<Vec<u8>> = (0..message.len())
        .map(|len| message[..len].to_vec())
        .collect();
    for at in [message.len() - 1, message.len() / 2] {
        let mut flipped = message.clone();
        flipped[at] ^= 1;
        inputs.push(flipped);
    }
    inputs.extend([
        [message.as_slice(), &[0]].concat(),
        b"not a message".to_vec(),
    ]);

    let mut device = load(86, &states[1]);
    for input in &inputs {
        assert!(
            device.decrypt(channel, input).is_err(),
            "refused: {} bytes",
            input.len()
        );
    }
    let decrypted = device
        .decrypt(channel, &message)
        .expect("the refusals changed nothing: the intact message decrypts");
    assert_eq!(decrypted.sender, device_id(85));
    assert_eq!(decrypted.plaintext.to_vec(), b"intact");
}
