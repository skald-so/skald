//! An ssh-agent of the tests' own. Unix only: on Windows, OpenSSH's ssh-agent is a system service
//! that tests do not run a copy of.

use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::machine::Machine;
use crate::ssh_kit::{SSH_AUTH_SOCK, SshKey, openssh, openssh_tool, succeed};

/// How long the tests' ssh-agent may take to listen.
const LISTEN: Duration = Duration::from_secs(10);

/// An ssh-agent of the tests' own, listening on a socket in a directory of its own, which is
/// removed with it on drop.
pub struct Agent {
    child: Child,
    dir: PathBuf,
    pub socket: PathBuf,
}

impl Agent {
    /// Starts one and waits until it listens.
    pub fn start() -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        // A Unix socket's path holds at most about 100 bytes.
        let temp = std::env::temp_dir();
        let base = if temp.as_os_str().len() < 60 {
            temp
        } else {
            PathBuf::from("/tmp")
        };
        let dir = base.join(format!(
            "skald-ssh-agent-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("make the ssh-agent's directory");
        let socket = dir.join("agent");
        let child = Command::new(openssh_tool("ssh-agent"))
            .env_clear()
            .arg("-D")
            .arg("-a")
            .arg(&socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("run ssh-agent");
        let agent = Agent { child, dir, socket };
        let started = Instant::now();
        while !agent.socket.exists() {
            assert!(started.elapsed() < LISTEN, "ssh-agent does not listen");
            thread::sleep(Duration::from_millis(20));
        }
        agent
    }

    /// Adds `key`, unencrypted, then removes its files: only the agent holds it.
    pub fn hold(&self, key: &SshKey) {
        succeed(
            openssh("ssh-add")
                .env(SSH_AUTH_SOCK, &self.socket)
                .arg(&key.path),
        );
        key.remove_files();
    }

    /// `machine` with its runs reaching this agent.
    pub fn serving(&self, machine: Machine) -> Machine {
        machine.with_env(SSH_AUTH_SOCK, &self.socket)
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.dir);
    }
}
