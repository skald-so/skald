//! The tools of the [scripted sessions](crate::sessions), pinned: `rustc`, `cargo` and `rustfmt` of
//! the Rust toolchain, a code generator that `cargo install` builds at a pinned version, and the
//! system's shell commands. Each runs with an environment of its own: a `PATH` of the toolchain,
//! the generator and the system's directories, and a `CARGO_HOME` in the session's directory, never
//! the machine's.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{Result, git};

/// The system's directories of shell commands, in `PATH` order.
pub const SYSTEM: [&str; 2] = ["/usr/bin", "/bin"];

/// The variables a tool's process gets; nothing of the parent's environment.
#[derive(Debug, Clone)]
pub struct Environment {
    vars: Vec<(String, OsString)>,
}

impl Environment {
    /// An environment whose `PATH` holds `dirs`, in order.
    pub fn new(dirs: &[&Path]) -> Result<Self> {
        let path = std::env::join_paths(dirs)?;
        Ok(Self {
            vars: vec![("PATH".into(), path)],
        })
    }

    /// An environment whose `PATH` holds only the [`SYSTEM`] directories.
    pub fn system() -> Result<Self> {
        Self::new(&SYSTEM.map(Path::new))
    }

    /// The environment with `name` set to `value`.
    pub fn with(mut self, name: &str, value: impl Into<OsString>) -> Self {
        self.vars.push((name.into(), value.into()));
        self
    }

    /// `program`, a path, in `dir` with only this environment.
    pub fn command(&self, program: impl AsRef<OsStr>, dir: &Path) -> Command {
        let mut command = Command::new(program);
        command.current_dir(dir).env_clear();
        command.envs(self.vars.iter().map(|(name, value)| (name, value)));
        command
    }
}

/// Runs `command` and returns its standard output if it succeeds; otherwise an error that names
/// `what` with its standard error.
pub fn output(mut command: Command, what: &str) -> Result<String> {
    let output = command.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{what}: {}", stderr.trim()).into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

/// The version of `program`, a path, run with `--version` in `dir` and `env`: the line of its
/// output that is `expected`, without the spaces around it. Any other version is refused.
pub fn pinned(program: &Path, dir: &Path, env: &Environment, expected: &str) -> Result<String> {
    let mut command = env.command(program, dir);
    command.arg("--version");
    let name = program.file_name().unwrap_or_default().to_string_lossy();
    let printed = output(command, &name)?;
    match printed
        .lines()
        .map(str::trim)
        .find(|line| *line == expected)
    {
        Some(line) => Ok(line.to_owned()),
        None => Err(format!(
            "{name} prints {:?}, not the pinned {expected:?}",
            printed.trim()
        )
        .into()),
    }
}

/// The directory of the binaries of the Rust toolchain whose `rustc` this process's `PATH` gives,
/// asked in `dir`, which holds no toolchain file: `rustup` installs no toolchain for it.
pub fn toolchain(dir: &Path) -> Result<PathBuf> {
    let rustc = git::on_path("rustc").ok_or("rustc is not on PATH")?;
    let mut env = Environment {
        vars: vec![("RUSTUP_AUTO_INSTALL".into(), "0".into())],
    };
    for name in ["PATH", "RUSTUP_HOME"] {
        if let Some(value) = std::env::var_os(name) {
            env = env.with(name, value);
        }
    }
    let mut command = env.command(rustc, dir);
    command.args(["--print", "sysroot"]);
    let sysroot = output(command, "rustc --print sysroot")?;
    Ok(Path::new(sysroot.trim_end()).join("bin"))
}

/// The pinned versions of the Rust toolchain's tools, each the first line `--version` prints.
pub struct Pins {
    pub rustc: &'static str,
    pub cargo: &'static str,
    pub rustfmt: &'static str,
}

/// A code generator: the arguments `cargo install` builds it with, the name of its binary, and the
/// line of its `--version` output that its pinned version prints.
pub struct Generator {
    pub install: &'static [&'static str],
    pub binary: &'static str,
    pub version: &'static str,
}

/// The tools of the sessions: the environment of their shell commands, and each tool's version.
pub struct Tools {
    /// `PATH`: the toolchain, the generator, the [`SYSTEM`] directories; `CARGO_HOME`: the
    /// cargo cache of the sessions.
    pub env: Environment,
    /// Each tool with its version, as the tools print them.
    pub versions: Vec<(String, String)>,
}

/// Sets up the tools in `tmp`, an empty directory: checks the toolchain's versions against `pins`,
/// builds `generator` with the toolchain's `cargo` into `tmp`, with a cargo cache of its own there,
/// and checks its version. Fails on a version that is not pinned.
pub fn prepare(tmp: &Path, pins: &Pins, generator: &Generator) -> Result<Tools> {
    let bin = toolchain(tmp)?;
    let system = SYSTEM.map(Path::new);
    let env = Environment::new(&[&[bin.as_path()], &system[..]].concat())?;
    let mut versions = Vec::new();
    for (name, expected) in [
        ("rustc", pins.rustc),
        ("cargo", pins.cargo),
        ("rustfmt", pins.rustfmt),
    ] {
        let version = pinned(&bin.join(name), tmp, &env, expected)?;
        versions.push((name.to_owned(), version));
    }
    let cargo_home = tmp.join("cargo");
    let root = tmp.join("generator");
    let cargo = env
        .clone()
        .with("CARGO_HOME", &cargo_home)
        .with("CARGO_TARGET_DIR", tmp.join("target"));
    let mut install = cargo.command(bin.join("cargo"), tmp);
    install
        .args(["install", "--quiet", "--root"])
        .arg(&root)
        .args(generator.install);
    output(install, "cargo install")?;
    let binary = root.join("bin").join(generator.binary);
    let version = pinned(&binary, tmp, &env, generator.version)?;
    versions.push((generator.binary.to_owned(), version));
    let mut uname = Environment::system()?.command("/usr/bin/uname", tmp);
    uname.arg("-sr");
    let system_version = output(uname, "uname")?;
    versions.push((
        "the shell and its commands".into(),
        format!("the system's: {}", system_version.trim()),
    ));
    let generator_bin = root.join("bin");
    let dirs = [&[bin.as_path(), generator_bin.as_path()], &system[..]].concat();
    let env = Environment::new(&dirs)?.with("CARGO_HOME", cargo_home);
    Ok(Tools { env, versions })
}
