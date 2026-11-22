//! What a Codex executor's run gets of the device: of its environment, only what Codex needs to run
//! and sign in; of the user's config home, only the sign-in, in a config home of the run's own, and
//! the flags that leave out the user's config, rules, plugins, hooks and skills and save no session.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use skald_adapters::codex::Executor;
use skald_adapters::executor::{Mode, Request};
use skald_proto::{Address, TrustLevel};

use crate::fixtures::{Scratch, plain, token};
use crate::stand_in::{
    ConfigHome, MARKERS, Script, StandIn, device_environment, finish, values_of,
};

fn request() -> Request {
    Request {
        from: "acme/alice/reviewer@skald.example"
            .parse::<Address>()
            .expect("a valid address"),
        level: TrustLevel::Own,
        text: "Say done.".to_owned(),
    }
}

/// The files under `root` with their contents, found breadth first.
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut dirs = vec![root.to_owned()];
    while let Some(dir) = dirs.pop() {
        for path in fs::read_dir(&dir)
            .expect("list a directory")
            .map(|entry| entry.expect("an entry").path())
        {
            match path.is_dir() {
                true => dirs.push(path),
                false => {
                    let contents = fs::read(&path).expect("read a file");
                    found.insert(path, contents);
                }
            }
        }
    }
    found
}

/// The user's config home planted on a device whose home is in `scratch`: `CODEX_HOME` if
/// `codex_home`, else `~/.codex`.
fn plant(scratch: &Scratch, codex_home: bool) -> ConfigHome {
    let at = match codex_home {
        true => scratch.path().join("codex-home"),
        false => scratch.dir("home").join(".codex"),
    };
    ConfigHome::plant(&at)
}

/// Runs a request in `full` with a stand-in that answers, on a device whose home is in `scratch`
/// and whose config home is `config`, named by `CODEX_HOME` if `codex_home`: the stand-in. Fails
/// unless the run finishes.
#[track_caller]
fn run_on(scratch: &Scratch, config: &ConfigHome, codex_home: bool) -> StandIn {
    let home = scratch.dir("home");
    let stand_in = StandIn::new(scratch, Script::Answer(token("answer")));
    let environment = device_environment(&home, codex_home.then_some(config.path.as_path()));
    let executor = Executor::new(stand_in.path.clone()).environment(environment);
    let folder = scratch.dir("folder");

    let run = (executor.start(&folder, Mode::Full, &request()))
        .unwrap_or_else(|refused| panic!("the executor refused full: {refused}"));
    let outcome = finish(run, |run| run.wait());

    assert!(
        outcome.finished,
        "the run did not finish: {:?}",
        outcome.answer
    );
    stand_in
}

/// A run gets none of the device's variables that Codex needs neither to run nor to sign in, and
/// keeps `PATH`, which it needs to run.
pub fn a_run_gets_none_of_the_device_s_other_variables() {
    let scratch = Scratch::new();
    let stand_in = run_on(&scratch, &plant(&scratch, true), true);

    for name in MARKERS {
        assert!(!stand_in.got(name), "the run got {name}");
    }
    assert!(stand_in.got("PATH"), "the run did not get PATH");
}

/// A run gets a `CODEX_HOME` of its own that holds only a copy of the user's `auth.json`, so none of
/// the user's MCP servers, hooks, plugins, rules, skills and instruction files load, while the
/// sign-in works; the user's config home stays as it was.
pub fn a_run_gets_a_config_home_of_its_own_with_only_the_user_s_sign_in() {
    for codex_home in [true, false] {
        let scratch = Scratch::new();
        let config = plant(&scratch, codex_home);
        let before = files(&config.path);
        let stand_in = run_on(&scratch, &config, codex_home);

        let given = (stand_in.codex_home())
            .unwrap_or_else(|| panic!("the run got no CODEX_HOME (CODEX_HOME set: {codex_home})"));
        // The run removes its config home once it ends: only its parent still resolves.
        let parent = plain(given.parent().expect("the run's config home has a parent"));
        let given = parent.join(given.file_name().expect("the run's config home has a name"));
        assert_ne!(
            given, config.path,
            "the run got the user's config home (CODEX_HOME set: {codex_home})"
        );
        assert_eq!(
            stand_in.codex_home_entries(),
            ["auth.json"],
            "the run's config home holds more than the sign-in (CODEX_HOME set: {codex_home})"
        );
        assert_eq!(
            stand_in.codex_home_auth().as_deref(),
            Some(config.auth.as_str()),
            "the run's auth.json is not the user's (CODEX_HOME set: {codex_home})"
        );
        assert_eq!(
            files(&config.path),
            before,
            "the run changed the user's config home"
        );
    }
}

/// A run gets the flags that the live checks found leave out the user's config with its MCP
/// servers, the rules, the plugins, the hooks and the skills' instructions, and save no session.
pub fn a_run_gets_the_flags_that_leave_out_the_user_s_config() {
    let scratch = Scratch::new();
    let stand_in = run_on(&scratch, &plant(&scratch, true), true);
    let flags = stand_in.flags();

    for flag in ["--ignore-user-config", "--ignore-rules", "--ephemeral"] {
        assert!(
            flags.iter().any(|given| given == flag),
            "no {flag}: {flags:?}"
        );
    }
    let disabled = values_of(&flags, "--disable");
    for feature in ["plugins", "hooks"] {
        assert!(
            disabled.iter().any(|given| given == feature),
            "{feature} not disabled: {flags:?}"
        );
    }
    assert!(
        values_of(&flags, "-c").contains(&"skills.include_instructions=false".to_owned()),
        "the skills' instructions are not left out: {flags:?}"
    );
}
