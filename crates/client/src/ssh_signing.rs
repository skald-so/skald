//! Signing a device's key with its person's SSH key that GitHub lists (`docs/protocol/membership.md`,
//! "Device certificate"). A `client` device that issues its own certificate, as the account's first
//! device or with the recovery key, signs it first, through OpenSSH's `ssh-keygen`, so passphrases,
//! ssh-agent and security keys stay OpenSSH's. The GitHub user is the login of the account's GitHub
//! sign-in method, or the one `--github` names; the device takes its ID and SSH keys from GitHub's
//! API ([`crate::ssh_checks`]). It signs with the key `--ssh-key` names, or with a listed key that
//! ssh-agent holds or whose private key file is in the person's SSH directory with its public key,
//! `<file>.pub`, beside it. Unless the run is its person's at a terminal, only ssh-agent signs: no
//! key file is used, and nothing asks for a passphrase. Without a signature, the certificate goes
//! without `ssh`, the run says why, and the sign-in goes on.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use skald_crypto::{SshCheck, check_ssh_signature};
use skald_proto::{CertBody, SSH_LABEL, SSH_NAMESPACE, signed_data};

use crate::connection::{Connection, printable};
use crate::ssh_checks::{self, ByLogin};
use crate::{page, person};

/// The armor of an SSH signature `ssh-keygen -Y sign` writes.
const BEGIN: &str = "-----BEGIN SSH SIGNATURE-----";
const END: &str = "-----END SSH SIGNATURE-----";

/// The file of the data `ssh-keygen` signs at the person's terminal, and the file of the signature
/// it writes beside it.
const DATA: &str = "data";
const SIGNATURE: &str = "data.sig";

/// How a device that issues its own certificate signs its key.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The private key file `--ssh-key` names.
    pub key: Option<PathBuf>,
    /// `--no-ssh`: no signature.
    pub off: bool,
    /// The GitHub user `--github` names, in place of the account's GitHub sign-in method's.
    pub github: Option<String>,
    /// Whether the run is its person's at a terminal: only then may key files sign, `ssh-keygen`
    /// asking there for a passphrase.
    pub may_ask: bool,
}

/// `GET /session`'s answer, as far as signing reads it.
#[derive(Deserialize)]
struct Session {
    github_login: Option<String>,
}

impl Options {
    /// Signs `body`, setting its `ssh` and `github`, unless signing is off, and says which key of
    /// which GitHub user signed, or why none did. The session `token` on `connection` gives the
    /// login of the account's GitHub sign-in method.
    pub fn sign(&self, connection: &mut Connection, token: &str, body: &mut CertBody) {
        if self.off {
            return;
        }
        match self.signed(connection, token, body) {
            Ok((fingerprint, login)) => println!(
                "Signed this device's certificate with the SSH key {fingerprint} of the GitHub \
                 user {login}: members of its spaces see {login}."
            ),
            Err(why) => {
                body.ssh = None;
                body.github = None;
                println!("No SSH key signs this device's certificate: {why}.");
            }
        }
    }

    /// [`Options::sign`]; gives the fingerprint of the key that signed and the GitHub user's login,
    /// or why none signed.
    fn signed(
        &self,
        connection: &mut Connection,
        token: &str,
        body: &mut CertBody,
    ) -> Result<(String, String), String> {
        let program = page::program(Path::new("ssh-keygen"))
            .ok_or("ssh-keygen of OpenSSH 8.1 or later is not on the PATH")?;
        let login = match &self.github {
            Some(login) => login.clone(),
            None => connection
                .get_as("/session", token)
                .ok()
                .and_then(|answer| serde_json::from_slice::<Session>(&answer).ok())
                .ok_or("the server did not give the account's GitHub login")?
                .github_login
                .ok_or("the account has no GitHub sign-in method; --github names a GitHub user")?,
        };
        if !ssh_checks::valid_login(&login) {
            return Err(format!("{} is not a GitHub login", printable(&login)));
        }
        let user = match ssh_checks::by_login(&login) {
            ByLogin::Found(user) => user,
            ByLogin::NoUser => return Err(format!("GitHub has no user {login}")),
            ByLogin::Unavailable => {
                return Err(format!(
                    "GitHub did not give the SSH keys of the GitHub user {login}"
                ));
            }
        };
        let login = user.login;
        let listed: Vec<&str> = user.keys.iter().map(String::as_str).collect();
        // At the person's terminal, the key named, or the listed keys ssh-agent holds, then the
        // listed key files; otherwise the listed keys ssh-agent holds, of the key named if any.
        let agent_keys: Vec<&str> = match (&self.key, self.may_ask) {
            (Some(_), true) => Vec::new(),
            (Some(file), false) => {
                let public = fs::read_to_string(public_file(file)).unwrap_or_default();
                (listed.iter().copied())
                    .filter(|line| blob(&public).is_some_and(|key| blob(line) == Some(key)))
                    .collect()
            }
            (None, _) => listed.clone(),
        };
        let held = (!agent_keys.is_empty() && agent_reachable())
            .then(|| Held::write(&agent_keys).ok())
            .flatten();
        // Each file, and whether it is a key file, signed at the person's terminal; ssh-agent's
        // keys sign without it, so nothing it says shows there.
        let held = held.iter().flat_map(Held::files).map(|file| (file, false));
        let candidates: Vec<(PathBuf, bool)> = match (&self.key, self.may_ask) {
            (Some(file), true) => vec![(file.clone(), true)],
            (None, true) => {
                let files = ssh_dir().map_or_else(Vec::new, |dir| key_files(&dir, &listed));
                held.chain(files.into_iter().map(|file| (file, true)))
                    .collect()
            }
            (_, false) => held.collect(),
        };
        let signer = Signer::new(
            program,
            signed_data(
                SSH_LABEL,
                &[
                    body.account.as_bytes(),
                    body.device.as_bytes(),
                    &body.signing,
                ],
            ),
            self.may_ask,
        )
        .map_err(|err| format!("cannot write a temporary file: {err}"))?;
        body.github = Some(user.id);
        let fingerprint = candidates.iter().find_map(|(file, at_terminal)| {
            body.ssh = Some(signer.sign(file, *at_terminal)?);
            match check_ssh_signature(body, &listed) {
                SshCheck::Passed { key } => Some(fingerprint(listed[key])),
                _ => None,
            }
        });
        let fingerprint = fingerprint.ok_or_else(|| {
            let file = (self.key.as_ref()).map(|file| printable(&file.display().to_string()));
            match (file, self.may_ask) {
                (Some(file), true) => {
                    format!("{file} did not sign, or is not a key GitHub lists for {login}")
                }
                (Some(file), false) => format!(
                    "ssh-agent holds no key of {file} that GitHub lists for {login}; without its \
                     person at a terminal, only ssh-agent signs"
                ),
                (None, true) => format!(
                    "no SSH key GitHub lists for {login} is in ssh-agent or in the SSH directory"
                ),
                (None, false) => format!(
                    "no SSH key GitHub lists for {login} is in ssh-agent; without its person at a \
                     terminal, only ssh-agent signs"
                ),
            }
        })?;
        Ok((fingerprint, login))
    }
}

/// `ssh-keygen -Y sign` over `data`, by the program at `program`: with a key file at the person's
/// terminal, over a file in a directory of its own, `ssh-keygen` taking the terminal as its
/// standard input and error to ask for a passphrase or a security key's touch, as git runs it;
/// otherwise, as with ssh-agent's keys, over its standard input, asking and showing nothing.
struct Signer {
    program: PathBuf,
    data: Vec<u8>,
    /// At the person's terminal, the directory of the data's file.
    at_terminal: Option<Scratch>,
}

impl Signer {
    /// A signer at the person's terminal if `at_terminal`.
    fn new(program: PathBuf, data: Vec<u8>, at_terminal: bool) -> io::Result<Self> {
        let at_terminal = at_terminal.then(Scratch::new).transpose()?;
        if let Some(scratch) = &at_terminal {
            fs::write(scratch.join(DATA), &data)?;
        }
        Ok(Signer {
            program,
            data,
            at_terminal,
        })
    }

    /// The signature, in its binary form, by the key of the private key file `key`, or of the
    /// public key file `key` whose private key ssh-agent holds; `None` if `ssh-keygen` gives none.
    /// At the person's terminal only if `at_terminal`.
    fn sign(&self, key: &Path, at_terminal: bool) -> Option<Vec<u8>> {
        let mut command = Command::new(&self.program);
        command
            .args(["-Y", "sign", "-n", SSH_NAMESPACE, "-f"])
            .arg(key);
        let Some(scratch) = self.at_terminal.as_ref().filter(|_| at_terminal) else {
            return self.sign_input(command);
        };
        let signature = scratch.join(SIGNATURE);
        let _ = fs::remove_file(&signature);
        // It reads the terminal, not the thread reading its person's lines.
        let _held_off = person::hold_off();
        let status = (command.arg(scratch.join(DATA)))
            .stdout(Stdio::null())
            .status()
            .ok()?;
        status
            .success()
            .then(|| fs::read(&signature).ok())
            .flatten()
            .and_then(|text| armored(&text))
    }

    /// [`Signer::sign`] over the standard input of `command`, with an empty passphrase, so that it
    /// asks for none, there or through a program.
    fn sign_input(&self, mut command: Command) -> Option<Vec<u8>> {
        command
            .args(["-P", ""])
            .env_remove("SSH_ASKPASS")
            .env_remove("SSH_ASKPASS_REQUIRE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().ok()?;
        let written = (child.stdin.take()).map(|mut stdin| stdin.write_all(&self.data));
        let output = child.wait_with_output().ok()?;
        (matches!(written, Some(Ok(()))) && output.status.success())
            .then(|| armored(&output.stdout))
            .flatten()
    }
}

/// A directory of the command's own in the system's temporary directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> io::Result<Self> {
        let dir = std::env::temp_dir().join(format!("skald-ssh-{}", page::random_name()?));
        fs::create_dir(&dir)?;
        Ok(Scratch(dir))
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Each of a list of public key lines in a file of its own, with no private key beside it, in a
/// directory of its own: `ssh-keygen` signs with such a key only if ssh-agent holds it.
struct Held {
    dir: Scratch,
    count: usize,
}

impl Held {
    fn write(listed: &[&str]) -> io::Result<Self> {
        let held = Held {
            dir: Scratch::new()?,
            count: listed.len(),
        };
        for (at, line) in listed.iter().enumerate() {
            fs::write(held.file(at), format!("{line}\n"))?;
        }
        Ok(held)
    }

    fn file(&self, at: usize) -> PathBuf {
        self.dir.join(&format!("{at}.pub"))
    }

    /// The files, in the list's order.
    fn files(&self) -> Vec<PathBuf> {
        (0..self.count).map(|at| self.file(at)).collect()
    }
}

/// The public key file of the private key file `file`: `<file>.pub`.
fn public_file(file: &Path) -> PathBuf {
    let mut name = OsString::from(file.as_os_str());
    name.push(".pub");
    PathBuf::from(name)
}

/// Whether ssh-agent may be reached: on Windows, its service always may; elsewhere, through
/// `SSH_AUTH_SOCK`.
fn agent_reachable() -> bool {
    cfg!(windows) || std::env::var_os("SSH_AUTH_SOCK").is_some()
}

/// The person's SSH directory, `.ssh` in the home.
fn ssh_dir() -> Option<PathBuf> {
    let home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    Some(PathBuf::from(std::env::var_os(home)?).join(".ssh"))
}

/// The private key files in `dir` whose public key, `<file>.pub` beside each, is one of `listed`,
/// public key lines, by name. Only those public key files are read.
fn key_files(dir: &Path, listed: &[&str]) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = (entries.filter_map(Result::ok))
        .filter_map(|entry| {
            let public = entry.path();
            let private = public.with_extension("");
            if public.extension()? != "pub" || !private.is_file() {
                return None;
            }
            let text = fs::read_to_string(&public).ok()?;
            let key = blob(&text)?;
            listed
                .iter()
                .any(|line| blob(line) == Some(key))
                .then_some(private)
        })
        .collect();
    files.sort();
    files
}

/// The type and the base64 key of an OpenSSH public key line.
fn blob(line: &str) -> Option<(&str, &str)> {
    let mut fields = line.split_whitespace();
    Some((fields.next()?, fields.next()?))
}

/// The SHA256 fingerprint of the public key line `line`, as `ssh-keygen -l` shows it.
fn fingerprint(line: &str) -> String {
    let key = blob(line).and_then(|(_, key)| STANDARD.decode(key).ok());
    format!(
        "SHA256:{}",
        STANDARD_NO_PAD.encode(Sha256::digest(key.unwrap_or_default()))
    )
}

/// The binary form of the armored SSH signature `text` holds.
fn armored(text: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(text).ok()?;
    let mut lines = text.lines().map(str::trim);
    lines.find(|line| *line == BEGIN)?;
    let mut base64 = String::new();
    for line in lines {
        if line == END {
            return STANDARD.decode(base64).ok();
        }
        base64.push_str(line);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;

    const ED25519: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";

    #[test]
    fn the_fingerprint_is_the_unpadded_base64_of_the_key_s_sha_256() {
        let key = STANDARD
            .decode("AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl")
            .unwrap();
        let expected = format!("SHA256:{}", STANDARD_NO_PAD.encode(Sha256::digest(&key)));
        assert_eq!(fingerprint(&format!("{ED25519} comment")), expected);
        assert!(!expected.ends_with('='), "{expected}");
    }

    #[test]
    fn an_armored_signature_gives_its_binary_form_and_anything_else_none() {
        let text = format!("{BEGIN}\nU1NI\nU0lH\n{END}\n");
        assert_eq!(armored(text.as_bytes()), Some(b"SSHSIG".to_vec()));
        assert_eq!(armored(format!("{BEGIN}\nU1NI\n").as_bytes()), None);
        assert_eq!(armored(b"U1NI"), None);
        assert_eq!(armored(format!("{BEGIN}\n!!\n{END}").as_bytes()), None);
    }

    /// Only private key files whose `.pub` beside them is listed count, in order of name.
    #[test]
    fn key_files_are_those_whose_listed_public_key_is_beside_them() {
        let dir = TestDir::new("ssh-signing-key-files");
        let sk = "sk-ssh-ed25519@openssh.com AAAAGnNrLXNzaC1lZDI1NTE5QG9wZW5zc2guY29t";
        let other = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBBBB";
        let files = [
            ("b_listed", Some(ED25519)),
            ("a_listed", Some(ED25519)),
            ("other", Some(other)),
            ("security", Some(sk)),
            ("no_private", None),
        ];
        for (name, public) in files {
            if let Some(public) = public {
                fs::write(dir.join(name), "private").unwrap();
                fs::write(dir.join(format!("{name}.pub")), format!("{public} c\n")).unwrap();
            }
        }
        fs::write(dir.join("no_private.pub"), format!("{ED25519}\n")).unwrap();
        fs::write(dir.join("config"), ED25519).unwrap();
        let listed = [ED25519, sk];

        let expected: Vec<PathBuf> = ["a_listed", "b_listed", "security"]
            .iter()
            .map(|name| dir.join(name))
            .collect();
        assert_eq!(key_files(&dir, &listed), expected);
        assert!(key_files(&dir.join("missing"), &listed).is_empty());
    }

    /// No file but a public key file with a private key file beside it is read: not a FIFO, which
    /// a read waits on until something writes to it.
    #[cfg(unix)]
    #[test]
    fn key_files_read_no_other_file() {
        use std::os::unix::ffi::OsStrExt as _;
        use std::sync::mpsc;
        use std::thread;
        use std::time::Duration;

        let dir = TestDir::new("ssh-signing-other-files");
        for name in ["id_ed25519", "lone.pub"] {
            let path = std::ffi::CString::new(dir.join(name).as_os_str().as_bytes()).unwrap();
            // SAFETY: a NUL-terminated path, for the time of the call.
            assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        }
        let path = dir.to_path_buf();
        let (found, files) = mpsc::channel();
        thread::spawn(move || found.send(key_files(&path, &[ED25519])));

        let files = files.recv_timeout(Duration::from_secs(10));

        assert_eq!(files, Ok(Vec::new()));
    }
}
