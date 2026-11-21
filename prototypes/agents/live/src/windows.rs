//! What only Windows needs, which `script/test`, on macOS and Linux, neither builds nor runs:
//! finding a CLI npm installed, the system's variables, plain paths, and the commands of the
//! executor checks in PowerShell.

use std::fs;
use std::path::{Path, PathBuf};

/// The setting that gives Codex's read-only sandbox a backend on Windows. Without it Codex
/// rejects every command a read-only session runs, as "blocked by policy". `elevated` would need
/// an administrator's setup.
pub(crate) const SANDBOX: &str = "windows.sandbox=\"unelevated\"";

/// On Windows npm installs a CLI as `<name>.cmd`, which `Command` neither finds nor hands a
/// multi-line prompt: `<name>.exe` on `PATH`; for Codex, WinGet's binary, on `PATH` or in its
/// package folder; else `node` with the npm package's script next to `<name>.cmd`.
pub(crate) fn cli_program(name: &str) -> (PathBuf, Option<PathBuf>) {
    let dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if let Some(exe) = dirs
        .iter()
        .map(|dir| dir.join(format!("{name}.exe")))
        .find(|exe| exe.is_file())
    {
        return (exe, None);
    }
    if name == "codex"
        && let Some(exe) = winget_codex(&dirs)
    {
        return (exe, None);
    }
    let script = match name {
        "codex" => "node_modules/@openai/codex/bin/codex.js",
        _ => "node_modules/@anthropic-ai/claude-code/cli.js",
    };
    for dir in dirs
        .iter()
        .filter(|dir| dir.join(format!("{name}.cmd")).is_file())
    {
        if dir.join(script).is_file() {
            let node = dir.join("node.exe");
            let node = if node.is_file() {
                node
            } else {
                PathBuf::from("node")
            };
            return (node, Some(dir.join(script)));
        }
    }
    (PathBuf::from(name), None)
}

/// The Codex binary WinGet installs as `codex-<arch>-pc-windows-msvc.exe`: on `PATH`, where WinGet
/// puts its package folder when it cannot link `codex.exe`, or in that folder.
fn winget_codex(dirs: &[PathBuf]) -> Option<PathBuf> {
    let exe = format!("codex-{}-pc-windows-msvc.exe", std::env::consts::ARCH);
    let packages = std::env::var_os("LOCALAPPDATA")
        .map(|local| PathBuf::from(local).join(r"Microsoft\WinGet\Packages"))
        .and_then(|packages| fs::read_dir(packages).ok())
        .into_iter()
        .flatten()
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|dir| {
            dir.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("OpenAI.Codex_"))
        });
    dirs.iter()
        .cloned()
        .chain(packages)
        .map(|dir| dir.join(&exe))
        .find(|path| path.is_file())
}

/// What a program needs of the environment on Windows: the system's variables, from
/// `live-check`'s own, and `home` as `USERPROFILE`.
pub(crate) fn environment(home: &Path) -> Vec<(&'static str, String)> {
    const SYSTEM: [&str; 10] = [
        "SYSTEMROOT",
        "SYSTEMDRIVE",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "TEMP",
        "TMP",
        "PROGRAMDATA",
        "PROGRAMFILES",
        "USERNAME",
    ];
    let mut env: Vec<(&'static str, String)> = SYSTEM
        .into_iter()
        .filter_map(|name| Some((name, std::env::var(name).ok()?)))
        .collect();
    env.push(("USERPROFILE", home.to_string_lossy().into_owned()));
    env
}

/// `path` without the `\\?\` that `fs::canonicalize` puts before it, which no shell takes.
pub(crate) fn plain_path(path: PathBuf) -> PathBuf {
    match path
        .to_str()
        .and_then(|text| text.strip_prefix(r"\\?\"))
        .filter(|text| !text.starts_with("UNC\\"))
    {
        Some(plain) => PathBuf::from(plain),
        None => path,
    }
}

/// What the executor checks' commands are written in: PowerShell, in which Codex runs commands on
/// Windows.
pub(crate) mod shell {
    use std::path::Path;

    pub const READ: &str = "Get-Content -Raw";
    pub const WRITE: &str = "Set-Content -NoNewline -Value written -Path";
    pub const RAN: &str = "Write-Output \"ran-$(20+22)\"";
    pub const CURL: &str = "curl.exe";
    pub const NULL: &str = "NUL";
    pub const THEN_REACHED: &str = "; if ($LASTEXITCODE -eq 0) { Write-Output \"public-$(1+1)\" }";
    pub const MAKE_FILE: [&str; 4] = ["New-Item", "-ItemType", "File", "-Path"];
    pub const MARK: &str = "mark.cmd";

    pub fn quote(path: &Path) -> String {
        format!("'{}'", path.to_string_lossy().replace('\'', "''"))
    }

    /// `program` as the first word of a command that `cmd` or PowerShell runs: a path with a space
    /// in it would need each one's own quoting.
    pub fn program(program: &Path) -> String {
        program.to_string_lossy().into_owned()
    }

    pub fn print_parent(var: &str) -> String {
        format!("Write-Output \"parent: $(if ($env:{var}) {{ $env:{var} }} else {{ 'unset' }})\"")
    }

    pub fn mark_script(marks: &Path) -> String {
        format!("@echo off\r\ntype nul > \"{}\\%1\"\r\n", marks.display())
    }
}
