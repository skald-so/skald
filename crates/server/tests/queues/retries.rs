//! A device's retry of an upload the server refused gets the first refusal, whatever its bytes.

use skald_proto::QueueId;

use crate::kit::{DEFAULT, SIZE_LIMIT, World, assert_not_held, keyed, queued, task};
use crate::presence::random_bytes;

/// Each refusal of queues.md: dave's laptop's upload, outside the channel; carol's with the ID of
/// alice's task; alice's with a `ct` over the limit, under the epoch after the current one, and to
/// a queue the channel does not have. After a Commit makes that epoch current, each device's retry
/// with the same task ID, as a task the server would otherwise store, gets the first answer, and
/// nothing more is stored.
#[test]
fn a_retry_of_a_refused_upload_gets_the_first_refusal() {
    let mut w = World::with(3_907_500_001, &[SIZE_LIMIT, "20000"]);
    let [alice, _, carol, dave, _] = w.laptops();
    let default = alice.queue(DEFAULT).id;
    let first = keyed(w.task(default), b"release");
    alice.stores(&first);

    let mut same_id = w.task(default);
    same_id.task = first.task;
    let mut over = w.task(default);
    over.ct = random_bytes(20_001);
    let next = task(default, w.epoch() + 1);
    let nowhere = w.task(QueueId::generate());
    let refused = [
        (&dave, w.task(default), "dave's"),
        (&carol, same_id, "carol's with alice's task ID"),
        (&alice, over, "alice's over the limit"),
        (&alice, next, "alice's under the next epoch"),
        (&alice, nowhere, "alice's to no queue"),
    ];
    let answers: Vec<_> = (refused.iter())
        .map(|(client, attempt, _)| client.enqueue(attempt))
        .collect();
    w.rotate();

    for ((client, attempt, what), answer) in refused.iter().zip(&answers) {
        assert!((400..500).contains(&answer.status), "{what}: refused");
        let mut retry = w.task(default);
        retry.task = attempt.task;
        let again = client.enqueue(&retry);
        let found = (again.status, &again.content_type, &again.body);
        let expected = (answer.status, &answer.content_type, &answer.body);
        assert_eq!(found, expected, "{what}: the retry gets the first answer");
        assert_not_held(w.db(), &retry, what);
    }
    assert_eq!(alice.tasks(default), queued(&[&first]), "the first alone");
}
