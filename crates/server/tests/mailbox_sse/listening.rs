//! A stream and its instance's listening to the others ("Frames"): a `space-notice` is no mailbox
//! entry, so a stream gets none of a revision stored before it opened, even when it opens before
//! its instance first listens, which then sends its connections the notices of revisions it has
//! not seen.

use std::thread;
use std::time::Duration;

use crate::fixture::Fixture;
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
