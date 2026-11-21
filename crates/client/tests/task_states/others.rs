//! A member who is neither of the uploader's account nor an admin is refused a restart and a cancel
//! of a task with `not-allowed`, which the CLI shows.

use skald_proto::{
    Cbor as _, ChannelId, PROOF_HEADER, PROTOCOL_VERSION, QueueId, QueuedTask, Role, Sched,
    SpaceId, TaskId, Unknown, VERSION_HEADER,
};

use crate::agents::Agents;
use crate::cloud::{Cloud, agent};
use crate::fixtures::CHANNEL;
use crate::kit::{latest, received};
use crate::presence::proof;
use crate::queuing::{default_queue, names_problem, run_in, tasks_path};
use crate::states::{CANCEL_TASK, DEAD, FAILED, QUEUED, RESTART_TASK, claims, fails, read};
use crate::talk::Talk;
use crate::world::{ACME, Person, next_by, set_role, store};

/// The problem the CLI names.
const NOT_ALLOWED: &str = "not-allowed";

/// `uploader`'s device uploads a task of `at`'s channel's queue `queue` sealed under `epoch`, as
/// the server's tests of queues make one: the server does not open it. Fails unless the server
/// stores it; gives its ID.
#[track_caller]
fn uploads(
    cloud: &Cloud,
    uploader: &Person,
    (at, queue): ((SpaceId, ChannelId), QueueId),
    epoch: u64,
) -> TaskId {
    let task = QueuedTask {
        task: TaskId::generate(),
        queue,
        sched: Sched::default(),
        epoch,
        enc: [7; 32],
        ct: vec![9; 64],
        unknown: Unknown::default(),
    };
    let url = format!("{}{}", cloud.url, tasks_path(at, queue));
    let request = (agent().post(&url))
        .header("content-type", "application/cbor")
        .header(PROOF_HEADER, proof(cloud, uploader))
        .header("authorization", format!("Bearer {}", uploader.token))
        .header(VERSION_HEADER, PROTOCOL_VERSION.to_string());
    let mut answer =
        (request.send(&task.to_cbor()[..])).unwrap_or_else(|err| panic!("POST {url}: {err}"));
    let status = answer.status().as_u16();
    let said = answer.body_mut().read_to_string().unwrap_or_default();
    assert!((200..300).contains(&status), "not stored: {status} {said}");
    task.task
}

/// In the talk of the tests of channel messages, revision 3 makes Bob and Carol admins, and a
/// revision Ada's device signs makes Ada a member. Bob's device uploads two tasks to `default`;
/// Carol's device claims the first and fails it without retry. In `~/work/acme`, the person's
/// `skald restart-task --channel <channel> <first's ID>` and `skald cancel-task --channel
/// <channel> <second's ID>` each fail naming `not-allowed`; the first stays `dead` and the second
/// `queued`.
#[test]
fn member_who_is_no_admin_is_refused_a_restart_and_a_cancel_with_not_allowed() {
    let talk = Talk::with(
        ACME,
        &[7_402_005_001, 7_402_005_002, 7_402_005_003],
        &["ada-steps-down", "bob-runs-acme", "carol-runs-acme-too"],
        false,
        (Role::Admin, |_| {}),
    );
    let agents = Agents::bound(talk);
    let talk = &agents.talk;
    let (cloud, ada, at) = (talk.cloud(), &talk.ada, talk.at());
    let stepped_down = next_by(&latest(cloud, &ada.token, at.0), ada, |list| {
        set_role(list, ada.account, Role::Member);
    });
    store(cloud, &ada.token, &stepped_down);
    received(&ada.machine);
    let (bob, carol) = (&talk.played[0], &talk.played[1].person);
    let epoch = (bob.mls.epoch(at.1)).expect("Bob's device is in the channel");
    let queue = default_queue(cloud, ada, at);
    let first = uploads(cloud, &bob.person, (at, queue), epoch);
    let held = claims(cloud, carol, at, queue);
    assert_eq!(held.task, first, "Carol's device claimed another task");
    fails(cloud, carol, &held, false);
    let second = uploads(cloud, &bob.person, (at, queue), epoch);
    let person = (agents.folder.as_path(), None);
    let channel = at.1.to_string();

    let restart = run_in(
        &ada.machine,
        person,
        RESTART_TASK,
        &[CHANNEL, &channel, &first.to_string()],
    );

    assert!(!restart.success, "restarted: {restart:?}");
    assert!(names_problem(&restart, NOT_ALLOWED), "{restart:?}");

    let cancel = run_in(
        &ada.machine,
        person,
        CANCEL_TASK,
        &[CHANNEL, &channel, &second.to_string()],
    );

    assert!(!cancel.success, "cancelled: {cancel:?}");
    assert!(names_problem(&cancel, NOT_ALLOWED), "{cancel:?}");
    let first_now = read(cloud, ada, (at, queue), first);
    assert_eq!(first_now.shows(), (DEAD, 1, None, Some(FAILED)));
    assert_eq!(read(cloud, ada, (at, queue), second).state, QUEUED);
}
