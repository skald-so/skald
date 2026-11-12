//! The tools of the [scripted sessions](crate::sessions), pinned: `rustc`, `cargo` and `rustfmt` of
//! the Rust toolchain, tools of the system such as Node.js and Python, and tools that `cargo`,
//! `npm` and `pip` install at pinned versions; the system's shell commands are recorded. Each runs
//! with an environment of its own: a `PATH` of those tools and the system's directories, and a
//! home, configs and caches in the command's directory, never the machine's.

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

/// The line of `printed` that pins `expected`: the line, without the spaces around it, that is
/// `expected` or starts with it and a space, such as `pip 24.3.1 from <path>` for `pip 24.3.1`.
pub fn version_line<'p>(printed: &'p str, expected: &str) -> Option<&'p str> {
    printed.lines().map(str::trim).find(|line| {
        line.strip_prefix(expected)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
    })
}

/// The version of `program`, a path, run with `--version` in `dir` and `env`: `expected`, when a
/// line of its output pins it as [`version_line`] finds it. Any other version is refused.
pub fn pinned(program: &Path, dir: &Path, env: &Environment, expected: &str) -> Result<String> {
    let mut command = env.command(program, dir);
    command.arg("--version");
    let name = program.file_name().unwrap_or_default().to_string_lossy();
    let printed = output(command, &name)?;
    match version_line(&printed, expected) {
        Some(_) => Ok(expected.to_owned()),
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

/// A tool, a binary on the sessions' `PATH`, and the version it is pinned at: a line its
/// `--version` prints, as [`version_line`] finds it.
#[derive(Debug, Clone, Copy)]
pub struct Pin {
    pub tool: &'static str,
    pub version: &'static str,
}

/// What installs a tool into the command's directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installer {
    /// `cargo install --root <dir>/cargo-tools`.
    Cargo,
    /// `npm install --prefix <dir>/npm`, with the pinned `npm`.
    Npm,
    /// `pip install` in a virtual environment `<dir>/venv`, made by the pinned `python3`.
    Pip,
}

/// Tools that `installer` installs with `args`, such as `pnpm@10.6.3`, each pinned by `pins`.
#[derive(Debug, Clone, Copy)]
pub struct Install {
    pub installer: Installer,
    pub args: &'static [&'static str],
    pub pins: &'static [Pin],
}

/// The tools of the sessions: the Rust toolchain's, the system's that this process's `PATH`
/// gives, and the installed ones, each pinned.
#[derive(Debug, Clone, Copy)]
pub struct Toolset {
    pub toolchain: &'static [Pin],
    pub system: &'static [Pin],
    pub installs: &'static [Install],
}

/// The tools of the sessions: the environment of their shell commands, and each tool's version.
pub struct Tools {
    /// `PATH`: the toolchain, the installed tools, the system's tools, the [`SYSTEM`]
    /// directories; the home, configs and caches of [`isolation`].
    pub env: Environment,
    /// Each tool with its version.
    pub versions: Vec<(String, String)>,
}

/// The variables that keep the tools' homes, configs and caches in `tmp`, the command's
/// directory, by name: a home and its XDG directories, the caches of cargo, npm, pnpm, yarn,
/// dprint, pip and ruff, npm's user and global configs, which [`prepare`] leaves empty, no pip
/// config, and the modules of the npm installs for scripts that `require` them.
pub fn isolation(tmp: &Path) -> Vec<(&'static str, OsString)> {
    let home = tmp.join("home");
    let mut vars: Vec<(&str, OsString)> = vec![
        ("HOME", home.clone().into()),
        ("XDG_CACHE_HOME", home.join(".cache").into()),
        ("XDG_CONFIG_HOME", home.join(".config").into()),
        ("XDG_DATA_HOME", home.join(".local/share").into()),
        ("XDG_STATE_HOME", home.join(".local/state").into()),
    ];
    for (name, dir) in [
        ("CARGO_HOME", "cargo"),
        ("npm_config_cache", "npm-cache"),
        ("npm_config_userconfig", "npmrc"),
        ("npm_config_globalconfig", "npmrc-global"),
        ("npm_config_store_dir", "pnpm-store"),
        ("YARN_CACHE_FOLDER", "yarn-cache"),
        ("DPRINT_CACHE_DIR", "dprint-cache"),
        ("PIP_CACHE_DIR", "pip-cache"),
        ("RUFF_CACHE_DIR", "ruff-cache"),
        ("NODE_PATH", "npm/node_modules"),
    ] {
        vars.push((name, tmp.join(dir).into()));
    }
    vars.extend([
        ("PIP_CONFIG_FILE", "/dev/null".into()),
        ("PIP_DISABLE_PIP_VERSION_CHECK", "1".into()),
        ("npm_config_update_notifier", "false".into()),
    ]);
    vars
}

/// Links `target` as `link`; only where links are symbolic links of Unix.
fn link(target: &Path, link: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        Err(format!("no link to {} as {}", target.display(), link.display()).into())
    }
}

/// Sets up `toolset` in `tmp`, an empty directory: checks the toolchain's versions, links the
/// system's tools that this process's `PATH` gives into `tmp/bin` and checks theirs, runs each
/// install with the tools' environment and checks the versions of what it installed, and records
/// the system's release for its shell commands. Fails on a version that is not pinned.
pub fn prepare(tmp: &Path, toolset: &Toolset) -> Result<Tools> {
    let toolchain = toolchain(tmp)?;
    let links = tmp.join("bin");
    let cargo_tools = tmp.join("cargo-tools");
    let npm = tmp.join("npm");
    let venv = tmp.join("venv");
    let bins = [
        toolchain.clone(),
        venv.join("bin"),
        cargo_tools.join("bin"),
        npm.join("node_modules/.bin"),
        links.clone(),
    ];
    let mut dirs: Vec<&Path> = bins.iter().map(PathBuf::as_path).collect();
    dirs.extend(SYSTEM.map(Path::new));
    let mut env = Environment::new(&dirs)?;
    for (name, value) in isolation(tmp) {
        env = env.with(name, value);
    }
    std::fs::create_dir_all(tmp.join("home"))?;
    std::fs::write(tmp.join("npmrc"), "")?;
    std::fs::write(tmp.join("npmrc-global"), "")?;
    std::fs::create_dir(&links)?;
    let mut versions = Vec::new();
    let mut check = |dir: &Path, pins: &[Pin]| -> Result<()> {
        for pin in pins {
            let version = pinned(&dir.join(pin.tool), tmp, &env, pin.version)?;
            versions.push((pin.tool.to_owned(), version));
        }
        Ok(())
    };
    check(&toolchain, toolset.toolchain)?;
    for pin in toolset.system {
        let found = git::on_path(pin.tool).ok_or_else(|| format!("{} is not on PATH", pin.tool))?;
        link(&found, &links.join(pin.tool))?;
    }
    check(&links, toolset.system)?;
    for install in toolset.installs {
        let (mut command, bin) = match install.installer {
            Installer::Cargo => {
                let mut command = env.command(toolchain.join("cargo"), tmp);
                command.env("CARGO_TARGET_DIR", tmp.join("target"));
                command
                    .args(["install", "--quiet", "--root"])
                    .arg(&cargo_tools);
                (command, cargo_tools.join("bin"))
            }
            Installer::Npm => {
                let mut command = env.command(links.join("npm"), tmp);
                command.args(["install", "--no-audit", "--no-fund", "--loglevel=error"]);
                command.arg("--prefix").arg(&npm);
                (command, npm.join("node_modules/.bin"))
            }
            Installer::Pip => {
                if !venv.exists() {
                    let mut make = env.command(links.join("python3"), tmp);
                    make.args(["-m", "venv"]).arg(&venv);
                    output(make, "python3 -m venv")?;
                }
                let mut command = env.command(venv.join("bin/pip"), tmp);
                command.args(["install", "--quiet"]);
                (command, venv.join("bin"))
            }
        };
        command.args(install.args);
        output(command, &format!("{:?} install", install.installer))?;
        check(&bin, install.pins)?;
    }
    let mut uname = Environment::system()?.command("/usr/bin/uname", tmp);
    uname.arg("-sr");
    let system_version = output(uname, "uname")?;
    versions.push((
        "the shell and its commands".into(),
        format!("the system's: {}", system_version.trim()),
    ));
    Ok(Tools { env, versions })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_line_is_the_pin_or_starts_with_it_and_a_space() {
        let printed = "tool\n  pip 24.3.1 from /venv (python 3.13)\nrustc 1.0.0\n";
        assert_eq!(version_line(printed, "rustc 1.0.0"), Some("rustc 1.0.0"));
        assert_eq!(
            version_line(printed, "pip 24.3.1"),
            Some("pip 24.3.1 from /venv (python 3.13)")
        );
        assert_eq!(version_line(printed, "pip 24.3"), None);
        assert_eq!(version_line(printed, "rustc 1.0"), None);
        assert_eq!(version_line(printed, "1.0.0"), None);
    }
}
