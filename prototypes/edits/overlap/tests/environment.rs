//! The environment of the `git` commands the prototype runs: nothing of the parent's reaches them
//! but `PATH`.

use std::collections::BTreeSet;
use std::process::Command;

use edit_overlap::git;
use tempfile::TempDir;

/// Variables of the test's environment that must not reach git, with fake values: a sentinel, the
/// home and config directories that hold the user's global ignore and attributes files, and
/// config injected through the environment.
const PARENT: [(&str, &str); 6] = [
    ("SKALD_TEST_SENTINEL", "fake"),
    ("HOME", "/nonexistent/skald-test-home"),
    ("XDG_CONFIG_HOME", "/nonexistent/skald-test-config"),
    ("GIT_CONFIG_COUNT", "1"),
    ("GIT_CONFIG_KEY_0", "skald.sentinel"),
    ("GIT_CONFIG_VALUE_0", "fake"),
];

#[test]
fn the_parent_environment_does_not_reach_git() {
    const NAME: &str = "the_parent_environment_does_not_reach_git";
    if std::env::var_os(PARENT[0].0).is_none() {
        // The test reruns itself with the variables set: setting them in this process races with
        // the other threads that read the environment.
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--test-threads=1"])
            .envs(PARENT)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "{stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let tmp = TempDir::new().unwrap();
    // The names of the variables of the environment git gives the commands it starts.
    let environment =
        git::run(tmp.path(), &["-c", "alias.environment=!env", "environment"]).unwrap();
    let names: BTreeSet<&str> = std::str::from_utf8(&environment)
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once('=').map(|(name, _)| name))
        .collect();
    for (name, _) in PARENT {
        assert!(!names.contains(name), "{name} reaches git");
    }
    assert!(names.contains("PATH"), "PATH does not reach git");
}
