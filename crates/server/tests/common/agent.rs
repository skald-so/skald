//! The tests' own HTTP client. The client's tests include this file too.

use std::sync::OnceLock;
use std::time::Duration;

/// An HTTP client that answers any status and follows no redirect. One for the test binary, which
/// keeps its connections open until their server ends: the server closes them first, so their
/// TIME_WAIT holds the server's port and none of the machine's ephemeral ports. macOS has 16,384
/// of them, each held for 30 s after a connection its client closes; several runs of the tests at
/// once would use them all up.
pub fn agent() -> ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT
        .get_or_init(|| {
            ureq::Agent::config_builder()
                .proxy(None)
                .http_status_as_error(false)
                .max_redirects(0)
                .max_redirects_will_error(false)
                // Past the longest test. The oldest connections go first, those of ended servers.
                .max_idle_age(Duration::from_secs(600))
                .max_idle_connections(200)
                .max_idle_connections_per_host(50)
                .build()
                .new_agent()
        })
        .clone()
}

/// What `request` gives, sending it again once if its connection ended before an answer came: a
/// kept connection that its server closed just as the request went out, before this process saw
/// the close.
pub fn call<T>(mut request: impl FnMut() -> Result<T, ureq::Error>) -> Result<T, ureq::Error> {
    use std::io::ErrorKind;
    match request() {
        Err(ureq::Error::Io(err))
            if matches!(
                err.kind(),
                ErrorKind::UnexpectedEof | ErrorKind::ConnectionReset | ErrorKind::BrokenPipe
            ) =>
        {
            request()
        }
        result => result,
    }
}
