//! Runs the live check of the Codex app-server: the `agent-app-server` client on an app-server of
//! its own and, with `--all`, on an app-server that listens on a Unix socket, next to the terminal
//! UI of a session a person works in. Everything that starts no process is in the library's
//! `app_server` module.

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use agent_app_server::{Client, Socket};
use agent_live::app_server::{
    self, AppServerCheck, CLIENT_BIN, Connect, Delivery, client, listening, person,
};
use agent_live::channel::{Files, TERM};
use agent_live::{Agent, Kind, Options, Stub, binary, queue, token};
use agent_source::{Source, now_ms};
use serde_json::json;

use crate::Result;
use crate::pty::{Added, Env, Prepared};
use crate::run::{finish, side_by_side, start};

/// The longest wait for a turn.
const TURN: Duration = Duration::from_secs(120);

/// How long the client's output is read before the check looks at its state again.
const TICK: Duration = Duration::from_millis(100);

/// The app-server path's live check: an idle thread; with `--all`, also a busy thread, and, on one
/// app-server, another thread next to the session a person works in.
pub fn app_server(options: &Options) -> Result<bool> {
    let work = start(Agent::Codex)?;
    let check = Check {
        work: work.path(),
        client: &binary(&std::env::current_exe()?, CLIENT_BIN)?,
        options,
        path: &std::env::var("PATH").unwrap_or_default(),
    };
    let mut verdicts = AppServerCheck::default();

    // Idle: a message waits for thread a, which the client starts on its own app-server.
    let (a, stub_a, env_a) = check.own("a", Kind::Idle)?;
    let added_a = check.add(&a)?;
    let delivery_a = check.run(&a, &env_a, stub_a.as_ref(), None, |_| Ok(()))?;
    println!("{}", verdicts.idle(&delivery_a, &added_a.now()?));

    if options.all {
        // Busy: the message is added when the agent starts the shell command of the thread's
        // first turn.
        let (c, stub_c, env_c) = check.own("c", Kind::Busy)?;
        let source_c = Source::open(&c.source)?;
        let mut added_c = None;
        let prompt = queue::prompt(Kind::Busy);
        let delivery_c = check.run(&c, &env_c, stub_c.as_ref(), Some(&prompt), |delivery| {
            if delivery.command_started_ms.is_some() && added_c.is_none() {
                added_c = Some(Added::new(&source_c, &c.source, &token())?);
            }
            Ok(())
        })?;
        let sent_c = added_c.as_ref().map(Added::now).transpose()?;
        println!("{}", verdicts.busy(&delivery_c, sent_c.as_ref()));

        check.shared(&mut verdicts)?;
    }

    let verdict = verdicts.verdict(options);
    Ok(finish(work, options, &verdict, verdicts.pass()))
}

/// What every session of the check shares.
struct Check<'a> {
    work: &'a Path,
    /// The `agent-app-server` binary.
    client: &'a Path,
    options: &'a Options<'a>,
    path: &'a str,
}

impl Check<'_> {
    /// The files of the `kind` thread `name` on an app-server of its own, its stub unless the model
    /// is real, and the client's environment.
    fn own(&self, name: &str, kind: Kind) -> Result<(Files, Option<Stub>, Env)> {
        let files = Files::new(&self.work.join(name));
        fs::create_dir_all(&files.project)?;
        let stub = self.options.stub(queue::script(kind))?;
        let env = Env::of(
            &files,
            stub.as_ref(),
            self.path,
            |home, stub, path| Agent::Codex.environment(home, stub, path),
            Vec::new(),
        )?;
        Ok((files, stub, env))
    }

    /// Adds a check's message to the source of the session in `files`.
    fn add(&self, files: &Files) -> Result<Added> {
        Added::new(&Source::open(&files.source)?, &files.source, &token())
    }

    /// Runs the client for the session in `files` on its own app-server, on `stub` if given; see
    /// [`run_client`].
    fn run(
        &self,
        files: &Files,
        env: &Env,
        stub: Option<&Stub>,
        prompt: Option<&str>,
        on_line: impl FnMut(&Delivery) -> Result<()>,
    ) -> Result<Delivery> {
        let url = stub.map(Stub::url);
        let options = app_server::options(self.options.model, url.as_deref());
        let args = client(files, &Connect::Own(options), prompt);
        let mut delivery = run_client(self.client, &args, files, env, on_line)?;
        delivery.stub = stub.map(Stub::log);
        if let Some(stub) = &delivery.stub {
            stub.save(&files.dir.join("stub.json"))?;
        }
        Ok(delivery)
    }

    /// One app-server on a Unix socket, with the terminal UI of a session a person works in, d, and
    /// once its first turn ended, the client: a thread of its own, b, then d's thread, each with a
    /// message waiting.
    fn shared(&self, verdicts: &mut AppServerCheck) -> Result<()> {
        let person_files = Files::new(&self.work.join("d"));
        let files_b = Files::new(&self.work.join("b"));
        for files in [&person_files, &files_b] {
            fs::create_dir_all(&files.project)?;
        }
        let socket = self.work.join("s.sock");
        let stub = self.options.stub(queue::script(Kind::Idle))?;
        let url = stub.as_ref().map(Stub::url);
        let options = app_server::options(self.options.model, url.as_deref());
        // The app-server and the client get the environment of the `inbox` check; the terminal UI
        // also the terminal.
        let env = || -> io::Result<Env> {
            Env::of(
                &person_files,
                stub.as_ref(),
                self.path,
                |home, stub, path| Agent::Codex.environment(home, stub, path),
                Vec::new(),
            )
        };
        let mut server = Server::start(
            &listening(&socket, &person_files.project, &options),
            &person_files,
            &env()?,
        )?;
        server.wait_for(&socket)?;

        let tui = person(&socket, &person_files.project, self.options.model);
        let tui_env = Env::of(
            &person_files,
            stub.as_ref(),
            self.path,
            queue::stub_environment,
            vec![("TERM".to_owned(), TERM.to_owned())],
        )?;
        let (client_env, project) = (env()?, person_files.project.clone());
        let prepared = Prepared::new(
            "d",
            Agent::Codex,
            false,
            person_files.clone(),
            tui,
            tui_env,
            stub,
        )?;
        let added_b = self.add(&files_b)?;
        let added_d = self.add(&person_files)?;
        let mut live = prepared.start()?;
        let started = Instant::now();
        let thread = loop {
            if !live.pump_for(Duration::from_millis(500))? {
                break None;
            }
            if let Some(thread) = person_thread(&socket, &project)? {
                break Some(thread);
            }
            if started.elapsed() > TURN {
                eprintln!("live-check: the person's session has no idle thread");
                break None;
            }
        };

        let clients_done = AtomicBool::new(false);
        let (pumped, deliveries) = side_by_side(
            || -> Result<()> {
                while !clients_done.load(Ordering::SeqCst) {
                    live.pump_for(TICK)?;
                }
                // The terminal UI draws the last reply.
                live.pump_for(Duration::from_secs(1))?;
                Ok(())
            },
            Some(|| -> Result<(Delivery, Option<Delivery>)> {
                let delivered = (|| {
                    let socket_b = Connect::Socket {
                        path: &socket,
                        thread: None,
                    };
                    let args = client(&files_b, &socket_b, None);
                    let b = run_client(self.client, &args, &files_b, &client_env, |_| Ok(()))?;
                    let d = match &thread {
                        Some(thread) => {
                            let socket_d = Connect::Socket {
                                path: &socket,
                                thread: Some(thread),
                            };
                            let args = client(&person_files, &socket_d, None);
                            Some(run_client(
                                self.client,
                                &args,
                                &person_files,
                                &client_env,
                                |_| Ok(()),
                            )?)
                        }
                        None => None,
                    };
                    Ok((b, d))
                })();
                clients_done.store(true, Ordering::SeqCst);
                delivered
            }),
        );
        pumped?;
        let (mut delivery_b, delivery_d) = deliveries.expect("the clients ran")?;
        let shown_b = live.showed(added_b.token());
        let shown_d = live.showed(added_d.token());
        let session = live.end()?;
        drop(server);

        delivery_b.stub.clone_from(&session.stub);
        println!("{}", verdicts.other(&delivery_b, &added_b.now()?, shown_b));
        let mut delivery_d = delivery_d.unwrap_or_else(|| Delivery::new(now_ms()));
        delivery_d.stub = session.stub;
        println!("{}", verdicts.person(&delivery_d, &added_d.now()?, shown_d));
        Ok(())
    }
}

/// The thread of the person's session in `project` on the app-server on `socket`, once its first
/// turn ended.
fn person_thread(socket: &Path, project: &Path) -> Result<Option<String>> {
    let stop = Arc::new(AtomicBool::new(false));
    let mut client = Client::new(Socket::connect(socket)?, io::sink(), stop);
    client.initialize()?;
    // Codex keeps the directory as it got it or with its symbolic links resolved.
    let cwd = [project.to_path_buf(), fs::canonicalize(project)?];
    let list = client.call("thread/list", json!({ "cwd": cwd }))?;
    Ok(app_server::person_thread(&list))
}

/// An app-server the check runs on a Unix socket; dropping it stops it.
struct Server(Child);

impl Server {
    /// Starts `codex <args>` in `files.project` with `env`; its output goes to `files.dir`.
    fn start(args: &[String], files: &Files, env: &Env) -> Result<Self> {
        let mut command = Agent::Codex.command();
        env.apply(&mut command);
        let child = command
            .args(args)
            .current_dir(&files.project)
            .stdin(Stdio::null())
            .stdout(File::create(files.dir.join("app-server.stdout"))?)
            .stderr(File::create(files.dir.join("app-server.stderr"))?)
            .spawn()?;
        Ok(Self(child))
    }

    /// Waits up to 10 s for the app-server to listen on `socket`.
    fn wait_for(&mut self, socket: &Path) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !socket.exists() {
            if Instant::now() > deadline || self.0.try_wait()?.is_some() {
                return Err(
                    format!("the app-server does not listen on {}", socket.display()).into(),
                );
            }
            thread::sleep(TICK);
        }
        Ok(())
    }
}

impl Drop for Server {
    /// Stops the app-server, even when the check fails early.
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Runs the client `agent-app-server <args>` in `files.project` with `env`, calling `on_line` after
/// each line it prints, until the turn that took the session's message answered or [`TURN`]
/// passes; then ends its stdin, which stops it. Its output goes to `files.dir`.
fn run_client(
    client: &Path,
    args: &[String],
    files: &Files,
    env: &Env,
    mut on_line: impl FnMut(&Delivery) -> Result<()>,
) -> Result<Delivery> {
    let mut command = Agent::Codex.command_for(client);
    env.apply(&mut command);
    let mut child = command
        .args(args)
        .current_dir(&files.project)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(File::create(files.dir.join("client.stderr"))?)
        .spawn()?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let (lines, received) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(|line| line.ok()) {
            if lines.send((line, now_ms())).is_err() {
                break;
            }
        }
    });
    let mut log = File::create(files.dir.join("client.jsonl"))?;
    let mut delivery = Delivery::new(now_ms());
    let deadline = Instant::now() + TURN;
    while delivery.answer().is_none() {
        if Instant::now() > deadline {
            eprintln!("live-check: client in {} timed out", files.dir.display());
            break;
        }
        match received.recv_timeout(TICK) {
            Ok((line, at_ms)) => {
                writeln!(log, "{line}")?;
                delivery.record(&line, at_ms);
                on_line(&delivery)?;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                eprintln!(
                    "live-check: client in {} ended early; see client.stderr",
                    files.dir.display()
                );
                break;
            }
        }
    }
    drop(child.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait()?.is_none() {
        if Instant::now() > deadline {
            child.kill()?;
            break;
        }
        thread::sleep(TICK);
    }
    let status = child.wait()?;
    if !status.success() {
        eprintln!(
            "live-check: client in {} {status}; see client.stderr",
            files.dir.display()
        );
    }
    Ok(delivery)
}
