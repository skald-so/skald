//! A TLS endpoint before a plain HTTP server, as Caddy is before `skald-server`: it takes TLS
//! connections on a free local port and forwards what each carries to the server, both ways. It
//! keeps every byte that crossed the wire to and from the machines, and what the machines sent
//! inside TLS.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use rustls::{ServerConfig, ServerConnection};

/// What crossed the endpoint so far.
#[derive(Default)]
struct Log {
    /// Every byte on the wire between the machines and the endpoint, both ways.
    wire: Mutex<Vec<u8>>,
    /// What the machines sent inside TLS.
    plain: Mutex<Vec<u8>>,
}

/// The endpoint, serving until the tests end.
pub struct Front {
    port: u16,
    connections: Arc<AtomicUsize>,
    log: Arc<Log>,
}

impl Front {
    /// Forwards to `target`, such as `127.0.0.1:41234`. Its first connections present the
    /// certificates of `tls`, one each in order; the connections after them, the last one.
    pub fn start(target: &str, tls: Vec<Arc<ServerConfig>>) -> Self {
        assert!(!tls.is_empty(), "a certificate to present");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let port = listener
            .local_addr()
            .expect("the endpoint's address")
            .port();
        let connections = Arc::new(AtomicUsize::new(0));
        let log = Arc::new(Log::default());
        let (accepted, logged) = (Arc::clone(&connections), Arc::clone(&log));
        let target = target.to_owned();
        thread::spawn(move || {
            for machine in listener.incoming().flatten() {
                let index = accepted.fetch_add(1, Ordering::SeqCst).min(tls.len() - 1);
                let (config, target) = (Arc::clone(&tls[index]), target.clone());
                let log = Arc::clone(&logged);
                thread::spawn(move || serve(machine, config, &target, &log));
            }
        });
        Front {
            port,
            connections,
            log,
        }
    }

    /// Its URL with the host `host`, such as `https://127.0.0.1:41235` for `127.0.0.1`.
    pub fn url(&self, host: &str) -> String {
        format!("https://{host}:{}", self.port)
    }

    /// How many connections it took so far.
    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    /// Every byte on the wire between the machines and the endpoint so far, both ways.
    pub fn wire(&self) -> Vec<u8> {
        self.log.wire.lock().unwrap().clone()
    }

    /// What the machines sent inside TLS so far, in lowercase.
    pub fn plain(&self) -> String {
        String::from_utf8_lossy(&self.log.plain.lock().unwrap()).to_lowercase()
    }
}

/// The machine's end of a connection: its TLS and its socket, written to only under one lock, so
/// that TLS records go out in the order TLS makes them.
struct Tls {
    connection: ServerConnection,
    machine: TcpStream,
}

impl Tls {
    /// Sends the machine what TLS has for it, keeping it in `log`.
    fn flush(&mut self, log: &Log) -> io::Result<()> {
        let mut out = Vec::new();
        while self.connection.wants_write() {
            self.connection.write_tls(&mut out)?;
        }
        log.wire.lock().unwrap().extend_from_slice(&out);
        self.machine.write_all(&out)
    }
}

/// Serves a connection of a machine with TLS by `config`, until the machine closes it or TLS fails;
/// connects to `target` once the machine has sent something inside TLS.
fn serve(machine: TcpStream, config: Arc<ServerConfig>, target: &str, log: &Arc<Log>) {
    let (Ok(connection), Ok(mut from_machine)) =
        (ServerConnection::new(config), machine.try_clone())
    else {
        return;
    };
    let tls = Arc::new(Mutex::new(Tls {
        connection,
        machine,
    }));
    let mut server: Option<TcpStream> = None;
    let mut buffer = [0; 16 * 1024];
    while let Ok(count @ 1..) = from_machine.read(&mut buffer) {
        log.wire.lock().unwrap().extend_from_slice(&buffer[..count]);
        let Some(plain) = decrypt(&tls, &buffer[..count], log) else {
            break;
        };
        if plain.is_empty() {
            continue;
        }
        log.plain.lock().unwrap().extend_from_slice(&plain);
        if server.is_none() {
            let Ok(stream) = TcpStream::connect(target) else {
                break;
            };
            let Ok(answers) = stream.try_clone() else {
                break;
            };
            let (tls, log) = (Arc::clone(&tls), Arc::clone(log));
            thread::spawn(move || answer(answers, &tls, &log));
            server = Some(stream);
        }
        let Some(stream) = server.as_mut() else {
            break;
        };
        if stream.write_all(&plain).is_err() {
            break;
        }
    }
    if let Some(stream) = server {
        let _ = stream.shutdown(Shutdown::Write);
    }
}

/// What `received`, bytes the machine sent, carried inside TLS, once TLS has taken them and
/// answered what it has to; `None` once TLS failed, such as when the machine refused the
/// certificate.
fn decrypt(tls: &Mutex<Tls>, mut received: &[u8], log: &Log) -> Option<Vec<u8>> {
    let mut tls = tls.lock().unwrap();
    let mut plain = Vec::new();
    while !received.is_empty() {
        tls.connection.read_tls(&mut received).ok()?;
        let processed = tls.connection.process_new_packets();
        // On a failure too: TLS tells the machine why.
        tls.flush(log).ok()?;
        processed.ok()?;
        match tls.connection.reader().read_to_end(&mut plain) {
            Err(err) if err.kind() != io::ErrorKind::WouldBlock => return None,
            _ => {}
        }
    }
    Some(plain)
}

/// Forwards what `server` answers to the machine inside TLS until the server closes, then closes
/// the machine's end.
fn answer(mut server: TcpStream, tls: &Mutex<Tls>, log: &Log) {
    let mut buffer = [0; 16 * 1024];
    while let Ok(count @ 1..) = server.read(&mut buffer) {
        let mut tls = tls.lock().unwrap();
        let sent = tls.connection.writer().write_all(&buffer[..count]);
        if sent.and_then(|()| tls.flush(log)).is_err() {
            return;
        }
    }
    let mut tls = tls.lock().unwrap();
    tls.connection.send_close_notify();
    let _ = tls.flush(log);
    let _ = tls.machine.shutdown(Shutdown::Write);
}
