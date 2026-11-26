//! `skald` at a terminal: in a pseudo-terminal on Unix and a pseudo-console (ConPTY) on Windows,
//! with `portable-pty`, as a person runs it. What it prints is read as the person sees it, without
//! the terminal's control sequences; the test types on its input.

use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::machine::{Machine, RUN_LIMIT};

/// How long a finished run's output may take to drain once its terminal closes.
const DRAIN: Duration = Duration::from_secs(20);

/// A piece of a run's output, by its length, or what the test wrote on its input, since its start.
enum Event {
    Output(Duration, usize),
    Input(Duration, Vec<u8>),
}

/// A run of `skald` at a terminal.
pub struct Terminal {
    child: Box<dyn Child + Send + Sync>,
    /// Dropped once the run exits: on Windows, closing the pseudo-console ends its output.
    master: Option<Box<dyn MasterPty + Send>>,
    input: Arc<Mutex<Box<dyn Write + Send>>>,
    output: Arc<Mutex<Vec<u8>>>,
    /// When each piece of its output came and each input went, for a failure to show.
    events: Arc<Mutex<Vec<Event>>>,
    reader: Option<JoinHandle<()>>,
    started: Instant,
    private_output: bool,
}

/// What a run at a terminal printed, as the person saw it, and how it exited.
#[derive(Debug)]
pub struct Shown {
    pub success: bool,
    pub text: String,
}

impl Terminal {
    /// Starts `skald` with `args` on `machine` at a terminal, in the machine's home, set up as the
    /// machine's other runs are.
    pub fn start(machine: &Machine, args: &[&str]) -> Self {
        let home = machine.root().join("home");
        Self::spawn(&machine.command_in(&home, args), &home)
    }

    /// As [`Terminal::start`], in the working directory `dir`.
    // The recovery key's tests start none in another directory.
    #[allow(dead_code)]
    pub fn start_in(machine: &Machine, dir: &Path, args: &[&str]) -> Self {
        Self::spawn(&machine.command_in(dir, args), dir)
    }

    /// As [`Terminal::start`], its input not the terminal but empty, as a shell redirects it.
    #[cfg(unix)]
    pub fn start_without_input(machine: &Machine, args: &[&str]) -> Self {
        let home = machine.root().join("home");
        let skald = machine.command_in(&home, args);
        let mut shell = Command::new("/bin/sh");
        shell
            .args(["-c", "exec \"$0\" \"$@\" < /dev/null"])
            .arg(skald.get_program())
            .args(skald.get_args());
        for (name, value) in skald.get_envs() {
            if let Some(value) = value {
                shell.env(name, value);
            }
        }
        Self::spawn(&shell, &home)
    }

    fn spawn(command: &Command, dir: &Path) -> Self {
        let mut builder = CommandBuilder::new(command.get_program());
        builder.args(command.get_args());
        builder.env_clear();
        for (name, value) in command.get_envs() {
            if let Some(value) = value {
                builder.env(name, value);
            }
        }
        builder.cwd(dir);
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 50,
                // Wide enough that no line it prints wraps.
                cols: 400,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open a pseudo-terminal");
        let child = pair
            .slave
            .spawn_command(builder)
            .expect("run skald at a terminal");
        // The run holds the terminal's other end; the test's copy would keep its output open.
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("read the terminal");
        let input = Arc::new(Mutex::new(
            pair.master.take_writer().expect("write to the terminal"),
        ));
        let output = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let started = Instant::now();
        let reader = {
            let (output, input) = (Arc::clone(&output), Arc::clone(&input));
            let events = Arc::clone(&events);
            thread::spawn(move || {
                let mut buffer = [0; 4096];
                let mut asked = 0;
                while let Ok(read) = reader.read(&mut buffer) {
                    if read == 0 {
                        break;
                    }
                    let mut output = output.lock().expect("a run's output");
                    output.extend_from_slice(&buffer[..read]);
                    let came = Event::Output(started.elapsed(), read);
                    events.lock().expect("a run's events").push(came);
                    // A pseudo-console asks where the cursor is before it shows anything: it is at
                    // the start, as on a new terminal.
                    let queries = count(&output, b"\x1b[6n");
                    while asked < queries {
                        asked += 1;
                        write_input(&input, &events, started, b"\x1b[1;1R");
                    }
                }
            })
        };
        Terminal {
            child,
            master: Some(pair.master),
            input,
            output,
            events,
            reader: Some(reader),
            started,
            private_output: false,
        }
    }

    /// Suppresses transcripts in failures for runs that display or accept credentials.
    #[allow(dead_code)]
    #[must_use]
    pub fn with_private_output(mut self) -> Self {
        self.private_output = true;
        self
    }

    fn diagnostic(&self) -> String {
        if self.private_output {
            return "(terminal output withheld)".to_owned();
        }
        let output = self.output.lock().expect("a run's output");
        let tail = &output[output.len().saturating_sub(1 << 10)..];
        let events = self.events.lock().expect("a run's events");
        let timeline: Vec<String> = (events.iter().rev().take(40).rev())
            .map(|event| match event {
                Event::Output(at, length) => format!("{at:.3?} out {length} bytes"),
                Event::Input(at, bytes) => format!("{at:.3?} in {}", escaped(bytes)),
            })
            .collect();
        format!(
            "{}\n--- its output's last bytes: {}\n--- the last output and input, since its start:\n{}",
            plain(&output),
            escaped(tail),
            timeline.join("\n")
        )
    }

    /// What it printed so far, as the person sees it.
    pub fn shown(&self) -> String {
        plain(&self.output.lock().expect("a run's output"))
    }

    /// Whether it has exited.
    pub fn exited(&mut self) -> bool {
        self.child
            .try_wait()
            .expect("wait for skald at a terminal")
            .is_some()
    }

    /// Waits until what it printed passes `seen`, for up to [`RUN_LIMIT`] from its start; gives
    /// what it printed then. Fails if it exits first.
    #[track_caller]
    pub fn wait_for(&mut self, what: &str, seen: impl Fn(&str) -> bool) -> String {
        loop {
            let shown = self.shown();
            if seen(&shown) {
                return shown;
            }
            assert!(
                !self.exited(),
                "skald exited without showing {what}: {}",
                self.diagnostic()
            );
            assert!(
                self.started.elapsed() < RUN_LIMIT,
                "skald did not show {what}: {}",
                self.diagnostic()
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Waits until it shows `part`, as [`Terminal::wait_for`] does.
    #[track_caller]
    pub fn wait_to_show(&mut self, part: &str) -> String {
        self.wait_for(part, |shown| shown.contains(part))
    }

    /// The person presses Enter; a run that no longer reads its terminal does not see it.
    pub fn enter(&self) {
        self.type_line("");
    }

    /// The person types `text` and presses Enter; a run that no longer reads its terminal does not
    /// see it.
    pub fn type_line(&self, text: &str) {
        let line = format!("{text}\r");
        write_input(&self.input, &self.events, self.started, line.as_bytes());
    }

    /// The person ends the terminal's input at the start of a line: Ctrl-D on Unix, Ctrl-Z and
    /// Enter on Windows.
    // Only the tests of account fingerprints end it.
    #[allow(dead_code)]
    pub fn end_input(&self) {
        let end: &[u8] = if cfg!(windows) { b"\x1a\r" } else { b"\x04" };
        write_input(&self.input, &self.events, self.started, end);
    }

    /// Kills it at once, as the system does, without a signal it can handle; gives what it
    /// printed until then.
    // Only Windows kills it through the run's handle.
    #[cfg_attr(unix, allow(unused_mut))]
    pub fn kill(mut self) -> Shown {
        #[cfg(unix)]
        if let Some(pid) = self.child.process_id() {
            let pid = i32::try_from(pid).expect("a process ID");
            // SAFETY: `pid` is the run's own process, which has not been waited for.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        // TerminateProcess.
        #[cfg(not(unix))]
        let _ = self.child.kill();
        self.finish()
    }

    /// Waits for it to exit, for up to [`RUN_LIMIT`] from its start; gives what it printed.
    pub fn finish(mut self) -> Shown {
        let deadline = self.started + RUN_LIMIT;
        let mut status = self.child.try_wait().expect("wait for skald at a terminal");
        while status.is_none() {
            assert!(
                Instant::now() < deadline,
                "skald ran for more than {RUN_LIMIT:?}: {}",
                self.diagnostic()
            );
            thread::sleep(Duration::from_millis(50));
            status = self.child.try_wait().expect("wait for skald at a terminal");
        }
        self.close();
        Shown {
            success: status.is_some_and(|status| status.success()),
            text: self.shown(),
        }
    }

    /// Closes the terminal and reads the rest of what the run printed.
    fn close(&mut self) {
        drop(self.master.take());
        if let Some(reader) = self.reader.take() {
            let deadline = Instant::now() + DRAIN;
            while !reader.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            if reader.is_finished() {
                reader.join().expect("read the terminal");
            }
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            #[cfg(unix)]
            if let Some(pid) = self.child.process_id()
                && let Ok(pid) = i32::try_from(pid)
            {
                // SAFETY: as in `kill`.
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        self.close();
    }
}

/// Writes `bytes` on a run's terminal `input`, noting it in its `events`; a run that no longer
/// reads its terminal does not see them.
fn write_input(
    input: &Mutex<Box<dyn Write + Send>>,
    events: &Mutex<Vec<Event>>,
    started: Instant,
    bytes: &[u8],
) {
    let mut input = input.lock().expect("the terminal's input");
    let _ = input.write_all(bytes).and_then(|()| input.flush());
    let went = Event::Input(started.elapsed(), bytes.to_vec());
    events.lock().expect("a run's events").push(went);
}

/// `bytes` with each control character escaped, as in a Rust string.
fn escaped(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).escape_debug().to_string()
}

/// How many times `needle` is in `haystack`.
fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

/// `bytes`, as a terminal shows them: without control sequences, a cursor moved forward as spaces
/// and one moved to another row as a new line.
pub fn plain(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            0x1b if bytes.get(at + 1) == Some(&b'[') => {
                let start = at + 2;
                let mut end = start;
                while end < bytes.len() && !(0x40..=0x7e).contains(&bytes[end]) {
                    end += 1;
                }
                let params = String::from_utf8_lossy(&bytes[start..end.min(bytes.len())]);
                let first = params
                    .trim_start_matches('?')
                    .split(';')
                    .next()
                    .and_then(|n| n.parse::<usize>().ok());
                match bytes.get(end) {
                    Some(b'C') => out.extend(std::iter::repeat_n(b' ', first.unwrap_or(1))),
                    Some(b'H' | b'f' | b'E' | b'B') => out.push(b'\n'),
                    _ => {}
                }
                at = end + 1;
            }
            0x1b if bytes.get(at + 1) == Some(&b']') => {
                let mut end = at + 2;
                while end < bytes.len()
                    && bytes[end] != 0x07
                    && !(bytes[end] == 0x1b && bytes.get(end + 1) == Some(&b'\\'))
                {
                    end += 1;
                }
                at = if bytes.get(end) == Some(&0x07) {
                    end + 1
                } else {
                    end + 2
                };
            }
            0x1b if matches!(bytes.get(at + 1), Some(b'(' | b')' | b'*' | b'+')) => at += 3,
            0x1b => at += 2,
            b'\r' | 0x07 | 0x08 | 0x00 => at += 1,
            byte => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
