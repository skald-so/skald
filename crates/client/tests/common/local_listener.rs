//! A listener on a free local port, for the servers, stubs and proxies of the tests. The client's
//! tests, the adapters' tests and the stand-ins include it through `#[path]`.

use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

/// WSAENOBUFS: Windows refuses a bind to port 0 with it now and then while most of its ephemeral
/// ports are free, in the same second as its Tcpip event 4231 ("all such ports being in use").
const NO_BUFFER_SPACE: i32 = 10_055;

/// Listens on a free port of 127.0.0.1. A bind Windows refuses for want of buffer space is tried
/// again after a pause that grows from 50 ms, for about 5 s in all; any other error ends the test.
pub fn local_listener() -> TcpListener {
    let start = Instant::now();
    let mut pause = Duration::from_millis(50);
    loop {
        match TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => return listener,
            Err(err)
                if err.raw_os_error() == Some(NO_BUFFER_SPACE)
                    && start.elapsed() < Duration::from_secs(5) =>
            {
                eprintln!("bind a local port: {err}; again in {pause:?}");
                thread::sleep(pause);
                pause = (pause * 2).min(Duration::from_secs(1));
            }
            Err(err) => panic!("bind a local port: {err:?}"),
        }
    }
}
