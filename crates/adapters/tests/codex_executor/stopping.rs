//! Stopping a run of the Codex executor ends Codex and every process it started.

use std::thread;
use std::time::{Duration, Instant};

use skald_adapters::codex::Executor;
use skald_adapters::executor::{Mode, Request};
use skald_proto::{Address, TrustLevel};

use crate::fixtures::Scratch;
use crate::stand_in::{ConfigHome, Script, StandIn, device_environment, finish};

/// How long the stand-in and its child may take to start beating.
const START_LIMIT: Duration = Duration::from_secs(20);

/// How long a stopped process's heartbeat must stay still.
const STILL: Duration = Duration::from_millis(1500);

/// Waits until each of the stand-in's heartbeat files has grown; fails after [`START_LIMIT`].
#[track_caller]
fn wait_beating(stand_in: &StandIn) {
    let start = Instant::now();
    while ["beat", "child-beat"]
        .iter()
        .any(|name| stand_in.beats(name) < 2)
    {
        assert!(
            start.elapsed() < START_LIMIT,
            "the stand-in and its child did not start within {START_LIMIT:?}: beats {}, {}",
            stand_in.beats("beat"),
            stand_in.beats("child-beat")
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// A run stopped while Codex and a child it started run ends both, and gives the run as
/// unfinished.
#[test]
fn stopping_a_run_ends_codex_and_a_child_it_started() {
    let scratch = Scratch::new();
    let stand_in = StandIn::new(&scratch, Script::Lingers);
    let config = ConfigHome::plant(&scratch.path().join("codex-home"));
    let environment = device_environment(&scratch.dir("home"), Some(&config.path));
    let executor = Executor::new(stand_in.path.clone()).environment(environment);
    let request = Request {
        from: "acme/alice/reviewer@skald.example"
            .parse::<Address>()
            .expect("a valid address"),
        level: TrustLevel::Own,
        text: "Work until stopped.".to_owned(),
    };
    let folder = scratch.dir("folder");

    let run = (executor.start(&folder, Mode::Full, &request))
        .unwrap_or_else(|refused| panic!("the executor refused full: {refused}"));
    wait_beating(&stand_in);
    run.stop();
    let outcome = finish(run, |run| run.wait());

    assert!(!outcome.finished, "a stopped run gives a finished run");
    thread::sleep(Duration::from_millis(300));
    let stopped = ["beat", "child-beat"].map(|name| stand_in.beats(name));
    thread::sleep(STILL);
    assert_eq!(
        stand_in.beats("beat"),
        stopped[0],
        "Codex still runs after the run was stopped"
    );
    assert_eq!(
        stand_in.beats("child-beat"),
        stopped[1],
        "a process Codex started still runs after the run was stopped"
    );
}
