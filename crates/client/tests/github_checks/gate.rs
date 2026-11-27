//! A gate before a server, at an address of its own on this machine: open, it forwards each
//! connection to the server, both ways; closed, it drops each connection unanswered, as when the
//! server cannot be reached. Its address stays the same either way.

use std::io;
use std::net::{Shutdown, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use super::common::reach;

// Other helpers of the same test binary load it too.
#[path = "../common/local_listener.rs"]
#[allow(dead_code, clippy::duplicate_mod)]
mod local_listener;
use local_listener::local_listener;

pub struct Gate {
    port: u16,
    open: Arc<AtomicBool>,
}

impl Gate {
    /// An open gate before `target`, such as `127.0.0.1:41234`, serving until the tests end.
    pub fn start(target: &str) -> Self {
        let listener = local_listener();
        let port = listener.local_addr().expect("the gate's address").port();
        let open = Arc::new(AtomicBool::new(true));
        let (target, letting) = (target.to_owned(), Arc::clone(&open));
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                if !letting.load(Ordering::SeqCst) {
                    let _ = client.shutdown(Shutdown::Both);
                    continue;
                }
                let target = target.clone();
                thread::spawn(move || forward(client, &target));
            }
        });
        Gate { port, open }
    }

    /// Its URL: `http://127.0.0.1:<port>`.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// From now on, drops each connection unanswered.
    pub fn close(&self) {
        self.open.store(false, Ordering::SeqCst);
    }

    /// From now on, forwards each connection.
    pub fn open(&self) {
        self.open.store(true, Ordering::SeqCst);
    }
}

/// Forwards what `client` and `target` send each other until either closes.
fn forward(mut client: TcpStream, target: &str) {
    let Ok(mut upstream) = reach("gate", target) else {
        return;
    };
    let (Ok(mut from_client), Ok(mut to_upstream)) = (client.try_clone(), upstream.try_clone())
    else {
        return;
    };
    thread::spawn(move || {
        let _ = io::copy(&mut from_client, &mut to_upstream);
        let _ = to_upstream.shutdown(Shutdown::Write);
    });
    let _ = io::copy(&mut upstream, &mut client);
    let _ = client.shutdown(Shutdown::Write);
}
