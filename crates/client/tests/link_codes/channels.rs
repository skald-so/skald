//! A device linked with a link code gets into the account's channels: the linked device adds it
//! once it has KeyPackages, at the end of `skald link` or at a later run, or it joins by itself.
//! The device that certified it adds it also where another device signs the space's revisions, and
//! waits for its KeyPackages up to the timeout. Neither it nor the new device warns of a Commit.
//! Each of the account's channels then has it as one leaf, and it shows the history from before
//! its link once a member device has run since.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{DeviceId, Role};

use crate::cloud::WAIT;
use crate::devices::{BOB, said_by};
use crate::kit::{Change, assert_members, group, latest, received, revise};
use crate::linking::{LINK, NewDevice, YES, link, says, start_link, typed};
use crate::reading::{assert_history, sends, shown_history};
use crate::relay::Sent;
use crate::talk::Talk;
use crate::terminal::Shown;
use crate::world::{ACME, WARNING};

/// The host name of Ada's new machine.
const LAPTOP: &str = "ada-laptop";
/// What `skald link` says once the new device has no KeyPackages within the timeout.
const NO_KEY_PACKAGES: &str = "no KeyPackages";

/// A talk of Ada and Bob, with `ids` and `logins`, and Ada's new machine, which shows a link code.
fn with_new_device(ids: &[u64], logins: &[&str]) -> (Talk, NewDevice) {
    let talk = Talk::new(ids, logins, false);
    let new = NewDevice::start(&talk.world, &talk.ada, LAPTOP);
    (talk, new)
}

/// Fails unless both of the space's channels of `talk` have Ada's first device, Bob's and `more`,
/// and no other.
#[track_caller]
pub fn assert_channels_have(talk: &Talk, more: &[DeviceId]) {
    let at = talk.at();
    let devices = [&[talk.ada.device, talk.played[BOB].device()], more].concat();
    for channel in [talk.space.default, at.1] {
        let held = group(talk.cloud(), &talk.ada.token, at.0, channel);
        assert_members(&held.members, &devices);
    }
}

/// Links `new` on Ada's first machine of `talk` with its code, `skald link` taking `extra`, after
/// `withhold` had the relay keep back its upload of KeyPackages, the request line given; calls
/// `looked` once `skald link` said it linked the new device. Gives what `skald link` showed.
fn link_without_key_packages(
    talk: &Talk,
    new: &mut NewDevice,
    extra: &[&str],
    withhold: impl FnOnce(&str),
    looked: impl FnOnce(&str),
) -> Shown {
    let upload = format!("POST /devices/{}/key-packages", new.device);
    withhold(&upload);
    new.open_run(talk.cloud(), &talk.ada);
    let mut linking = start_link(&talk.ada, extra);
    linking.type_line(&typed(&new.code()));
    linking.wait_to_show(LAPTOP);
    linking.type_line(YES);
    linking.wait_to_show("Linked the device");
    looked(&upload);
    let linked = linking.finish();
    assert!(linked.success, "{LINK}: {}", linked.text);
    linked
}

/// Fails if a warning of `shown`, a run's output, names a Commit: one the server did not answer or
/// refused.
#[track_caller]
fn assert_no_commit_warning(shown: &str) {
    let named: Vec<String> = (shown.lines().map(str::to_lowercase))
        .filter(|line| line.contains(WARNING) && line.contains("commit"))
        .collect();
    assert!(named.is_empty(), "{named:?}: {shown}");
}

/// In the talk's channel Bob says a line and Ada's machine says one. Ada's machines reach the
/// server at its public address. Her new machine is linked with its code on her first one: by the
/// end of `skald link`, which warns of no Commit, her first device's Commit that adds the new one
/// is stored, as both of the space's channels have Ada's two devices and Bob's, each once. Then
/// the new machine runs, and her first one, each warning of no Commit, and the new machine's
/// `skald history` shows both lines, oldest first.
#[test]
fn the_linked_device_gets_into_the_accounts_channels_and_reads_their_history() {
    let mut talk = Talk::new(
        &[4_175_201_001, 4_175_201_002],
        &["ada-links-into-channels", "bob-in-the-channel"],
        false,
    );
    let (at, space) = (talk.at(), talk.space.name.clone());
    let said = vec![
        said_by(&mut talk, BOB, (at, 1), "bob opens the launch checklist").0,
        sends(&talk.ada, (&space, at.1), "ada adds the release notes"),
    ];

    let mut new = NewDevice::start(&talk.world, &talk.ada, LAPTOP);
    let linked = link(talk.cloud(), &talk.ada, &mut new);
    assert_no_commit_warning(&linked.text);
    assert_channels_have(&talk, &[new.device]);
    let laptop = new.into_person(talk.cloud(), &talk.ada);
    for machine in [&laptop.machine, &talk.ada.machine, &laptop.machine] {
        assert_no_commit_warning(&received(machine).shown());
    }

    assert_channels_have(&talk, &[laptop.device]);
    let shown = shown_history(&laptop.machine, at.1);
    assert_history(&shown, at.1, &said, &[]);
}

/// Bob is an admin of the talk's space and his device signs its latest revision, so Ada's first
/// device adds only the devices it certified. Ada's new machine is linked with its code on her first
/// one: by the end of `skald link` her first device has added it to both of the space's channels,
/// before her new machine runs again.
#[test]
fn the_certifying_device_adds_the_new_one_by_the_end_of_skald_link() {
    let talk = Talk::with(
        ACME,
        &[4_175_201_011, 4_175_201_012],
        &["ada-certifies-the-laptop", "bob-signs-acme"],
        false,
        (Role::Admin, |_| {}),
    );
    let (cloud, ada, bob) = (talk.cloud(), &talk.ada, &talk.played[BOB].person);
    let at = talk.at();
    revise(
        cloud,
        &latest(cloud, &bob.token, at.0),
        bob,
        &Change::default(),
    );
    received(&ada.machine);

    let mut new = NewDevice::start(&talk.world, ada, LAPTOP);
    link(cloud, ada, &mut new);
    for channel in [talk.space.default, at.1] {
        let held = group(cloud, &ada.token, at.0, channel);
        assert_members(&held.members, &[ada.device, bob.device, new.device]);
    }
}

/// Ada's new machine's upload of its KeyPackages is held back until her first machine's `skald
/// link`, its timeout as long as the test waits, has looked for them twice and found none; then it
/// goes on. The new machine ends linked, and by the end of `skald link`, which says nothing of
/// missing KeyPackages, her first device has added the new one to both of the space's channels.
#[test]
fn the_certifying_device_waits_for_keypackages_that_come_late() {
    let (talk, mut new) = with_new_device(
        &[4_175_201_021, 4_175_201_022],
        &["ada-waits-for-keypackages", "bob-beside-the-wait"],
    );
    let cloud = talk.cloud();
    let since = Instant::now();
    let count = format!("GET /devices/{}/key-packages", new.device);
    // The wait covers the new machine's own steps up to its upload too, which take long under load.
    // No recovery key is derived from here on, so its cost takes none of it.
    let timeout = WAIT.as_secs().to_string();
    let linked = link_without_key_packages(
        &talk,
        &mut new,
        &["--timeout", &timeout],
        |upload| cloud.relay.hold_next(upload),
        |_| {
            // The new device asks once, before its upload; its first device then asks twice, so
            // only a device that waits gets them. The upload goes on then, or at once if it comes
            // later: no other request has to come to let it go.
            let deadline = Instant::now() + WAIT;
            while cloud.count_sent(since, &count) < 3 {
                assert!(Instant::now() < deadline, "no look for KeyPackages");
                thread::sleep(Duration::from_millis(50));
            }
            cloud.relay.let_go();
        },
    );
    // What became of the new machine first: without its upload the rest cannot hold.
    let shown = new.finish();
    // When its uploads came, against the looks for its KeyPackages.
    let at = |sent: &Sent| sent.at.duration_since(since).as_secs_f64();
    let sent = cloud.relay.sent_since(since);
    let looks: Vec<f64> = (sent.iter())
        .filter(|sent| sent.head.starts_with(&count.to_lowercase()))
        .map(at)
        .collect();
    let upload = format!("post /devices/{}/key-packages ", new.device);
    let uploads: Vec<String> = (sent.iter())
        .filter(|sent| sent.head.starts_with(&upload))
        .map(|sent| format!("{:.1} s", at(sent)))
        .collect();
    let seen = format!(
        "the new device showed: {}\nits uploads of KeyPackages came at {uploads:?}; {} looks for \
         them, the first at {:.1} s, the last at {:.1} s",
        shown.text,
        looks.len(),
        looks.first().copied().unwrap_or_default(),
        looks.last().copied().unwrap_or_default(),
    );
    assert!(shown.success, "the new device failed: {seen}");
    assert!(
        !says(&linked.text, &[NO_KEY_PACKAGES]),
        "{}\n{seen}",
        linked.text
    );
    assert_channels_have(&talk, &[new.device]);
}

/// The server never takes the KeyPackages of Ada's new machine. Her first machine's `skald link`,
/// its timeout 8 seconds, ends once they have not come within it, saying that a later run adds the
/// new device; neither of the space's channels has it.
#[test]
fn skald_link_ends_at_the_timeout_without_keypackages() {
    let (talk, mut new) = with_new_device(
        &[4_175_201_031, 4_175_201_032],
        &["ada-gets-no-keypackages", "bob-beside-no-keypackages"],
    );
    let cloud = talk.cloud();
    let linked = link_without_key_packages(
        &talk,
        &mut new,
        &["--timeout", "8"],
        |upload| cloud.relay.refuse(upload, "503 Service Unavailable"),
        |_| {},
    );
    assert!(
        says(&linked.text, &[NO_KEY_PACKAGES, "later run"]),
        "{}",
        linked.text
    );
    assert_channels_have(&talk, &[]);
    new.finish();
}
