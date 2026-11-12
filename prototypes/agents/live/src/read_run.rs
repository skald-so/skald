//! Runs the live checks of read confirmations: the `ack` tool, the session's own file, and resending
//! a message no layer confirmed. Everything that runs no CLI is in the library's `read` module.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use agent_live::hook::HOOK_BIN;
use agent_live::read::{
    self, Confirmed, RESEND_AFTER_MS, ReadCheck, ack_prompt, ack_script, read_session_file,
    resend_session, resend_unconfirmed, transcript_session,
};
use agent_live::{Agent, Event, INBOX_BIN, Options, SENDER, Server, Stub, channel, prepare, token};
use agent_socket::{FROM, Receipts, hand_over};
use agent_source::{Message, Source, now_ms};

use crate::Result;
use crate::run::{finish, run_session, start_with};

/// A check message of `source` with `token`: its state and records now.
fn confirmed(source: &Source, message: &Message, token: &str) -> Result<Confirmed> {
    Ok(Confirmed {
        token: token.to_owned(),
        state: source.state(&message.msg_id)?,
        records: source.records(&message.msg_id)?,
    })
}

/// The `ack` tool's live check: an idle session acks its message; with `--all`, a session acks
/// several messages in its next calls and replies to one.
pub fn ack(agent: Agent, options: &Options) -> Result<bool> {
    let (work, inbox, path) = start_with(agent, INBOX_BIN)?;
    let mut check = ReadCheck::new("ack");
    let run_case = |name: &str, piggyback: bool, count: usize| -> Result<_> {
        let stub = options.stub(ack_script(piggyback))?;
        let url = stub.as_ref().map(Stub::url);
        let run = prepare(
            work.path(),
            name,
            agent,
            &path,
            stub.as_ref(),
            |_, project, source| {
                let server = Server {
                    inbox: &inbox,
                    source,
                };
                agent.session_with(
                    project,
                    server,
                    &ack_prompt(piggyback),
                    "",
                    options.model,
                    url.as_deref(),
                )
            },
        )?;
        let source = Source::open(&run.source)?;
        let added = (0..count)
            .map(|_| {
                let token = token();
                let message = source.add_with_ack(SENDER, &channel::text(&token))?;
                Ok((message, token))
            })
            .collect::<Result<Vec<_>>>()?;
        let session = run_session(&run, stub.as_ref(), |_| {})?;
        Ok((session, source, added))
    };

    let (session, source, added) = run_case("a", false, 1)?;
    let (message, token) = &added[0];
    let injected = read::injected(&source, &message.msg_id)?;
    let message = confirmed(&source, message, token)?;
    println!("{}", check.ack(&session, &message, &injected));

    if options.all {
        let (session, source, added) = run_case("p", true, 3)?;
        let messages = added
            .iter()
            .map(|(message, token)| confirmed(&source, message, token))
            .collect::<Result<Vec<_>>>()?;
        println!("{}", check.piggyback(&session, &messages));
    }

    let verdict = check.verdict(agent, options);
    Ok(finish(work, options, &verdict, check.pass()))
}

/// The session file's live check: the `PostToolUse` hook hands a message over, the `Stop` hook
/// finds it in the session's file, and so does the client after the session.
pub fn transcript(agent: Agent, options: &Options) -> Result<bool> {
    let (work, handler, path) = start_with(agent, HOOK_BIN)?;
    let stub = options.stub(read::TRANSCRIPT_SCRIPT)?;
    let url = stub.as_ref().map(Stub::url);
    let run = prepare(
        work.path(),
        "a",
        agent,
        &path,
        stub.as_ref(),
        |dir, project, source| {
            transcript_session(
                agent,
                dir,
                project,
                &handler,
                source,
                options.model,
                url.as_deref(),
            )
        },
    )?;
    let source = Source::open(&run.source)?;
    let token = token();
    let message = channel::add_message(&source, &token)?;
    // The client polls the session's file while the session runs: its record tells when the
    // message showed there.
    let ended = AtomicBool::new(false);
    let session = thread::scope(|scope| {
        scope.spawn(|| -> Result<()> {
            let source = Source::open(&run.source)?;
            while !ended.load(Ordering::SeqCst) && read_session_file(agent, &source)? == 0 {
                thread::sleep(Duration::from_millis(50));
            }
            Ok(())
        });
        let session = run_session(&run, stub.as_ref(), |_| {});
        ended.store(true, Ordering::SeqCst);
        session
    })?;
    read_session_file(agent, &source)?;
    let mut check = ReadCheck::new("transcript");
    println!(
        "{}",
        check.transcript(&session, &confirmed(&source, &message, &token)?)
    );
    let verdict = check.verdict(agent, options);
    Ok(finish(work, options, &verdict, check.pass()))
}

/// The resend live check, on the model stub only: while a bypass session runs `sleep 20`, one
/// message goes through the socket without the session's token, so the session holds it, and one
/// with it. Neither is confirmed within [`RESEND_AFTER_MS`], so both go again by the `Stop` hook,
/// which drops the one already in the transcript as a repeat.
pub fn resend(options: &Options) -> Result<bool> {
    let (work, handler, path) = start_with(Agent::Claude, HOOK_BIN)?;
    let stub = options.stub(read::RESEND_SCRIPT)?;
    let fallback_dir = work.path().join("a/fallback");
    let run = prepare(
        work.path(),
        "a",
        Agent::Claude,
        &path,
        stub.as_ref(),
        |dir, _, source| {
            resend_session(dir, &handler, source, &dir.join("fallback"), options.model)
        },
    )?;
    let primary = Source::open(&run.source)?;
    let fallback = Source::open(&fallback_dir)?;
    let mut sent: Option<Result<[(Message, String); 2]>> = None;
    let session = thread::scope(|scope| {
        run_session(&run, stub.as_ref(), |event| {
            if *event == Event::ShellStarted && sent.is_none() {
                sent = Some(send_both(scope, &run.source, &fallback_dir));
            }
        })
    })?;
    // The client reads the session's file for what the hook did not confirm yet.
    if let Some(registration) = primary.registration()? {
        fallback.register(&registration)?;
        read_session_file(Agent::Claude, &fallback)?;
    }
    let mut check = ReadCheck::new("resend");
    let Some(sent) = sent.transpose()? else {
        let verdict = check.verdict(Agent::Claude, options);
        println!("  the agent ran no shell command");
        return Ok(finish(work, options, &verdict, false));
    };
    let last = session
        .stub
        .as_ref()
        .and_then(|stub| stub.requests.last().cloned())
        .unwrap_or_default();
    let [(held, held_token), (late, late_token)] = &sent;
    println!(
        "{}",
        check.lost(
            &confirmed(&primary, held, held_token)?,
            &confirmed(&fallback, held, held_token)?,
            last.matches(held_token.as_str()).count(),
        )
    );
    println!(
        "{}",
        check.repeated(
            &confirmed(&primary, late, late_token)?,
            &confirmed(&fallback, late, late_token)?,
            last.matches(late_token.as_str()).count(),
        )
    );
    let verdict = check.verdict(Agent::Claude, options);
    Ok(finish(work, options, &verdict, check.pass()))
}

/// Sends the resend check's two messages through the socket of the session `primary` registered:
/// the first without the token, from a reply address whose receipts a thread of `scope` records;
/// the second with it. A thread then sends again by `fallback` what no layer confirmed in time.
fn send_both<'scope>(
    scope: &'scope thread::Scope<'scope, '_>,
    primary: &Path,
    fallback: &Path,
) -> Result<[(Message, String); 2]> {
    let source = Source::open(primary)?;
    let socket = source
        .registration()?
        .and_then(|registration| registration.socket)
        .ok_or("the session registered no socket")?;
    let receipts = Receipts::bind(Path::new(&socket))?;
    let held_token = token();
    let held = channel::add_message(&source, &held_token)?;
    hand_over(&source, false, &receipts.address())?;
    let late_token = token();
    let late = channel::add_message(&source, &late_token)?;
    hand_over(&source, true, FROM)?;
    let (primary, fallback) = (primary.to_owned(), fallback.to_owned());
    let listen = primary.clone();
    scope.spawn(move || -> Result<()> {
        let source = Source::open(&listen)?;
        receipts.collect(
            &source,
            Instant::now() + Duration::from_millis(RESEND_AFTER_MS),
            |_| {},
        )?;
        Ok(())
    });
    scope.spawn(move || -> Result<()> {
        thread::sleep(Duration::from_millis(RESEND_AFTER_MS + 200));
        let (primary, fallback) = (Source::open(&primary)?, Source::open(&fallback)?);
        resend_unconfirmed(&primary, &fallback, RESEND_AFTER_MS, now_ms())?;
        Ok(())
    });
    Ok([(held, held_token), (late, late_token)])
}
