//! Keep-alives: while idle, a stream sends a comment line at the server's interval, so that a proxy
//! that cuts idle streams keeps it open; it pushes entries as before.

use std::time::{Duration, Instant};

use crate::fixture::Fixture;
use crate::https::WAKE;
use crate::link::Entry;
use crate::sse::{Event, KEEP_ALIVE, Sse, sent_to_bobs_laptop};

/// The interval of the tests' server, shortened.
const INTERVAL: Duration = Duration::from_secs(1);
/// How late past its interval a keep-alive may come.
const LATE: Duration = Duration::from_millis(500);

/// On a server whose keep-alive interval is 1 second, bob's idle stream gets a keep-alive within a
/// second of the end of catch-up and of each previous one, three times, then an entry stored
/// meanwhile.
#[test]
fn an_idle_stream_gets_a_keep_alive_within_its_interval() {
    let f = Fixture::with(3_141_420_101, &[KEEP_ALIVE, "1"]);
    let t = &f.team;
    let mut bob = Sse::of(&t.flow, &t.bob).open(None);
    assert_eq!(bob.catch_up(), []);

    let mut last = Instant::now();
    for count in 1..=3 {
        let (event, at) = bob
            .next(INTERVAL + LATE)
            .unwrap_or_else(|| panic!("no keep-alive {count} within {:?}", INTERVAL + LATE));
        assert_eq!(event, Event::KeepAlive, "keep-alive {count}");
        let gap = at.saturating_duration_since(last);
        assert!(gap <= INTERVAL + LATE, "keep-alive {count} after {gap:?}");
        last = at;
    }

    let sent = sent_to_bobs_laptop(&f);
    assert_eq!(bob.entry_within(WAKE), Entry::Message(1, sent));
}
