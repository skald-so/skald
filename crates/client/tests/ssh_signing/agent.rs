//! A key ssh-agent holds signs, without a terminal and as an agent, where an encrypted key file is
//! not used. Unix only: on Windows, OpenSSH's ssh-agent is a system service that tests do not run
//! a copy of.

use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::machine::Machine;
use crate::{
    ADA_GITHUB, ADA_LOGIN, Kind, PASSPHRASE, SSH_AUTH_SOCK, Setup, SshKey, assert_asks_nothing,
    assert_member_sees_passed, bob_with, keys_dir, new_key, openssh, openssh_tool, succeed,
};

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

/// `setup` with ssh-agent holding a new key it lists as a signing key of Ada's GitHub user, beside
/// the keys `authentication`; gives the key.
fn holding(agent: &Agent, mut setup: Setup, authentication: &[&SshKey]) -> (Setup, SshKey) {
    setup.machine = agent.serving(setup.machine);
    let held = new_key(&keys_dir(&setup.machine).join("held"), Kind::Ed25519, "");
    setup.list(authentication, &[&held]);
    agent.hold(&held);
    (setup, held)
}

/// A key only the machine's ssh-agent holds, which GitHub lists as a signing key of Ada's GitHub
/// user, and an unlisted key file in her SSH directory: signing in without a terminal, her first
/// device's certificate carries a signature by the agent's key, the run says so, and Bob's device
/// checks it as passed.
#[test]
fn without_a_terminal_a_key_ssh_agent_holds_signs() {
    let agent = Agent::start();
    let (setup, held) = holding(&agent, Setup::new(""), &[]);

    let signed = setup.sign_up(&[]);

    signed.assert_signed((ADA_GITHUB, ADA_LOGIN), &held, &[&signed.key]);
    let bob = bob_with(&signed.world, &signed.api.url(), &signed.ada);
    assert_member_sees_passed(&bob, signed.ada.device, ADA_LOGIN);
}

/// A listed key the machine's ssh-agent holds, and a listed encrypted key file in Ada's SSH
/// directory: signing in as an agent at a terminal, the certificate carries a signature by the
/// agent's key, not by the file's, and nothing asks for a passphrase.
#[test]
fn as_an_agent_a_key_ssh_agent_holds_signs_and_an_encrypted_key_file_is_not_used() {
    let agent = Agent::start();
    let setup = Setup::new(PASSPHRASE);
    let file = setup.key.clone();
    let (setup, held) = holding(&agent, setup, &[&file]);

    let signed = setup.sign_up_at_terminal(true, &[]);

    signed.assert_signed((ADA_GITHUB, ADA_LOGIN), &held, &[&file]);
    assert_asks_nothing(&signed.shown);
}
