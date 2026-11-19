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

/// A run of `skald` at a terminal.
pub struct Terminal {
    child: Box<dyn Child + Send + Sync>,
    /// Dropped once the run exits: on Windows, closing the pseudo-console ends its output.
    master: Option<Box<dyn MasterPty + Send>>,
    input: Arc<Mutex<Box<dyn Write + Send>>>,
    output: Arc<Mutex<Vec<u8>>>,
    reader: Option<JoinHandle<()>>,
    started: Instant,
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
        let reader = {
            let (output, input) = (Arc::clone(&output), Arc::clone(&input));
            thread::spawn(move || {
                let mut buffer = [0; 4096];
                let mut asked = 0;
                while let Ok(read) = reader.read(&mut buffer) {
                    if read == 0 {
                        break;
                    }
                    let mut output = output.lock().expect("a run's output");
                    output.extend_from_slice(&buffer[..read]);
                    // A pseudo-console asks where the cursor is before it shows anything: it is at
                    // the start, as on a new terminal.
                    let queries = count(&output, b"\x1b[6n");
                    while asked < queries {
                        asked += 1;
                        let mut input = input.lock().expect("the terminal's input");
                        let _ = input.write_all(b"\x1b[1;1R").and_then(|()| input.flush());
                    }
                }
            })
        };
        Terminal {
            child,
            master: Some(pair.master),
            input,
            output,
            reader: Some(reader),
            started: Instant::now(),
        }
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
    pub fn wait_for(&mut self, what: &str, seen: impl Fn(&str) -> bool) -> String {
        loop {
            let shown = self.shown();
            if seen(&shown) {
                return shown;
            }
            assert!(
                !self.exited(),
                "skald exited without showing {what}: {shown}"
            );
            assert!(
                self.started.elapsed() < RUN_LIMIT,
                "skald did not show {what}: {shown}"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Waits until it shows `part`, as [`Terminal::wait_for`] does.
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
        let mut input = self.input.lock().expect("the terminal's input");
        let _ = input
            .write_all(format!("{text}\r").as_bytes())
            .and_then(|()| input.flush());
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
                self.shown()
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
