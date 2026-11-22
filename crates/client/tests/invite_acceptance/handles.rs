//! The person confirms before the device signs, and gets asked again for a handle that is taken or
//! no valid name.

use skald_proto::{DeviceKind, ServiceAgent, Unknown};

use crate::fixtures::{Issuer, add_device_of_kind};
use crate::world::{next_with, store};
use crate::{HANDLE, NO, Scene, YES, accept_at_terminal, handle};

/// The owner and the name of a service agent of `acme`.
const OWNER: &str = "ci";
const SERVICE_AGENT: &str = "deploy";

/// `skald accept-invite` at a terminal on Bob's machine, given the link, shows the invite and asks
/// to confirm; Bob says no. His machine sends no acceptance and follows nothing: the server holds
/// none, and no anchors object of his names `acme`.
#[test]
fn declined_confirmation_signs_nothing() {
    let scene = Scene::new(
        (7_305_405_001, "ada-invites-a-hesitant-bob"),
        (7_305_405_002, "bob-declines"),
    );
    let made = scene.invite();
    let since = scene.cloud().relay.mark();

    let declined = accept_at_terminal(&scene.bob.machine, &made.link, &[], (NO, &[]));

    assert!(!declined.text().contains("add-rule"), "{}", declined.text());
    scene.assert_nothing_accepted(&made, since);
}

/// Revision 3 of `acme`, signed by Ada's device, lists a service agent of Ada's, `ci/deploy`, on a
/// `service` device her device certified; the invite's link names it. `skald accept-invite --handle
/// <Ada's handle>` at a terminal on Bob's machine, given the link, confirmed: it asks for another
/// handle; given `Bob Reviews`, no valid name, it asks again; given `ci`, the service agent's
/// owner, it asks again; given `bob-reviews`, it succeeds. The server holds his acceptance asking
/// for `bob-reviews`.
#[test]
fn handle_a_member_or_a_service_agent_owner_holds_is_asked_again() {
    let scene = Scene::new(
        (7_305_405_101, "ada-keeps-her-name"),
        (7_305_405_102, "bob-wants-ada-s-name"),
    );
    let (cloud, ada) = (scene.cloud(), &scene.ada);
    let service = add_device_of_kind(
        cloud,
        (ada.account, DeviceKind::Service),
        "ada-ci-runner",
        &Issuer::Device(ada),
    );
    let third = next_with(&scene.latest(), (ada.device, &ada.key), |body| {
        body.service_agents.push(ServiceAgent {
            device: service.id,
            owner: handle(OWNER),
            agent: handle(SERVICE_AGENT),
            unknown: Unknown::default(),
        });
    });
    store(cloud, &ada.token, &third);
    let made = scene.invite();
    assert_eq!(made.revision.hash(), third.hash());

    let accepted = accept_at_terminal(
        &scene.bob.machine,
        &made.link,
        &[HANDLE, ada.handle.as_str()],
        (YES, &["Bob Reviews", OWNER, "bob-reviews"]),
    );

    assert!(accepted.shown.success, "{}", accepted.text());
    scene.assert_bob_accepted(&made, "bob-reviews");
}
