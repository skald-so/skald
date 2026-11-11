//! The environment of the `git` commands the prototype runs: nothing of the parent's reaches them
//! but `PATH`, and the settings `git::command` sets do.

use std::collections::BTreeMap;
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

/// A directory the test adds to its `PATH`, which must reach git's commands; it does not exist.
const PATH_ENTRY: &str = "/nonexistent/skald-test-path";

/// What `git::command` sets: the switches that turn off the global and system config and the
/// system attributes and prompts, and the fixed identity.
const SETTINGS: [(&str, &str); 8] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_ATTR_NOSYSTEM", "1"),
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_AUTHOR_NAME", "skald"),
    ("GIT_AUTHOR_EMAIL", "skald@example.com"),
    ("GIT_COMMITTER_NAME", "skald"),
    ("GIT_COMMITTER_EMAIL", "skald@example.com"),
];

#[test]
fn the_parent_environment_does_not_reach_git() {
    const NAME: &str = "the_parent_environment_does_not_reach_git";
    if std::env::var_os(PARENT[0].0).is_none() {
        // The test reruns itself with the variables set: setting them in this process races with
        // the other threads that read the environment.
        let path = std::env::var_os("PATH").unwrap_or_default();
        let path = std::env::join_paths(
            std::iter::once(PATH_ENTRY.into()).chain(std::env::split_paths(&path)),
        )
        .unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--test-threads=1"])
            .envs(PARENT)
            .env("PATH", path)
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
    // The environment git gives the commands it starts, by name.
    let environment =
        git::run(tmp.path(), &["-c", "alias.environment=!env", "environment"]).unwrap();
    let environment: BTreeMap<&str, &str> = std::str::from_utf8(&environment)
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    for (name, _) in PARENT {
        assert!(!environment.contains_key(name), "{name} reaches git");
    }
    // Git puts its own directories before the `PATH` it was given, and gives a default one when
    // it was given none.
    let path = environment.get("PATH").expect("PATH does not reach git");
    assert!(
        std::env::split_paths(path).any(|entry| entry.as_os_str() == PATH_ENTRY),
        "the test's PATH does not reach git: {path}"
    );
    for (name, value) in SETTINGS {
        assert_eq!(environment.get(name), Some(&value), "{name}");
    }
}
