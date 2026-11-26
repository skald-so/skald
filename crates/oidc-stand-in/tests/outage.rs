//! A stopped stand-in answers no request, on a connection opened before or after it stopped, and
//! keeps its port; restarted, it answers on that port again.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use skald_oidc_stand_in::{Key, Provider};

/// How long a read waits for an answer.
const WAIT: Duration = Duration::from_secs(10);

/// A connection to `provider`'s address.
fn connect(provider: &Provider) -> TcpStream {
    let address = provider.issuer().trim_start_matches("http://");
    let stream = TcpStream::connect(address).expect("connect to the stand-in");
    stream.set_read_timeout(Some(WAIT)).expect("a read timeout");
    stream
}

/// The status of the answer to a request for the metadata on `stream`, which stays open; the error
/// if none comes.
fn status(stream: &mut TcpStream) -> io::Result<u16> {
    stream
        .write_all(b"GET /.well-known/openid-configuration HTTP/1.1\r\nhost: 127.0.0.1\r\n\r\n")?;
    let mut reader = BufReader::new(stream);
    let mut head = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        if line == "\r\n" {
            break;
        }
        head.push(line.trim_end().to_lowercase());
    }
    let length: usize = (head.iter())
        .find_map(|line| line.strip_prefix("content-length:"))
        .map_or(Ok(0), |length| length.trim().parse())
        .expect("a content length");
    reader.read_exact(&mut vec![0; length])?;
    let status = head.first().and_then(|line| line.split(' ').nth(1));
    Ok(status
        .and_then(|status| status.parse().ok())
        .expect("a status"))
}

#[test]
fn a_stopped_stand_in_answers_nothing_and_answers_again_on_its_port_once_restarted() {
    let mut provider = Provider::start(&[Key::rsa()]);
    let mut before = connect(&provider);
    assert_eq!(status(&mut before).expect("an answer"), 200);

    provider.stop();
    assert!(
        status(&mut before).is_err(),
        "answered on a connection of before the stop"
    );
    assert!(
        status(&mut connect(&provider)).is_err(),
        "answered while stopped"
    );

    provider.restart();
    assert_eq!(status(&mut connect(&provider)).expect("an answer"), 200);
}
