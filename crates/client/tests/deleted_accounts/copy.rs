//! A full copy of the disk of a deleted account's device opens no message sent after a member
//! device removed it, even from a compromised server that serves the copy as before.

use serde_json::{Value, json};
use skald_proto::{RawEnvelope, Role};

use crate::fixtures::{fronted, send};
use crate::kit::{RECV, create_space, group_info_request, warnings_naming, with_channel};
use crate::played::Played;
use crate::revocations::copy_of;
use crate::scene::{LAPTOP, anchors_space, delete_account, linked, settle};
use crate::talk::{ADA_HOST, BOB_HOST, CAROL_HOST, messages_sent};
use crate::world::{ACME, devices_request, run};

/// What Bob says after Ada's deletion; neither text is anywhere else.
const AFTER: [&str; 2] = [
    "the new office door code is 4417",
    "payroll moves to the second bank on monday",
];

/// Bob's machine created `acme`, with Ada and Carol members, and a channel of all three. Ada's
/// first machine runs the CLI: it follows `acme` from the anchors object her second device, played,
/// signs, and with Carol's device, played, all four devices get into both channels. A full copy of
/// the disk of Ada's first machine is taken; then Ada deletes her account. Bob's next two runs,
/// `skald send` in the channel, say two lines, which Carol's device reads. A compromised server
/// then serves the copy as before the deletion: it gives back the account's sessions, sign-in
/// methods, username and devices, reports no account deleted, hides every GroupInfo from it, and
/// puts Bob's two lines into its catch-up, where they reach it. The copy's `skald recv` shows
/// neither line and warns that it cannot open them.
#[test]
fn copy_of_a_deleted_account_s_device_opens_no_message_sent_after_its_removal() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let bob = world.sign_up(4_305_102_001, "bob-says-after", BOB_HOST);
    let ada = world.sign_up(4_305_102_002, "ada-copied-before", ADA_HOST);
    let laptop = linked(&world, &ada, LAPTOP);
    let carol = world.sign_up(4_305_102_003, "carol-reads-after", CAROL_HOST);
    let space = create_space(cloud, &bob, ACME);
    let channel = with_channel(
        cloud,
        space.id,
        &bob,
        &[(&ada, Role::Member), (&carol, Role::Member)],
    );
    let mut laptop = Played::new(cloud, laptop);
    let mut carol = Played::new(cloud, carol);
    anchors_space(cloud, &laptop.person, space.id);
    let devices = [bob.device, ada.device, laptop.device(), carol.device()];
    settle(
        cloud,
        (space.id, &[space.default, channel]),
        (
            &[&ada.machine, &bob.machine],
            &mut [&mut laptop, &mut carol],
        ),
        &devices,
    );
    let copy = copy_of(&ada.machine, ADA_HOST);
    let account = ada.account;
    cloud.execute(&format!(
        "CREATE TABLE kept_methods AS
             SELECT * FROM sign_in_methods WHERE account_id = '{account}'::uuid;
         CREATE TABLE kept_sessions AS
             SELECT * FROM sessions WHERE account_id = '{account}'::uuid;"
    ));
    delete_account(cloud, &ada);
    let from = front.sent().len();
    for text in AFTER {
        let sent = send(&bob.machine, channel, text);
        assert!(sent.success, "{sent:?}");
    }
    let said: Vec<RawEnvelope> = (messages_sent(&front, channel, from).into_iter())
        .map(|sent| sent.envelope)
        .collect();
    assert_eq!(said.len(), 2, "not two lines sent");
    carol.take_in(cloud);
    let read: Vec<_> = (carol.read_in(channel).into_iter())
        .map(|(msg_id, _)| msg_id)
        .collect();
    for envelope in &said {
        let msg_id = envelope.envelope().header.msg_id;
        assert!(read.contains(&msg_id), "Carol did not read {msg_id}");
    }
    cloud.execute(&format!(
        "UPDATE accounts SET deleted_at = NULL, username = '{handle}' WHERE id = '{account}'::uuid;
         INSERT INTO sign_in_methods SELECT * FROM kept_methods;
         INSERT INTO sessions SELECT * FROM kept_sessions;
         UPDATE devices SET revoked = false WHERE account_id = '{account}'::uuid;",
        handle = ada.handle
    ));
    cloud.relay.rewrite(&devices_request(space.id), |body| {
        let mut list: Value = serde_json::from_str(body).expect("the server's device list");
        if let Some(fields) = list.as_object_mut() {
            fields.remove("deleted");
        }
        list.to_string()
    });
    for of in [space.default, channel] {
        cloud.relay.rewrite(&group_info_request(space.id, of), |_| {
            json!({ "group": true, "group_info": null }).to_string()
        });
    }
    front.inject(ada.device, said.clone());

    let ran = run(&copy, &[RECV]);

    assert_eq!(front.injected(), 2, "not put into catch-up: {ran:?}");
    let shown = ran.shown();
    for text in AFTER {
        assert!(!shown.contains(text), "the copy shows {text:?}: {ran:?}");
    }
    assert!(
        !warnings_naming(&ran, channel).is_empty() || !warnings_naming(&ran, bob.device).is_empty(),
        "no warning that it cannot open the lines: {ran:?}"
    );
}
