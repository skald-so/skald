//! `agent-socket` hands messages to a stand-in for a session's inbox socket and records the
//! receipts the stand-in sends to its reply address.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::process::{Command, Output};
use std::thread;

use agent_source::{Registration, State};
use agent_testing::{assert_failed, one_waiting, stderr};
use serde_json::Value;

fn agent_socket(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agent-socket"))
        .args(args)
        .env_clear()
        .output()
        .unwrap()
}

/// A registered session whose socket at `dir/1.sock` reads one connection, and the source.
fn session(dir: &Path) -> (UnixListener, agent_source::Source, agent_source::Message) {
    let socket = dir.join("1.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let (source, message) = one_waiting(&dir.join("source"));
    source
        .register(&Registration {
            session_id: "s1".to_owned(),
            socket: Some(socket.to_string_lossy().into_owned()),
            token: Some("child-token".to_owned()),
            at_ms: 1,
            ..Registration::default()
        })
        .unwrap();
    (listener, source, message)
}

/// The lines of the first connection to `listener`, as JSON.
fn lines(listener: UnixListener) -> thread::JoinHandle<Vec<Value>> {
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        BufReader::new(stream)
            .lines()
            .map(|line| serde_json::from_str(&line.unwrap()).unwrap())
            .collect()
    })
}

#[test]
fn hands_the_messages_over_with_the_token_unless_told_not_to() {
    for (args, auth) in [(&[][..], true), (&["--no-auth"][..], false)] {
        let dir = tempfile::tempdir().unwrap();
        let (listener, source, message) = session(dir.path());
        let read = lines(listener);
        let source_dir = dir.path().join("source");
        let mut all = vec![source_dir.to_str().unwrap()];
        all.extend(args);
        let output = agent_socket(&all);
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(String::from_utf8_lossy(&output.stdout), "1\n");
        let got = read.join().unwrap();
        assert_eq!(got.len(), 1 + usize::from(auth), "{got:?}");
        assert_eq!(got[0]["type"], if auth { "auth" } else { "user" });
        let last = got.last().unwrap();
        assert_eq!(last["msg_id"], message.msg_id.as_str());
        assert_eq!(last["session_id"], "s1");
        assert_eq!(last["from"], "skald");
        assert!(matches!(
            source.state(&message.msg_id).unwrap(),
            Some(State::HandedOver { .. })
        ));
    }
}

#[test]
fn records_the_receipts_that_come_to_its_reply_address() {
    let dir = tempfile::tempdir().unwrap();
    let (listener, source, message) = session(dir.path());
    let msg_id = message.msg_id.clone();
    // The stand-in holds the message and says so to the reply address.
    let session = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let line: Value =
            serde_json::from_str(&BufReader::new(stream).lines().next().unwrap().unwrap())
                .unwrap();
        let reply = line["from"].as_str().unwrap().strip_prefix("uds:").unwrap();
        let mut stream = UnixStream::connect(reply).unwrap();
        let receipt = serde_json::json!({
            "type": "control", "action": "peer_message_status", "status": "held",
            "reason": "held for approval", "orig_msg_id": msg_id,
        });
        writeln!(stream, "{receipt}").unwrap();
    });
    let source_dir = dir.path().join("source");
    let output = agent_socket(&[source_dir.to_str().unwrap(), "--no-auth", "--listen", "2"]);
    session.join().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("1\nheld {}\n", message.msg_id)
    );
    let records = source.records(&message.msg_id).unwrap();
    assert_eq!(records[0].signal, "socket-held");
    assert_eq!(records[0].detail.as_deref(), Some("held for approval"));
    // The reply address is gone with the process.
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        2,
        "only the session's socket and the source are left"
    );
}

#[test]
fn fails_without_a_session_or_with_wrong_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let (_source, _message) = one_waiting(dir.path());
    assert_failed(
        &agent_socket(&[dir.path().to_str().unwrap()]),
        "agent-socket: no session registered",
    );
    for args in [
        &[][..],
        &["dir", "--listen"],
        &["dir", "--listen", "soon"],
        &["dir", "--auth"],
        &["dir", "--listen", "1", "--no-auth"],
    ] {
        let output = agent_socket(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(stderr(&output).starts_with("usage: agent-socket"), "{args:?}");
    }
}
