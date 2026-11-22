//! The runs that send the anchors object: `skald sign-in`, which signs the personal space's
//! revision 1, and a later run that follows no space, which sends one the server has not taken
//! and none it has; a run whose send fails sends it once.

use crate::objects::{ANCHORS_REQUEST, assert_anchors, held, latest, personal_space};
use crate::world::{MEMBERS, Person, SPACES, World, run};

/// The host of the tests' machine.
const HOST: &str = "ada-laptop";
/// A space Ada's machine does not follow: `skald members` of it follows no space.
const UNKNOWN: &str = "initech";
/// What a run warns when the server does not take the anchors object.
const UNSENT: &str = "this device's anchors object waits for the server to take it";

/// Runs `skald members` of a space `person`'s machine does not follow: a run that connects and
/// follows no space. Its result does not matter.
fn run_following_none(person: &Person) {
    let _ = run(&person.machine, &[MEMBERS, UNKNOWN]);
}

/// Ada's first machine signs in, which signs revision 1 of her personal space: the server then
/// holds the machine's anchors object of that revision. A next run that follows no space sends
/// none, as the server took it.
#[test]
fn sign_in_leaves_the_anchors_object_of_the_personal_space_and_a_next_run_sends_none() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_172_020_501, "ada-signed-in-anchors", HOST);
    let personal = personal_space(cloud, &ada);
    assert_eq!(cloud.revisions(&ada.token, personal).len(), 1);
    assert_anchors(cloud, &ada, &latest(cloud, &ada), personal);
    let since = cloud.relay.mark();

    run_following_none(&ada);

    assert_eq!(
        cloud.count_sent(since, ANCHORS_REQUEST),
        0,
        "an anchors object the server took was sent again"
    );
}

/// The relay answers every request that stores an anchors object as a server without them would
/// while Ada's first machine signs in. Once it lets them through, a run that follows no space
/// sends the waiting one, and the server holds it.
#[test]
fn waiting_anchors_object_is_uploaded_by_a_next_run_that_follows_no_space() {
    let world = World::start();
    let cloud = &world.cloud;
    cloud.relay.refuse(ANCHORS_REQUEST, "404 Not Found");
    let ada = world.sign_up(4_172_020_502, "ada-waiting-anchors", HOST);
    assert!(
        held(cloud, &ada.token).is_empty(),
        "an anchors object got through"
    );
    cloud.relay.stop_refusing();

    run_following_none(&ada);

    let personal = personal_space(cloud, &ada);
    assert_anchors(cloud, &ada, &latest(cloud, &ada), personal);
}

/// The relay keeps refusing anchors objects. `skald spaces`, which follows the personal space and
/// signs its default channel, sends the anchors object once and warns once.
#[test]
fn run_whose_anchors_object_is_refused_sends_it_once() {
    let world = World::start();
    let cloud = &world.cloud;
    cloud.relay.refuse(ANCHORS_REQUEST, "404 Not Found");
    let ada = world.sign_up(4_172_020_503, "ada-refused-anchors", HOST);
    let since = cloud.relay.mark();

    let listed = run(&ada.machine, &[SPACES]);

    assert!(listed.success, "{listed:?}");
    let personal = personal_space(cloud, &ada);
    assert_eq!(
        cloud.revisions(&ada.token, personal).len(),
        2,
        "the personal space's default channel"
    );
    assert_eq!(cloud.count_sent(since, ANCHORS_REQUEST), 1);
    assert_eq!(listed.stderr.matches(UNSENT).count(), 1, "{listed:?}");
}
