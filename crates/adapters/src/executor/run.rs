//! A run of an executor's agent: its prompt on stdin, which then closes, so no person answers it.
//! Stopping the run, the agent's own end and dropping the run end every process the agent started
//! ([`tree`](super::tree)).

use std::fmt;
use std::fs;
use std::hash::{BuildHasher, RandomState};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

use super::Outcome;
use super::tree::{self, Tree};

/// What a stopped run gives as its answer.
const STOPPED: &str = "stopped before it ended";

/// What runs once a run's agent and every process it started have ended.
pub(crate) type After = Box<dyn FnOnce() + Send>;

/// A running agent of an executor. One thread may [`Run::stop`] it while another waits for it.
/// Dropping it ends the agent with every process it started.
pub struct Run {
    tree: Tree,
    agent: Mutex<Agent>,
    /// Taken by the one thread that waits; holds the outcome once the run has ended.
    ended: Mutex<Option<Outcome>>,
    stopped: AtomicBool,
    outcome: fn(&Output) -> Outcome,
}

/// The agent's process, and what lasts while it runs.
struct Agent {
    child: Child,
    /// Whether `child` is reaped: from then on its ID may name another process.
    reaped: bool,
    streams: Option<Streams>,
    after: Option<After>,
}

/// The threads that write the prompt and read the agent's output.
struct Streams {
    stdin: JoinHandle<()>,
    stdout: JoinHandle<Vec<u8>>,
    stderr: JoinHandle<Vec<u8>>,
}

impl fmt::Debug for Run {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Run")
            .field("tree", &self.tree)
            .finish_non_exhaustive()
    }
}

impl Run {
    /// Starts `command` with `prompt` on its stdin; [`Run::wait`] reads how it ended from its
    /// output with `outcome`. `after` runs once the agent and every process it started have ended.
    pub(crate) fn start(
        mut command: Command,
        prompt: String,
        outcome: fn(&Output) -> Outcome,
        after: Option<After>,
    ) -> io::Result<Run> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let marker = unique("run");
        let mark = tree::prepare(&mut command, &marker)?;
        let mut child = command.spawn()?;
        let tree = match Tree::new(&child, &marker, mark) {
            Ok(tree) => tree,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let stdin = child.stdin.take();
        let streams = Streams {
            stdin: thread::spawn(move || {
                if let Some(mut stdin) = stdin {
                    // An agent that ends without reading it closes the pipe.
                    let _ = stdin.write_all(prompt.as_bytes());
                }
            }),
            stdout: read(child.stdout.take()),
            stderr: read(child.stderr.take()),
        };
        Ok(Run {
            tree,
            agent: Mutex::new(Agent {
                child,
                reaped: false,
                streams: Some(streams),
                after,
            }),
            ended: Mutex::new(None),
            stopped: AtomicBool::new(false),
            outcome,
        })
    }

    /// Ends the agent and every process it started, unless it has ended; [`Run::wait`] then gives
    /// the run as unfinished.
    pub fn stop(&self) {
        let agent = locked(&self.agent);
        if !agent.reaped {
            self.stopped.store(true, Ordering::SeqCst);
            self.tree.end();
        }
    }

    /// Waits for the agent to end, ends every process it left, and gives how the run ended; the
    /// same each time once it has.
    pub fn wait(&self) -> Outcome {
        let mut ended = locked(&self.ended);
        if let Some(outcome) = &*ended {
            return outcome.clone();
        }
        // Without the agent's lock, so that `stop` works meanwhile; the agent stays unreaped.
        let exited = self.tree.exited();
        let (status, streams, after) = {
            let mut agent = locked(&self.agent);
            let status = exited.and_then(|()| reap(&self.tree, &mut agent));
            (status, agent.streams.take(), agent.after.take())
        };
        let (stdout, stderr) = match streams {
            Some(streams) => {
                let _ = streams.stdin.join();
                let stdout = streams.stdout.join().unwrap_or_default();
                (stdout, streams.stderr.join().unwrap_or_default())
            }
            None => Default::default(),
        };
        if let Some(after) = after {
            after();
        }
        let outcome = match status {
            _ if self.stopped.load(Ordering::SeqCst) => unfinished(STOPPED.to_owned()),
            Ok(status) => (self.outcome)(&Output {
                status,
                stdout,
                stderr,
            }),
            Err(error) => unfinished(format!("cannot wait for the agent to end: {error}")),
        };
        *ended = Some(outcome.clone());
        outcome
    }
}

/// Ends whatever the agent left, then reaps it.
fn reap(tree: &Tree, agent: &mut Agent) -> io::Result<ExitStatus> {
    tree.end();
    let status = agent.child.wait();
    agent.reaped = true;
    status
}

impl Drop for Run {
    fn drop(&mut self) {
        let agent = self.agent.get_mut().unwrap_or_else(PoisonError::into_inner);
        if !agent.reaped {
            let _ = reap(&self.tree, agent);
        }
        if let Some(after) = agent.after.take() {
            after();
        }
    }
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn unfinished(answer: String) -> Outcome {
    Outcome {
        answer,
        finished: false,
    }
}

/// Reads `stream` to its end on a thread of its own.
fn read(stream: Option<impl Read + Send + 'static>) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut stream) = stream {
            let _ = stream.read_to_end(&mut bytes);
        }
        bytes
    })
}

/// A name no other of this process's names shares, nor, most likely, another process's: `prefix`,
/// the process ID, a random number and a count.
pub(crate) fn unique(prefix: &str) -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let next = NEXT.fetch_add(1, Ordering::Relaxed);
    // Each `RandomState` has keys of its own, from the OS's random generator.
    let random = RandomState::new().hash_one(next);
    format!("{prefix}-{}-{random:016x}-{next}", std::process::id())
}

/// A directory of a run's own in this process's temporary directory, which only this user opens on
/// Unix; removed on drop.
#[derive(Debug)]
pub(crate) struct Scratch(PathBuf);

impl Scratch {
    /// A new one whose name starts with `prefix`.
    pub fn new(prefix: &str) -> io::Result<Self> {
        loop {
            let path = std::env::temp_dir().join(unique(prefix));
            #[cfg(unix)]
            let made = {
                use std::os::unix::fs::DirBuilderExt as _;
                fs::DirBuilder::new().mode(0o700).create(&path)
            };
            #[cfg(not(unix))]
            let made = fs::create_dir(&path);
            match made {
                Ok(()) => return Ok(Scratch(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;

    fn told(output: &Output) -> Outcome {
        Outcome {
            answer: String::from_utf8_lossy(&output.stdout).into_owned(),
            finished: output.status.success(),
        }
    }

    fn shell(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(script);
        command
    }

    fn start(script: &str) -> Run {
        Run::start(shell(script), String::new(), told, None).expect("start the shell")
    }

    /// Waits for `run` on a thread of its own; fails unless it ends within 30 s.
    #[track_caller]
    fn finish(run: Arc<Run>) -> Outcome {
        let (done, ended) = mpsc::channel();
        thread::spawn(move || {
            let _ = done.send(run.wait());
        });
        (ended.recv_timeout(Duration::from_secs(30))).expect("the run did not end within 30 s")
    }

    /// A shell command that starts, in a session of its own, as Codex starts its commands, a
    /// process that appends to `beat` every 100 ms, and leaves it. The process ends by itself once
    /// `beat`'s folder is gone or after 600 beats, well past the tests' waits.
    fn beating_in_its_own_session(beat: &Path) -> String {
        let folder = beat.parent().expect("the beat's folder").display();
        let beating = format!(
            "i=0; while [ -d '{folder}' ] && [ \\$i -lt 600 ]; do printf x >> '{}'; \
             i=\\$((i + 1)); sleep 0.1; done",
            beat.display()
        );
        format!(
            "perl -MPOSIX -e 'POSIX::setsid() or die; exec \"/bin/sh\", \"-c\", $ARGV[0]' \
             \"{beating}\" < /dev/null > /dev/null 2>&1 &"
        )
    }

    fn size(path: &Path) -> u64 {
        fs::metadata(path).map_or(0, |meta| meta.len())
    }

    /// Waits until `beat` has grown to 2 bytes, runs `end`, then fails unless `beat` stays still.
    #[track_caller]
    fn assert_end_stops(beat: &Path, end: impl FnOnce()) {
        let start = Instant::now();
        while size(beat) < 2 {
            assert!(start.elapsed() < Duration::from_secs(20), "no beat");
            thread::sleep(Duration::from_millis(50));
        }
        end();
        thread::sleep(Duration::from_millis(300));
        let stopped = size(beat);
        thread::sleep(Duration::from_millis(1000));
        assert_eq!(
            size(beat),
            stopped,
            "a process the agent started still runs"
        );
    }

    #[test]
    fn a_run_gets_its_prompt_and_gives_its_outcome() {
        let run = Run::start(shell("cat; exit 3"), "prompt".to_owned(), told, None).unwrap();
        assert_eq!(run.wait(), unfinished("prompt".to_owned()));
        assert_eq!(run.wait(), unfinished("prompt".to_owned()));
    }

    /// A process the agent left behind, holding its output open, does not keep the run waiting.
    #[test]
    fn a_run_ends_what_its_agent_left() {
        let begun = Instant::now();
        let outcome = finish(Arc::new(start("sleep 60 & echo done")));
        assert!(begun.elapsed() < Duration::from_secs(30), "{outcome:?}");
        assert_eq!(
            outcome,
            Outcome {
                answer: "done\n".to_owned(),
                finished: true
            }
        );
    }

    /// Stopping a run ends a process its agent started in a session of its own.
    #[test]
    fn stopping_a_run_ends_a_process_in_a_session_of_its_own() {
        let scratch = Scratch::new("skald-run-test").unwrap();
        let beat = scratch.path().join("beat");
        let script = format!("{}\nsleep 60", beating_in_its_own_session(&beat));
        let run = Arc::new(start(&script));
        assert_end_stops(&beat, || {
            run.stop();
            assert_eq!(finish(run), unfinished(STOPPED.to_owned()));
        });
    }

    /// The agent's end ends a process it started in a session of its own.
    #[test]
    fn the_agent_s_end_ends_a_process_in_a_session_of_its_own() {
        let scratch = Scratch::new("skald-run-test").unwrap();
        let beat = scratch.path().join("beat");
        let script = format!(
            "{}\ni=0; while [ ! -s '{}' ] && [ $i -lt 600 ]; do sleep 0.05; i=$((i + 1)); done",
            beating_in_its_own_session(&beat),
            beat.display()
        );
        let run = Arc::new(start(&script));
        assert_end_stops(&beat, || assert!(finish(run).finished));
    }

    /// One thread stops a run while another waits for it.
    #[test]
    fn a_run_stops_while_another_thread_waits() {
        let run = Arc::new(start("sleep 60"));
        let (done, ended) = mpsc::channel();
        let waiting = Arc::clone(&run);
        thread::spawn(move || {
            let _ = done.send(waiting.wait());
        });
        thread::sleep(Duration::from_millis(200));
        assert!(ended.try_recv().is_err(), "the run ended before the stop");
        run.stop();
        let outcome = (ended.recv_timeout(Duration::from_secs(30))).expect("the run did not end");
        assert_eq!(outcome, unfinished(STOPPED.to_owned()));
    }

    /// A stop once the run has ended changes nothing.
    #[test]
    fn a_stop_after_the_end_changes_nothing() {
        let run = start("echo done");
        let outcome = run.wait();
        run.stop();
        assert_eq!(run.wait(), outcome);
        assert!(outcome.finished);
    }

    #[test]
    fn a_run_shows_its_process_tree() {
        let run = start("true");
        run.wait();
        assert!(format!("{run:?}").starts_with("Run { tree: Tree {"));
    }

    #[test]
    fn dropping_a_run_ends_its_agent_and_runs_what_follows() {
        let scratch = Scratch::new("skald-run-test").unwrap();
        let pid_file = scratch.path().join("pid");
        let script = format!("echo $$ > '{}'; exec sleep 60", pid_file.display());
        let followed = Arc::new(AtomicBool::new(false));
        let follows = Arc::clone(&followed);
        let after: After = Box::new(move || follows.store(true, Ordering::SeqCst));
        let run = Run::start(shell(&script), String::new(), told, Some(after)).unwrap();
        let begun = Instant::now();
        while !fs::read_to_string(&pid_file).is_ok_and(|pid| pid.ends_with('\n')) {
            assert!(
                begun.elapsed() < Duration::from_secs(20),
                "the agent did not start"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let pid: libc::pid_t = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();

        drop(run);

        assert!(
            followed.load(Ordering::SeqCst),
            "what follows the run did not run"
        );
        // SAFETY: signal 0 only checks that the process exists.
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        assert!(!alive, "the agent outlived its run");
    }

    #[test]
    fn what_follows_a_run_runs_once_it_ends() {
        let scratch = Scratch::new("skald-run-test").unwrap();
        let path = scratch.path().to_owned();
        let after: After = Box::new(move || drop(scratch));
        let run = Run::start(shell("true"), String::new(), told, Some(after)).unwrap();
        assert!(path.is_dir());
        run.wait();
        assert!(!path.exists());
    }
}
