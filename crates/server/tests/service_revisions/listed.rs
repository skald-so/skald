//! What the tests share: a service agent added to `acme`, in `general`'s `services` too, the
//! changes that drop a device or an account from a revision, and a revision the server must refuse.

use skald_proto::{AccountId, DeviceId, NonEmpty, RosterBody, ServiceAgent};

use crate::agents::Acme;
use crate::api::assert_refused;
use crate::common::devices::assert_taken;

/// Stores, from alice's session, the next revision with `agent`'s entry and its device in
/// `general`'s `services`; the server must take it.
#[track_caller]
pub fn add(acme: &mut Acme, agent: &ServiceAgent) {
    let general = acme.f.general;
    let response = acme.propose(None, |body| {
        body.service_agents.push(agent.clone());
        let entry = body
            .channels
            .iter_mut()
            .find(|channel| channel.id == general);
        entry.expect("general").services.push(agent.device);
    });
    assert_taken(&response);
}

/// `body` without `device` in its service agent entries and its channels' `services`.
pub fn drop_device(body: &mut RosterBody, device: DeviceId) {
    body.service_agents.retain(|entry| entry.device != device);
    for channel in &mut body.channels {
        channel.services.retain(|listed| *listed != device);
    }
}

/// `body` without the member `account`, in the space and in each channel.
pub fn drop_account(body: &mut RosterBody, account: AccountId) {
    let members: Vec<_> = (body.members.iter())
        .filter(|member| member.account != account)
        .cloned()
        .collect();
    body.members = NonEmpty::from_vec(members).expect("a member");
    for channel in &mut body.channels {
        channel.members.retain(|member| member.account != account);
    }
}

/// Checks that the server refuses, from alice's session, the next revision with `change` applied,
/// also when sent again, and keeps the space's revisions as they were; `what` names the case.
#[track_caller]
pub fn assert_refused_revision(acme: &Acme, change: impl FnOnce(&mut RosterBody), what: &str) {
    let revision = acme.signed(change);
    let session = acme.alice_on(None);
    for attempt in ["", ", again"] {
        assert_refused(
            &session.store(acme.space(), &revision),
            &format!("{what}{attempt}"),
        );
    }
    assert_eq!(
        session.revisions(acme.space()),
        acme.f.revisions,
        "{what}: the stored revisions"
    );
}
