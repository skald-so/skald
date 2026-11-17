//! A stream and its instance's listening to the others ("Frames"): a `space-notice` is no mailbox
//! entry, so a stream gets none of a revision stored before it opened, even when it opens before
//! its instance first listens, which then sends its connections the notices of revisions it has
//! not seen. An instance that does not listen in time answers a stream and a waiting long-poll
//! `500` and writes why to its output.

use std::thread;
use std::time::Duration;

use crate::common::Response;
use crate::fixture::Fixture;
use crate::https::assert_problem;
use crate::link::QUIET;
use crate::proxy::Proxy;
use crate::sse::Sse;

/// How soon an instance whose first listening was refused tries again: within a few seconds.
const RELISTEN: Duration = Duration::from_secs(15);
/// How long after the request the test lets the instance listen.
const LET_LISTEN: Duration = Duration::from_millis(500);

/// bob's laptop opens a stream on an instance whose listening the database refuses; the instance
/// listens while the stream waits for it. The stream gets the empty `batch` and no notice of `acme`,
/// whose revisions were all stored before.
#[test]
fn a_stream_opened_before_its_instance_listens_gets_no_notice_of_an_earlier_revision() {
    let f = Fixture::new(3_141_420_401);
    let t = &f.team;
    let proxy = Proxy::to(f.db());
    proxy.refuse_listening(true);
    let behind = t.flow.stand.instance_at(&proxy.url(f.db()), &[], &[]);
    proxy.await_refused(RELISTEN);
    let bob = Sse::to(&behind, &t.flow, &t.bob, &t.bob.laptop);
    let proof = bob.https().proof();

    let mut stream = thread::scope(|scope| {
        scope.spawn(|| {
            thread::sleep(LET_LISTEN);
            proxy.refuse_listening(false);
        });
        bob.open_on(None, &proof)
    });

    assert_eq!(stream.catch_up(), []);
    stream.assert_quiet(QUIET);
}

/// What an instance that does not listen to the others in time writes to its output when it answers
/// a request `500`.
const NOT_LISTENING: &str = "skald-server: cannot answer a request: the instance does not listen \
                             to the others yet\n";

/// Checks that an instance of the server of the fixture of `seed`, whose listening the database
/// refuses, answers what `request` sends as bob's laptop `500` and writes why to its output.
fn assert_not_listening_answers_500(seed: u64, request: impl FnOnce(&Sse) -> Response) {
    let f = Fixture::new(seed);
    let t = &f.team;
    let proxy = Proxy::to(f.db());
    proxy.refuse_listening(true);
    let mut behind = t.flow.stand.instance_at(&proxy.url(f.db()), &[], &[]);
    proxy.await_refused(RELISTEN);

    let refused = request(&Sse::to(&behind, &t.flow, &t.bob, &t.bob.laptop));

    assert_problem(&refused, 500, Some("server-error"));
    let output = behind.stop();
    assert!(output.contains(NOT_LISTENING), "{output}");
}

/// bob's laptop opens a stream on an instance that does not listen to the others.
#[test]
fn a_stream_on_an_instance_that_does_not_listen_answers_500_and_writes_why() {
    assert_not_listening_answers_500(3_141_420_411, |bob| {
        bob.refused(None, Some(&bob.https().proof()))
    });
}

/// bob's laptop long-polls with a wait on an instance that does not listen to the others.
#[test]
fn a_long_poll_on_an_instance_that_does_not_listen_answers_500_and_writes_why() {
    assert_not_listening_answers_500(3_141_420_421, |bob| {
        let https = bob.https();
        https.poll_request(0, 1, Some(https.proof())).call()
    });
}
