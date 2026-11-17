//! A sender counts as a channel member only by a revision the device accepted: one the server lists
//! that the device did not accept admits no one.

use serde_json::Value;
use skald_proto::{Cbor as _, ChannelMember, MessageId, Role};

use crate::chain::{carol, key_of};
use crate::fixtures::person_address;
use crate::kit::{Change, b64, latest, revise};
use crate::reading::{Duo, add_entries, assert_history, body_of, warned_of};
use crate::talk::Line;
use crate::world::next_with;

/// The text of the entry of Carol's device; it is nowhere else.
const UNADMITTED: &str = "carol writes where no accepted revision put her";

/// Bob says a line. A revision Ada's device signs adds Carol to the space, outside the channel.
/// The relay then lists after it a revision Bob's device, no admin's, signs that adds Carol to the
/// channel, which Ada's device does not accept, and adds an entry of Carol's device under the key
/// of epoch 1. Ada's `skald history` shows Bob's line and not Carol's, and warns naming Carol's
/// device.
#[test]
fn entry_of_an_account_only_an_unaccepted_revision_puts_in_the_channel_is_dropped() {
    let mut duo = Duo::new(
        &[4_166_014_001, 4_166_014_002],
        &["ada-trusts-her-roster", "bob-keeps-the-thread"],
    );
    let (said, ..) = duo.bob_says(&duo.bob_s(1, "bob's line in the thread"));
    let Duo { talk, at, .. } = &duo;
    let cloud = &talk.world.cloud;
    let carol = carol(talk, 4_166_014_003, "carol-off-the-thread");
    let change = Change {
        members: vec![(&carol.person, Role::Member)],
        outside_default: true,
        ..Change::default()
    };
    let in_space = revise(
        cloud,
        &latest(cloud, &talk.ada.token, at.0),
        &talk.ada,
        &change,
    );
    let bob = &talk.played[0];
    let unaccepted = next_with(&in_space, (bob.person.device, &bob.person.key), |body| {
        let member = ChannelMember {
            account: carol.person.account,
            history: true,
        };
        for channel in body.channels.iter_mut().filter(|it| it.id == at.1) {
            channel.members.push(member);
        }
    });
    let listed = Value::String(b64(&unaccepted.to_cbor()));
    cloud
        .relay
        .rewrite(&format!("GET /spaces/{}/revisions", at.0), move |answer| {
            let mut given: Value = serde_json::from_str(answer).expect("the server's revisions");
            let list = given["revisions"]
                .as_array_mut()
                .expect("a list of revisions");
            list.push(listed.clone());
            given.to_string()
        });
    let line = Line::new(
        &person_address(&talk.space.name, &carol.person),
        1,
        UNADMITTED,
    );
    let body = body_of(carol.device(), at.1, (MessageId::generate(), &line));
    let entry = key_of(bob, at.1, 1)
        .seal_entry(1, &body, &carol.person.key)
        .expect("an entry");
    add_entries(cloud, *at, &[entry.to_cbor()]);

    let shown = duo.ada_s_history();

    assert_history(&shown, duo.at.1, &[said], &[UNADMITTED]);
    assert!(
        !warned_of(&shown, carol.device()).is_empty(),
        "no warning naming Carol's device: {shown:?}"
    );
}
