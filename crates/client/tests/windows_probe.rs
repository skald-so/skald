//! TEMPORARY: probes of the `Client on Windows` flakes. Removed before merge.
#![cfg(windows)]

#[allow(dead_code)]
mod common;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

/// A POST with a body the server does not read: the head first, the body `late` after it or with
/// it, then the answer read `late` after that.
fn unread_body(server: &common::Server, late: Option<Duration>) -> String {
    let addr = server.url.trim_start_matches("http://");
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let head = "POST /health HTTP/1.1\r\nhost: x\r\ncontent-type: application/json\r\n\
                content-length: 2\r\n\r\n";
    let outcome = (|| {
        match late {
            Some(late) => {
                stream
                    .write_all(head.as_bytes())
                    .map_err(|e| format!("head {:?}", e.raw_os_error()))?;
                thread::sleep(late);
                stream
                    .write_all(b"{}")
                    .map_err(|e| format!("body {:?}", e.raw_os_error()))?;
                thread::sleep(late);
            }
            None => stream
                .write_all(format!("{head}{{}}").as_bytes())
                .map_err(|e| format!("head and body {:?}", e.raw_os_error()))?,
        }
        let mut answer = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => answer.extend_from_slice(&buffer[..n]),
                Err(e) => {
                    return Err(format!(
                        "read {:?} after {} bytes",
                        e.raw_os_error(),
                        answer.len()
                    ));
                }
            }
            if answer.windows(4).any(|w| w == b"\r\n\r\n") && late.is_none() {
                break;
            }
        }
        let text = String::from_utf8_lossy(&answer);
        Ok(text.lines().next().unwrap_or("nothing").to_owned())
    })();
    outcome.unwrap_or_else(|e| e)
}

#[test]
fn probe() {
    let server = common::Server::start(None);
    let mut out = Vec::new();
    for late in [None, Some(Duration::from_millis(300))] {
        let mut stats = BTreeMap::<String, usize>::new();
        for _ in 0..10 {
            *stats.entry(unread_body(&server, late)).or_default() += 1;
        }
        out.push(format!("body late {late:?}: {stats:?}"));
    }
    panic!("PROBE\n{}", out.join("\n"));
}
