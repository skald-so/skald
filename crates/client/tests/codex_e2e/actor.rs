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

    #[cfg(unix)]
    let parent = std::os::unix::process::parent_id();
    #[cfg(windows)]
    let parent = 0; // Skald finds the launcher’s parent through the process table.
    let output = Command::new(env::var_os("SKALD_E2E_HOOK_EXE").expect("Skald executable"))
        .args(["hook", "codex"])
        .env("SKALD_LAUNCHER", format!("{} {parent}", process::id()))
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit())
        .output()
        .expect("run Skald hook");
    let records = env::var_os("SKALD_E2E_HOOK_CAPTURE").expect("hook record directory");
    let path = std::path::PathBuf::from(records).join(format!("{name}-{}", process::id()));
    let pending = path.with_extension("pending");
    fs::write(&pending, &output.stdout).expect("record hook stdout");
    fs::rename(pending, path).expect("publish hook stdout");
    io::stdout()
        .write_all(&output.stdout)
        .expect("forward hook stdout");
    process::exit(output.status.code().unwrap_or(1));
}
