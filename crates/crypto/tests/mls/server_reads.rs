//! What the server reads of a channel's `mls` payloads without its group's secrets: the framing of
//! each `MLSMessage`, whether it is a handshake message (`docs/protocol/delivery.md`, "Mailbox"),
//! and whether a GroupInfo a member uploads lets a device join by itself ("MLS profile").

use mls_rs::MlsMessage;
use skald_crypto::{ChannelView, MlsContent, MlsError, MlsFraming};
use skald_proto::ChannelId;

use crate::fixtures::device_id;
use crate::foreign;
use crate::group_info::{EXTERNAL_PUB, resigned};
use crate::steps::{add, bytes, channel_with, send, step, with_key_packages};

/// The framing of a PublicMessage or a PrivateMessage of `channel` at `epoch`.
fn message(channel: ChannelId, epoch: u64, public: bool, content: MlsContent) -> MlsFraming {
    MlsFraming::Message {
        group: channel.as_bytes().to_vec(),
        epoch,
        public,
        content,
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_framing_of_each_kind_of_message_is_read_and_handshake_messages_are_told_apart() {
    let channel = ChannelId::generate();
    // Device 41 adds device 42 with the KeyPackage of a client outside the crate, which proposes.
    let outsider = foreign::key_package(43, device_id(43).as_bytes());
    let (mut states, welcome) = channel_with(channel, 41, &[42], &[&outsider]);
    let application = send(41, &mut states[0], channel, b"hello");
    let (_, mut packages) = with_key_packages(44, 1);
    let commit = add(41, &mut states[0], channel, &[&packages[0]]);
    let group_info = step(42, &mut states[1], |device| {
        bytes(device.group_info(channel).expect("a member's GroupInfo"))
    });
    let joiner = foreign::key_package(46, device_id(46).as_bytes());
    let (mut foreign_group, _) = foreign::group(45, device_id(45).as_bytes(), channel, &[&joiner]);
    let proposal = foreign_group
        .propose_update(Vec::new())
        .and_then(|message| message.to_bytes())
        .expect("an Update proposal of mls-rs");

    let cases = [
        (welcome.clone(), MlsFraming::Welcome, true, "a Welcome"),
        (
            application.clone(),
            message(channel, 1, false, MlsContent::Application),
            false,
            "an application message",
        ),
        (
            commit.clone(),
            message(channel, 1, true, MlsContent::Commit),
            true,
            "a Commit",
        ),
        (
            proposal,
            message(channel, 1, true, MlsContent::Proposal),
            true,
            "a proposal",
        ),
        (group_info, MlsFraming::Other, false, "a GroupInfo"),
        (packages.remove(0), MlsFraming::Other, false, "a KeyPackage"),
    ];
    for (bytes, framing, handshake, what) in cases {
        let read = MlsFraming::read(&bytes).unwrap_or_else(|| panic!("{what} read"));
        assert_eq!(read, framing, "{what}");
        assert_eq!(
            read.is_handshake(),
            handshake,
            "{what}: a handshake message"
        );
    }

    for (bytes, what) in [
        (Vec::new(), "no bytes"),
        (vec![0xff; 64], "bytes that are no MLS message"),
        (commit[..commit.len() - 1].to_vec(), "a Commit cut short"),
        (
            [commit.as_slice(), &[0]].concat(),
            "a Commit with a byte after it",
        ),
        ([welcome.as_slice(), &welcome].concat(), "two Welcomes"),
    ] {
        assert_eq!(MlsFraming::read(&bytes), None, "{what}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_group_info_without_external_pub_is_not_joinable() {
    let channel = ChannelId::generate();
    let (mut states, _) = channel_with(channel, 51, &[52], &[]);
    let whole = step(52, &mut states[1], |device| {
        bytes(device.group_info(channel).expect("a member's GroupInfo"))
    });
    let view = ChannelView::observe_joinable(channel, &whole).expect("a joinable GroupInfo");
    assert_eq!(view.signer().device, device_id(52));
    assert_eq!(view.epoch(), 1);

    let without = resigned(&whole, 52, |fields| {
        fields.extensions.retain(|(kind, _)| *kind != EXTERNAL_PUB);
    });
    assert!(ChannelView::observe(channel, &without).is_ok(), "observed");
    assert!(
        matches!(
            ChannelView::observe_joinable(channel, &without),
            Err(MlsError::NotJoinable)
        ),
        "without external_pub"
    );
    // Refused as observing refuses it, before the extension.
    let other = ChannelId::generate();
    assert!(matches!(
        ChannelView::observe_joinable(other, &whole),
        Err(MlsError::OtherChannel)
    ));
    let key_package = MlsMessage::from_bytes(&foreign::key_package(53, device_id(53).as_bytes()))
        .and_then(|message| message.to_bytes())
        .expect("a KeyPackage");
    for (bytes, what) in [
        (key_package, "a KeyPackage"),
        (vec![0xff; 64], "bytes that are no MLS message"),
    ] {
        assert!(
            matches!(
                ChannelView::observe_joinable(channel, &bytes),
                Err(MlsError::Malformed)
            ),
            "{what}"
        );
    }
}
