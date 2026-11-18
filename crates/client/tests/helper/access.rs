//! Who reaches the helper: only the account that runs it. Its socket lives in the local state's
//! directory, where no other account reaches it; a command another account runs on the same state
//! gets nothing through it.

/// What Ada's machine sends.
#[cfg(unix)]
const FIRST: &str = "helper test: the payroll export is ready";
/// What another account tries to send with Ada's state.
#[cfg(target_os = "linux")]
const INTRUDER: &str = "helper test: sent by another account";

/// While a helper runs, the local state's directory holds a socket, at any depth, and neither the
/// group nor others reach it: a directory on the way from the state directory has no search
/// permission for them, or the socket no write permission.
#[cfg(unix)]
#[test]
fn helper_socket_in_the_state_directory_is_reachable_only_by_its_owner() {
    use crate::helped::Helped;
    use crate::talk::Talk;

    let talk = Talk::new(
        &[4_146_051_001, 4_146_051_002],
        &["ada-guards-her-socket", "bob-has-no-socket"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    helped.helper_after_sending(&talk, &[], FIRST);
    let state = talk.ada.machine.state_dir();

    let sockets = unix::sockets(&state);

    assert!(
        !sockets.is_empty(),
        "no socket in the state directory {}",
        state.display()
    );
    for socket in &sockets {
        for (who, search, write) in [("the group", 0o010, 0o020), ("others", 0o001, 0o002)] {
            assert!(
                !unix::reachable(&state, socket, search, write),
                "{who} reach the socket {}",
                socket.display()
            );
        }
    }
}

/// While Ada's helper runs, another OS account, in a Linux container that shares this machine's
/// network and sees Ada's state directory at the same path, runs `skald send` with that state
/// directory: it fails, the front sees nothing sent, Ada's helper keeps running, and Bob's device
/// reads only Ada's message.
#[cfg(target_os = "linux")]
#[test]
fn another_account_gets_nothing_through_the_helper() {
    use crate::fixtures::SEND;
    use crate::helped::{Helped, bob_reads};
    use crate::talk::Talk;

    let mut talk = Talk::new(
        &[4_146_052_001, 4_146_052_002],
        &["ada-shares-a-machine", "bob-reads-ada-only"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    let helper = helped.helper_after_sending(&talk, &[], FIRST);
    let before = talk.front.sent().len();
    let channel = talk.channel.to_string();

    let other = container::run_as_another_account(
        &talk.ada.machine.state_dir(),
        &[SEND, &channel, INTRUDER],
    );

    assert!(!other.success, "another account sent: {other:?}");
    assert_eq!(
        talk.front.sent().len(),
        before,
        "another account's send reached the server"
    );
    assert_eq!(
        helped.helpers(),
        [helper],
        "the helper did not keep running"
    );
    bob_reads(&mut talk, &[FIRST]);
}

#[cfg(unix)]
mod unix {
    use std::fs;
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    /// The sockets under `dir`, at any depth.
    pub fn sockets(dir: &Path) -> Vec<PathBuf> {
        let mut to_visit = vec![dir.to_path_buf()];
        let mut sockets = Vec::new();
        while let Some(next) = to_visit.pop() {
            for entry in fs::read_dir(&next).into_iter().flatten().flatten() {
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_socket() {
                    sockets.push(entry.path());
                } else if kind.is_dir() {
                    to_visit.push(entry.path());
                }
            }
        }
        sockets
    }

    /// Whether the class of accounts whose search permission on a directory is `search` and whose
    /// write permission on a file is `write` reaches `socket` from `state`: it searches every
    /// directory from `state` down to the socket's, and writes to the socket.
    pub fn reachable(state: &Path, socket: &Path, search: u32, write: u32) -> bool {
        let mode = |path: &Path| {
            fs::symlink_metadata(path)
                .unwrap_or_else(|err| panic!("the mode of {}: {err}", path.display()))
                .permissions()
                .mode()
        };
        let mut dir = socket.parent().expect("the socket's directory");
        loop {
            if mode(dir) & search == 0 {
                return false;
            }
            if dir == state {
                break;
            }
            dir = dir.parent().expect("a directory under the state directory");
        }
        mode(socket) & write != 0
    }
}

#[cfg(target_os = "linux")]
mod container {
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    use crate::common::CLIENT;
    use crate::machine::Finished;

    /// The image of the PostgreSQL the integration tests run on, which `script/test` has pulled:
    /// a Debian whose C library runs the `skald` these tests build.
    const IMAGE: &str = "postgres:19rc1";
    /// The other account: a user and group this machine's user is not.
    const ACCOUNT: &str = "4242:4242";
    /// Where `skald` is in the container.
    const SKALD: &str = "/usr/local/bin/skald";
    /// How long the other account's run may take.
    const LIMIT: Duration = Duration::from_secs(120);

    /// A container, removed on drop.
    struct Container(String);

    impl Drop for Container {
        fn drop(&mut self) {
            let _ = Command::new("docker")
                .args(["rm", "--force", &self.0])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }

    /// Runs `skald <args>` as [`ACCOUNT`] in a container on this machine's network, with the
    /// state directory `state` at the same path, its secrets in the file there, and a request
    /// timeout of 5 seconds.
    pub fn run_as_another_account(state: &Path, args: &[&str]) -> Finished {
        let state = state.to_str().expect("a UTF-8 path");
        let started = Command::new("docker")
            .args(["run", "--detach", "--network", "host", "--user", ACCOUNT])
            .args(["--volume", &format!("{CLIENT}:{SKALD}:ro")])
            .args(["--volume", &format!("{state}:{state}")])
            .args(["--env", &format!("SKALD_STATE_DIR={state}")])
            .args(["--env", "SKALD_KEY_STORAGE=file"])
            .args(["--env", "SKALD_TIMEOUT=5"])
            .args(["--env", "HOME=/tmp"])
            .args(["--entrypoint", SKALD, IMAGE])
            .args(args)
            .output()
            .expect("run docker");
        assert!(
            started.status.success(),
            "the container did not start: {started:?}"
        );
        let container = Container(String::from_utf8_lossy(&started.stdout).trim().to_owned());
        let deadline = Instant::now() + LIMIT;
        let code = loop {
            let inspected = Command::new("docker")
                .args([
                    "inspect",
                    "--format",
                    "{{.State.Status}} {{.State.ExitCode}}",
                ])
                .arg(&container.0)
                .output()
                .expect("inspect the container");
            let state = String::from_utf8_lossy(&inspected.stdout).trim().to_owned();
            if let Some(code) = state.strip_prefix("exited ") {
                break code.parse::<i32>().expect("an exit code");
            }
            assert!(
                Instant::now() < deadline,
                "the container ran past {LIMIT:?}: {state}"
            );
            thread::sleep(Duration::from_millis(200));
        };
        let logs = Command::new("docker")
            .args(["logs", &container.0])
            .output()
            .expect("read the container's output");
        Finished {
            success: code == 0,
            stdout: String::from_utf8_lossy(&logs.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&logs.stderr).into_owned(),
        }
    }
}
