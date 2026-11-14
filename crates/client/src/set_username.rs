//! `skald set-username`: sets the username of the account of the machine's session, on the server
//! of that session, through `PUT /account/username`. A username the server refuses fails with the
//! server's reason, and the account keeps its old one.

use std::path::Path;
use std::time::Duration;

use skald_proto::VersionRange;

use crate::connection::{self, Method, printable};
use crate::devices::{signed_in, text, to_session};
use crate::secrets;

/// Sets `username` as the username of the account the machine whose state is in `dir` signed in
/// to, its secrets in `storage`, and records it in the local state.
pub fn run(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    username: &str,
) -> Result<(), String> {
    let (state, server, token) = signed_in(dir, storage)?;

    let mut connection = to_session(&server, speaks, timeout)?;
    let body = serde_json::json!({ "username": username }).to_string();
    match connection.send_json_as(Method::Put, "/account/username", &token, body.as_bytes()) {
        Ok(_) => {}
        Err(connection::Error::Refused {
            status: 400 | 409,
            detail,
        }) => {
            return Err(refused(username, &detail));
        }
        Err(err) => return Err(err.to_string()),
    }
    state.set_username(username).map_err(text)?;
    println!(
        "The account's username on {server} is now {}.",
        printable(username)
    );
    Ok(())
}

/// What `skald set-username` says when the server refuses `username` for the reason `detail`.
fn refused(username: &str, detail: &str) -> String {
    format!(
        "the server refused the username \"{}\": {}; the account keeps its username",
        printable(username),
        printable(detail)
    )
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;
    use crate::secrets::{Map, Secrets};
    use crate::sign_in::{SESSION_SERVER, SESSION_TOKEN};
    use crate::state::State;
    use crate::test_dir::TestDir;

    /// A server that takes one request with a body of `length` bytes, answers it with
    /// `204 No Content` and gives its request line and its body.
    fn server_taking_one_request(length: usize) -> (String, thread::JoinHandle<(String, String)>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let url = format!("http://{}", listener.local_addr().expect("the address"));
        let served = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("a request");
            let mut reader = BufReader::new(stream.try_clone().expect("clone the stream"));
            let mut request_line = String::new();
            reader
                .read_line(&mut request_line)
                .expect("the request line");
            let mut line = String::new();
            while line != "\r\n" {
                line.clear();
                assert_ne!(reader.read_line(&mut line).expect("a header"), 0, "no body");
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).expect("the body");
            (&stream)
                .write_all(
                    b"HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                )
                .expect("answer");
            let body = String::from_utf8(body).expect("a UTF-8 body");
            (request_line.trim_end().to_owned(), body)
        });
        (url, served)
    }

    /// The username the server takes replaces the old one in the local state, with no sign-in.
    #[test]
    fn new_username_replaces_the_old_one_in_the_local_state_without_a_sign_in() {
        let dir = TestDir::new("set-username-recorded");
        let sent = r#"{"username":"ada-of-skald"}"#;
        let (server, served) = server_taking_one_request(sent.len());
        let state = State::open(&dir).unwrap();
        state.start_session(&server).unwrap();
        state.set_account("account-1", "ada-lovelace").unwrap();
        drop(state);
        Secrets::new(&dir, secrets::Storage::File)
            .save(&Map::from([
                (SESSION_TOKEN.to_owned(), "token-1".to_owned()),
                (SESSION_SERVER.to_owned(), server.clone()),
            ]))
            .unwrap();

        run(
            &dir,
            secrets::Storage::File,
            VersionRange::default(),
            Duration::from_secs(10),
            "ada-of-skald",
        )
        .unwrap();

        assert_eq!(
            served.join().unwrap(),
            ("PUT /account/username HTTP/1.1".to_owned(), sent.to_owned())
        );
        let session = State::open(&dir).unwrap().session().unwrap().unwrap();
        assert_eq!(session.username.as_deref(), Some("ada-of-skald"));
    }

    #[test]
    fn refusal_gives_the_username_and_the_servers_reason_without_control_characters() {
        assert_eq!(
            refused("root\u{1b}", "The username root is reserved.\u{7}"),
            "the server refused the username \"root\\u{1b}\": The username root is reserved.\\u{7}; \
             the account keeps its username"
        );
    }
}
