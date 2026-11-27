//! A separate application used by the Rust message SDK acceptance tests.

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::json;
use skald_client::{Client, DeviceContext, KeyStorage, MessageError};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let state_dir = PathBuf::from(args.next().expect("device state directory"));
    let seconds = args
        .next()
        .expect("request timeout")
        .to_str()
        .expect("UTF-8 timeout")
        .parse()
        .expect("timeout in seconds");
    let mut client = Client::new(DeviceContext::one_shot(
        state_dir,
        KeyStorage::File,
        Duration::from_secs(seconds),
    ));

    let result = match args.next().as_deref().and_then(|arg| arg.to_str()) {
        Some("send") => {
            let channel = args
                .next()
                .expect("channel")
                .to_str()
                .expect("UTF-8 channel")
                .parse()
                .expect("channel ID");
            let text = args.next().expect("message text");
            client
                .send(channel, text.to_str().expect("UTF-8 text"))
                .map(|id| {
                    println!("{}", json!({"type": "sent", "msg_id": id.to_string()}));
                })
        }
        Some("receive") => {
            let mut out = io::stdout().lock();
            client.receive(|message| {
                let received = json!({
                    "type": "message",
                    "msg_id": message.id.to_string(),
                    "reply_to": message.reply_to.map(|id| id.to_string()),
                    "text": message.text,
                });
                writeln!(out, "{received}")?;
                out.flush()
            })
        }
        _ => panic!("expected send or receive"),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(MessageError::SendRefused { problem, .. }) => {
            eprintln!(
                "{}",
                json!({"type": "send-refused", "problem": problem.to_string()})
            );
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}
