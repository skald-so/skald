//! A run of an executor's agent: its prompt on stdin, which then closes, so no person answers it;
//! the agent and every process it starts in a process group on Unix and a job object on Windows,
//! which stopping the run ends, as does the agent's own end.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use super::Outcome;

/// What a stopped run gives as its answer.
const STOPPED: &str = "stopped before it ended";

/// A running agent of an executor. Dropping it ends the agent with every process it started.
#[derive(Debug)]
pub struct Run {
    child: Option<Child>,
    tree: tree::Tree,
    streams: Option<Streams>,
    stopped: AtomicBool,
    outcome: fn(&Output) -> Outcome,
    /// Removed once the run ends.
    _scratch: Option<Scratch>,
}

/// The threads that write the prompt and read the agent's output.
#[derive(Debug)]
struct Streams {
    stdin: JoinHandle<()>,
    stdout: JoinHandle<Vec<u8>>,
    stderr: JoinHandle<Vec<u8>>,
}

impl Run {
    /// Starts `command` with `prompt` on its stdin; [`Run::wait`] reads how it ended from its
    /// output with `outcome`. `scratch` lasts as long as the run.
    pub(crate) fn start(
        mut command: Command,
        prompt: String,
        outcome: fn(&Output) -> Outcome,
        scratch: Option<Scratch>,
    ) -> io::Result<Run> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        tree::prepare(&mut command);
        let mut child = command.spawn()?;
        let tree = match tree::Tree::of(&child) {
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
            child: Some(child),
            tree,
            streams: Some(streams),
            stopped: AtomicBool::new(false),
            outcome,
            _scratch: scratch,
        })
    }

    /// Ends the agent and every process it started; [`Run::wait`] then gives the run as unfinished.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.tree.end();
    }

    /// Waits for the agent to end, ends every process it left, and gives how the run ended.
    pub fn wait(mut self) -> Outcome {
        let mut child = self.child.take().expect("a run is waited for once");
        let status = self.tree.ended(&mut child);
        let streams = self.streams.take().expect("a run is waited for once");
        let _ = streams.stdin.join();
        let stdout = streams.stdout.join().unwrap_or_default();
        let stderr = streams.stderr.join().unwrap_or_default();
        if self.stopped.load(Ordering::SeqCst) {
            return unfinished(STOPPED.to_owned());
        }
        match status {
            Ok(status) => (self.outcome)(&Output {
                status,
                stdout,
                stderr,
            }),
            Err(error) => unfinished(format!("cannot wait for the agent to end: {error}")),
        }
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            self.tree.end();
            let _ = child.wait();
        }
    }
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

/// A directory of a run's own in this process's temporary directory, which only this user opens on
/// Unix; removed on drop.
#[derive(Debug)]
pub(crate) struct Scratch(PathBuf);

impl Scratch {
    /// A new one whose name starts with `prefix`.
    pub fn new(prefix: &str) -> io::Result<Self> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let nanos = (SystemTime::now().duration_since(UNIX_EPOCH))
            .map(|since| since.as_nanos())
            .unwrap_or_default();
        loop {
            let next = NEXT.fetch_add(1, Ordering::Relaxed);
            let name = format!("{prefix}-{}-{nanos:x}-{next}", std::process::id());
            let path = std::env::temp_dir().join(name);
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

/// The agent's process group, which it leads: every process it starts joins it unless it leaves.
#[cfg(unix)]
mod tree {
    use std::io;
    use std::os::unix::process::CommandExt as _;
    use std::process::{Child, Command, ExitStatus};

    #[derive(Debug)]
    pub struct Tree(libc::pid_t);

    pub fn prepare(command: &mut Command) {
        command.process_group(0);
    }

    impl Tree {
        pub fn of(child: &Child) -> io::Result<Tree> {
            let pid = libc::pid_t::try_from(child.id())
                .map_err(|_| io::Error::other("the agent's process ID does not fit pid_t"))?;
            Ok(Tree(pid))
        }

        /// Kills every process of the group. Until the agent is reaped, its ID, and so the
        /// group's, names no other process.
        pub fn end(&self) {
            // SAFETY: kill takes any process group ID; an empty group fails with ESRCH.
            unsafe {
                libc::kill(-self.0, libc::SIGKILL);
            }
        }

        /// Waits for the agent to exit, kills what is left of its group before reaping it, then
        /// reaps it.
        pub fn ended(&self, child: &mut Child) -> io::Result<ExitStatus> {
            loop {
                // SAFETY: siginfo_t is plain data that waitid fills.
                let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                let options = libc::WEXITED | libc::WNOWAIT;
                // SAFETY: `info` outlives the call; WNOWAIT leaves the agent to `child.wait`.
                let waited =
                    unsafe { libc::waitid(libc::P_PID, self.0 as libc::id_t, &mut info, options) };
                if waited == 0 {
                    break;
                }
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
            self.end();
            child.wait()
        }
    }
}

/// A job object that holds the agent and every process it starts, unless one breaks away; closing
/// it kills what is left.
#[cfg(windows)]
mod tree {
    use std::io;
    use std::os::windows::io::AsRawHandle as _;
    use std::process::{Child, Command, ExitStatus};
    use std::ptr;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    };

    #[derive(Debug)]
    pub struct Tree(HANDLE);

    // SAFETY: a job object's handle may be used and closed from any thread.
    unsafe impl Send for Tree {}
    unsafe impl Sync for Tree {}

    pub fn prepare(_command: &mut Command) {}

    impl Tree {
        pub fn of(child: &Child) -> io::Result<Tree> {
            // SAFETY: an unnamed job object with the default security.
            let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
            if job.is_null() {
                return Err(io::Error::last_os_error());
            }
            let tree = Tree(job);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: `limits` outlives the call and has the size given.
            let set = unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            // SAFETY: the child's handle is open while `child` lives.
            if set == 0 || unsafe { AssignProcessToJobObject(job, child.as_raw_handle()) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(tree)
        }

        /// Kills every process of the job.
        pub fn end(&self) {
            // SAFETY: the handle is open until drop.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }

        /// Waits for the agent to exit, then kills what is left of the job.
        pub fn ended(&self, child: &mut Child) -> io::Result<ExitStatus> {
            let status = child.wait();
            self.end();
            status
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            // SAFETY: the handle is open and closed only here.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
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

    #[test]
    fn a_run_gets_its_prompt_and_gives_its_outcome() {
        let run = Run::start(shell("cat; exit 3"), "prompt".to_owned(), told, None).unwrap();
        assert_eq!(run.wait(), unfinished("prompt".to_owned()));
    }

    /// A process the agent left behind, holding its output open, does not keep the run waiting.
    #[test]
    fn a_run_ends_what_its_agent_left() {
        let script = "sleep 60 & echo done";
        let start = Instant::now();
        let run = Run::start(shell(script), String::new(), told, None).unwrap();
        let outcome = run.wait();
        assert!(start.elapsed() < Duration::from_secs(30), "{outcome:?}");
        assert_eq!(
            outcome,
            Outcome {
                answer: "done\n".to_owned(),
                finished: true
            }
        );
    }

    #[test]
    fn a_stopped_run_is_unfinished() {
        let run = Run::start(shell("sleep 60"), String::new(), told, None).unwrap();
        run.stop();
        assert_eq!(run.wait(), unfinished(STOPPED.to_owned()));
    }

    #[test]
    fn a_scratch_lasts_as_long_as_its_run() {
        let scratch = Scratch::new("skald-run-test").unwrap();
        let path = scratch.path().to_owned();
        let run = Run::start(shell("true"), String::new(), told, Some(scratch)).unwrap();
        assert!(path.is_dir());
        run.wait();
        assert!(!path.exists());
    }
}
