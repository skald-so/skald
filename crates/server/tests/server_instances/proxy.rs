//! A TCP proxy between an instance and the test PostgreSQL server that can refuse the instance's
//! listening connections, or stop forwarding on them without closing them, as a network that
//! drops a connection silently does. A listening connection is one whose client has sent `LISTEN`.

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::common::TestDb;

/// How long a stopped connection or the proxy's accepting waits before it looks again.
const LOOK: Duration = Duration::from_millis(20);

pub struct Proxy {
    addr: SocketAddr,
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    refusing: AtomicBool,
    refused: AtomicUsize,
    connections: Mutex<Vec<Arc<Connection>>>,
    closed: AtomicBool,
}

/// What the proxy knows of one connection.
#[derive(Default)]
struct Connection {
    listening: AtomicBool,
    stopped: AtomicBool,
}

impl Proxy {
    /// A proxy to the PostgreSQL server of `db`.
    pub fn to(db: &TestDb) -> Self {
        let url = url::Url::parse(&db.url()).expect("a database URL");
        let server = format!(
            "{}:{}",
            url.host_str().expect("a database host"),
            url.port().expect("a database port")
        );
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        listener
            .set_nonblocking(true)
            .expect("a nonblocking listener");
        let addr = listener.local_addr().expect("the proxy's address");
        let shared = Arc::<Shared>::default();
        let accepting = Arc::clone(&shared);
        thread::spawn(move || {
            while !accepting.closed.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((client, _)) => {
                        client.set_nonblocking(false).expect("a blocking stream");
                        let server = TcpStream::connect(&server).expect("connect to PostgreSQL");
                        forward(&accepting, client, server);
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => thread::sleep(LOOK),
                    Err(err) => panic!("accept a connection: {err}"),
                }
            }
        });
        Proxy { addr, shared }
    }

    /// The URL of `db` through this proxy.
    pub fn url(&self, db: &TestDb) -> String {
        let mut url = url::Url::parse(&db.url()).expect("a database URL");
        url.set_port(Some(self.addr.port()))
            .expect("a URL with a port");
        url.into()
    }

    /// Closes each connection whose client sends `LISTEN` from now on, or stops doing so.
    pub fn refuse_listening(&self, refusing: bool) {
        self.shared.refusing.store(refusing, Ordering::Relaxed);
    }

    /// How many listening connections the proxy has closed so far.
    pub fn refused(&self) -> usize {
        self.shared.refused.load(Ordering::Relaxed)
    }

    /// Waits up to `within` until the proxy has closed a listening connection.
    #[track_caller]
    pub fn await_refused(&self, within: Duration) {
        let deadline = Instant::now() + within;
        while self.refused() == 0 {
            assert!(
                Instant::now() < deadline,
                "no listening refused after {within:?}"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Stops forwarding on each listening connection open now, keeping it open; gives how many
    /// it stopped.
    pub fn stop_listening(&self) -> usize {
        let connections = self.shared.connections.lock().expect("the connections");
        let listening = connections
            .iter()
            .filter(|connection| connection.listening.load(Ordering::Relaxed));
        listening
            .map(|connection| connection.stopped.store(true, Ordering::Relaxed))
            .count()
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Relaxed);
    }
}

/// Forwards between `client` and `server` in both directions.
fn forward(shared: &Arc<Shared>, client: TcpStream, server: TcpStream) {
    let connection = Arc::<Connection>::default();
    shared
        .connections
        .lock()
        .expect("the connections")
        .push(Arc::clone(&connection));
    let pumps = [
        (true, client.try_clone(), server.try_clone()),
        (false, server.try_clone(), client.try_clone()),
    ];
    for (from_client, from, to) in pumps {
        let (from, to) = (from.expect("a stream"), to.expect("a stream"));
        let (shared, connection) = (Arc::clone(shared), Arc::clone(&connection));
        thread::spawn(move || pump(&shared, &connection, from_client, from, to));
    }
}

/// Copies what comes from `from` to `to` until either closes, refusing or stopping as `shared`
/// says; `from_client` if `from` is the client.
fn pump(
    shared: &Shared,
    connection: &Connection,
    from_client: bool,
    mut from: TcpStream,
    mut to: TcpStream,
) {
    let mut buffer = [0; 16 * 1024];
    loop {
        let read = match from.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let chunk = &buffer[..read];
        if from_client && chunk.windows(6).any(|window| window == b"LISTEN") {
            connection.listening.store(true, Ordering::Relaxed);
            if shared.refusing.load(Ordering::Relaxed) {
                shared.refused.fetch_add(1, Ordering::Relaxed);
                break;
            }
        }
        // A stopped connection stays open and forwards nothing.
        while connection.stopped.load(Ordering::Relaxed) {
            if shared.closed.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(LOOK);
        }
        if to.write_all(chunk).is_err() {
            break;
        }
    }
    let _ = from.shutdown(std::net::Shutdown::Both);
    let _ = to.shutdown(std::net::Shutdown::Both);
}
