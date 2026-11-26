//! Every process an executor's agent starts, which a run ends at once: on Unix, the agent's process
//! group and each process that keeps the run's marker variable, which the agent's commands inherit
//! also in a session or group of their own, as Codex starts them; on macOS also each process that
//! holds the run's marker pipe, as macOS hides the environment of its own programs; on Windows, a
//! job object.

pub use imp::{Tree, prepare};

/// The variable that marks each process of a run on Unix.
#[cfg(unix)]
const MARKER: &str = "SKALD_EXECUTOR_RUN";

#[cfg(unix)]
mod imp {
    use std::io;
    use std::os::unix::process::CommandExt as _;
    use std::process::{Child, Command};
    use std::thread;
    use std::time::Duration;

    use super::MARKER;

    /// How many times [`Tree::end`] looks for marked processes, the ones killed meanwhile
    /// starting none.
    const ROUNDS: usize = 50;

    /// Sets up `command` so that every process it starts belongs to its run, marked by `marker`
    /// and, on macOS, by the returned pipe, which [`Tree::new`] takes.
    pub fn prepare(command: &mut Command, marker: &str) -> io::Result<Mark> {
        command.process_group(0).env(MARKER, marker);
        Mark::new(command)
    }

    #[derive(Debug)]
    pub struct Tree {
        /// The agent's process ID, which is its group's.
        pid: libc::pid_t,
        /// The marker variable as a process's environment holds it.
        entry: Vec<u8>,
        mark: Mark,
    }

    impl Tree {
        /// The processes of the run whose agent is `child`, marked by `marker` and `mark`.
        pub fn new(child: &Child, marker: &str, mark: Mark) -> io::Result<Tree> {
            let pid = libc::pid_t::try_from(child.id())
                .map_err(|_| io::Error::other("the agent's process ID does not fit pid_t"))?;
            let entry = format!("{MARKER}={marker}").into_bytes();
            Ok(Tree { pid, entry, mark })
        }

        /// Kills the agent's group, then each marked process until none is left. Until the agent
        /// is reaped, its ID, and so its group's, names no other process.
        pub fn end(&self) {
            // SAFETY: kill takes any process group ID; an empty group fails with ESRCH.
            unsafe {
                libc::kill(-self.pid, libc::SIGKILL);
            }
            for _ in 0..ROUNDS {
                let marked = marked(&self.entry, &self.mark);
                if marked.is_empty() {
                    break;
                }
                for pid in marked {
                    // SAFETY: kill takes any process ID.
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                    }
                }
                thread::sleep(Duration::from_millis(10));
            }
        }

        /// Waits for the agent to exit, leaving it to be reaped.
        pub fn exited(&self) -> io::Result<()> {
            loop {
                // SAFETY: siginfo_t is plain data that waitid fills.
                let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                let options = libc::WEXITED | libc::WNOWAIT;
                // SAFETY: `info` outlives the call; WNOWAIT leaves the agent unreaped.
                let waited = unsafe {
                    libc::waitid(libc::P_PID, self.pid as libc::id_t, &mut info, options)
                };
                if waited == 0 {
                    return Ok(());
                }
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
    }

    /// The live processes whose environment holds `entry`, a variable as `NAME=value`, or that
    /// hold `mark`.
    fn marked(entry: &[u8], mark: &Mark) -> Vec<libc::pid_t> {
        let me = std::process::id() as libc::pid_t;
        let pids = processes().into_iter().filter(|&pid| pid > 0 && pid != me);
        pids.filter(|&pid| {
            environment(pid).is_some_and(|vars| holds(&vars, entry)) || mark.held_by(pid)
        })
        .collect()
    }

    /// Elsewhere the variable alone marks a run's processes.
    #[cfg(not(target_os = "macos"))]
    #[derive(Debug)]
    pub struct Mark;

    #[cfg(not(target_os = "macos"))]
    impl Mark {
        fn new(_command: &mut Command) -> io::Result<Mark> {
            Ok(Mark)
        }

        fn held_by(&self, _pid: libc::pid_t) -> bool {
            false
        }
    }

    /// A pipe whose write end the agent inherits without close-on-exec, so every process it starts
    /// holds it unless it closes it. Both ends stay open here while the run lives, so the write
    /// end's kernel handle names no other pipe meanwhile.
    #[cfg(target_os = "macos")]
    #[derive(Debug)]
    pub struct Mark {
        _read: std::os::fd::OwnedFd,
        _write: std::os::fd::OwnedFd,
        /// The write end's `pipe_handle`.
        handle: u64,
    }

    /// `PROC_PIDFDPIPEINFO` of `sys/proc_info.h`, which libc lacks.
    #[cfg(target_os = "macos")]
    const PROC_PIDFDPIPEINFO: libc::c_int = 6;

    /// `struct pipe_fdinfo` of `sys/proc_info.h`, which libc lacks; only `pipe_handle` is read.
    #[cfg(target_os = "macos")]
    #[repr(C)]
    struct PipeFdInfo {
        /// `struct proc_fileinfo`.
        _pfi: [u8; 24],
        _pipe_stat: libc::vinfo_stat,
        pipe_handle: u64,
        _pipe_peerhandle: u64,
        _pipe_status: libc::c_int,
        _rfu_1: libc::c_int,
    }

    #[cfg(target_os = "macos")]
    const _: () = assert!(size_of::<PipeFdInfo>() == 184);

    #[cfg(target_os = "macos")]
    impl Mark {
        fn new(command: &mut Command) -> io::Result<Mark> {
            use std::os::fd::{AsRawFd as _, OwnedFd};

            // Both ends close on exec: only the agent gets the write end, below.
            let (read, write) = io::pipe()?;
            let (read, write) = (OwnedFd::from(read), OwnedFd::from(write));
            let fd = write.as_raw_fd();
            let handle = pipe_handle(std::process::id() as libc::pid_t, fd)
                .ok_or_else(io::Error::last_os_error)?;
            // SAFETY: fcntl is async-signal-safe, and `fd` stays open in the parent until the
            // child has exec'd.
            unsafe {
                command.pre_exec(move || {
                    if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            Ok(Mark {
                _read: read,
                _write: write,
                handle,
            })
        }

        /// Whether process `pid` holds this pipe's write end.
        fn held_by(&self, pid: libc::pid_t) -> bool {
            pipes(pid).any(|fd| pipe_handle(pid, fd) == Some(self.handle))
        }
    }

    /// The pipe descriptors of process `pid`: none for another user's process.
    #[cfg(target_os = "macos")]
    fn pipes(pid: libc::pid_t) -> impl Iterator<Item = libc::c_int> {
        // SAFETY: a null buffer asks only for the size.
        let size =
            unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
        let count = usize::try_from(size).unwrap_or(0) / size_of::<libc::proc_fdinfo>();
        let mut fds: Vec<libc::proc_fdinfo> = Vec::with_capacity(count + 16);
        let capacity = (fds.capacity() * size_of::<libc::proc_fdinfo>()) as libc::c_int;
        // SAFETY: the buffer holds `capacity` bytes.
        let size = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDLISTFDS,
                0,
                fds.as_mut_ptr().cast(),
                capacity,
            )
        };
        let filled = usize::try_from(size).unwrap_or(0) / size_of::<libc::proc_fdinfo>();
        // SAFETY: proc_pidinfo filled `filled` entries, at most the capacity.
        unsafe { fds.set_len(filled.min(fds.capacity())) };
        fds.into_iter()
            .filter(|fd| fd.proc_fdtype == libc::PROX_FDTYPE_PIPE as u32)
            .map(|fd| fd.proc_fd)
    }

    /// The `pipe_handle` of descriptor `fd` of process `pid`, if it is a pipe.
    #[cfg(target_os = "macos")]
    fn pipe_handle(pid: libc::pid_t, fd: libc::c_int) -> Option<u64> {
        let mut info = std::mem::MaybeUninit::<PipeFdInfo>::zeroed();
        let size = size_of::<PipeFdInfo>() as libc::c_int;
        // SAFETY: `info` holds `size` bytes.
        let read = unsafe {
            libc::proc_pidfdinfo(pid, fd, PROC_PIDFDPIPEINFO, info.as_mut_ptr().cast(), size)
        };
        // SAFETY: zeroed is a valid PipeFdInfo, and a full read fills it.
        (read == size).then(|| unsafe { info.assume_init() }.pipe_handle)
    }

    /// Whether `vars`, variables each ended by NUL, holds `entry`.
    fn holds(vars: &[u8], entry: &[u8]) -> bool {
        vars.split(|&byte| byte == 0).any(|var| var == entry)
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn processes() -> Vec<libc::pid_t> {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        let names = entries.filter_map(|entry| entry.ok()?.file_name().into_string().ok());
        names.filter_map(|name| name.parse().ok()).collect()
    }

    /// The process's environment as `/proc` gives it: empty for a zombie.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn environment(pid: libc::pid_t) -> Option<Vec<u8>> {
        std::fs::read(format!("/proc/{pid}/environ")).ok()
    }

    #[cfg(target_os = "macos")]
    fn processes() -> Vec<libc::pid_t> {
        // SAFETY: a null buffer asks only for the count.
        let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
        let Ok(count) = usize::try_from(count) else {
            return Vec::new();
        };
        let mut pids: Vec<libc::pid_t> = vec![0; count + 64];
        let size = (pids.len() * size_of::<libc::pid_t>()) as libc::c_int;
        // SAFETY: the buffer holds `size` bytes.
        let count = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), size) };
        pids.truncate(usize::try_from(count).unwrap_or(0));
        pids
    }

    /// The process's arguments and environment, each ended by NUL, as `KERN_PROCARGS2` gives them:
    /// none for a zombie or another user's process.
    #[cfg(target_os = "macos")]
    fn environment(pid: libc::pid_t) -> Option<Vec<u8>> {
        let mut most: libc::c_int = 0;
        let mut size = size_of::<libc::c_int>();
        let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
        // SAFETY: `most` holds `size` bytes.
        let read = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                2,
                (&raw mut most).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if read != 0 {
            return None;
        }
        let mut args = vec![0u8; usize::try_from(most).ok()?];
        let mut size = args.len();
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
        // SAFETY: `args` holds `size` bytes.
        let read = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                args.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if read != 0 {
            return None;
        }
        args.truncate(size);
        Some(args)
    }

    /// Elsewhere only the agent's group ends.
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    fn processes() -> Vec<libc::pid_t> {
        Vec::new()
    }

    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    fn environment(_pid: libc::pid_t) -> Option<Vec<u8>> {
        None
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn finds_a_variable_among_others() {
            let vars = b"A=1\0SKALD_EXECUTOR_RUN=x\0B=2\0";
            assert!(holds(vars, b"SKALD_EXECUTOR_RUN=x"));
            assert!(!holds(vars, b"SKALD_EXECUTOR_RUN=y"));
            assert!(!holds(b"XSKALD_EXECUTOR_RUN=x\0", b"SKALD_EXECUTOR_RUN=x"));
        }

        #[test]
        fn this_process_is_listed_with_its_environment() {
            let me = std::process::id() as libc::pid_t;
            assert!(processes().contains(&me));
            let path = std::env::var("PATH").unwrap_or_default();
            let vars = environment(me).expect("this process's environment");
            assert!(holds(&vars, format!("PATH={path}").as_bytes()));
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::os::windows::io::AsRawHandle as _;
    use std::process::{Child, Command};
    use std::ptr;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_FAILED};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    };
    use windows_sys::Win32::System::Threading::{INFINITE, WaitForSingleObject};

    pub fn prepare(_command: &mut Command, _marker: &str) -> io::Result<Mark> {
        Ok(Mark)
    }

    /// The job object alone holds a run's processes.
    #[derive(Debug)]
    pub struct Mark;

    /// A job object that holds the agent and every process it starts, none of which may break
    /// away; closing it kills what is left.
    #[derive(Debug)]
    pub struct Tree {
        job: HANDLE,
        /// The agent's process handle, which its `Child` owns and keeps open while the run lives.
        agent: HANDLE,
    }

    // SAFETY: the handles may be used from any thread; only drop closes the job's.
    unsafe impl Send for Tree {}
    unsafe impl Sync for Tree {}

    impl Tree {
        pub fn new(child: &Child, _marker: &str, _mark: Mark) -> io::Result<Tree> {
            // SAFETY: an unnamed job object with the default security.
            let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
            if job.is_null() {
                return Err(io::Error::last_os_error());
            }
            let agent = child.as_raw_handle();
            let tree = Tree { job, agent };
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
            if set == 0 || unsafe { AssignProcessToJobObject(job, agent) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(tree)
        }

        /// Kills every process of the job.
        pub fn end(&self) {
            // SAFETY: the handle is open until drop.
            unsafe {
                TerminateJobObject(self.job, 1);
            }
        }

        /// Waits for the agent to exit.
        pub fn exited(&self) -> io::Result<()> {
            // SAFETY: the agent's handle is open while the run lives.
            match unsafe { WaitForSingleObject(self.agent, INFINITE) } {
                WAIT_FAILED => Err(io::Error::last_os_error()),
                _ => Ok(()),
            }
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            // SAFETY: the job's handle is open and closed only here.
            unsafe {
                CloseHandle(self.job);
            }
        }
    }
}
