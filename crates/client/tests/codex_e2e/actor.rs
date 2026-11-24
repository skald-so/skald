//! Native fixture invoked by Codex on Unix and Windows.

use std::env;
use std::fs;
use std::io::{self, Write};
use std::process::{self, Command, Stdio};

fn main() {
    let mut args = env::args().skip(1);
    let action = args.next().expect("fixture action");
    let name = args.next().expect("fixture observation");
    if action == "mark" {
        let exe = env::current_exe().expect("fixture executable");
        let marks = exe.parent().expect("fixture directory").join("marks");
        fs::write(marks.join(name), []).expect("write fixture marker");
        return;
    }
    assert_eq!(action, "capture");
    let records = env::var_os("SKALD_E2E_HOOK_CAPTURE").expect("hook record directory");
    let path = std::path::PathBuf::from(records).join(format!("{name}-{}", process::id()));
    let status = path.with_extension("status");
    fs::write(&status, "wrapper started").expect("record hook start");

    #[cfg(unix)]
    let parent = std::os::unix::process::parent_id();
    #[cfg(windows)]
    let parent = 0; // Skald finds the launcher’s parent through the process table.
    let output = Command::new(env::var_os("SKALD_E2E_HOOK_EXE").expect("Skald executable"))
        .args(["hook", "codex"])
        .env("SKALD_LAUNCHER", format!("{} {parent}", process::id()))
        .stdin(Stdio::inherit())
        .output()
        .expect("run Skald hook");
    let code = output.status.code().unwrap_or(1);
    let stderr = &output.stderr[..output.stderr.len().min(1_024)];
    fs::write(
        &status,
        format!("{code}\n{}", String::from_utf8_lossy(stderr)),
    )
    .expect("record hook exit code");
    let pending = path.with_extension("pending");
    fs::write(&pending, &output.stdout).expect("record hook stdout");
    fs::rename(pending, path).expect("publish hook stdout");
    io::stdout()
        .write_all(&output.stdout)
        .expect("forward hook stdout");
    io::stderr()
        .write_all(&output.stderr)
        .expect("forward hook stderr");
    process::exit(code);
}
