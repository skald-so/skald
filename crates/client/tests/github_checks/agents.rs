//! A run as an agent with another GitHub API address, which gives another result for the laptop,
//! changes nothing the person's later runs with the usual address show.

use crate::{AGENT_NAME, Api, DEVICES, LOGIN, Listed, Scene, run_with};
use crate::{plain_lines, warnings_naming, words};

/// The login the other address gives Ada's GitHub user.
const IMPOSTOR: &str = "ada-impostor-m5r8";

/// Ada alone, her person's runs with GitHub's API at the first stand-in, which lists the laptop's
/// key where `usual` says, and a run of `skald --agent claude devices` on her machine with it at
/// the second, which lists the key where `other` says and gives [`IMPOSTOR`]. The stand-ins serve
/// until dropped.
fn after_agent(usual: Listed, other: Listed) -> (Scene, [Api; 2]) {
    let apis = [Api::start(), Api::start()];
    let scene = Scene::alone(apis[0].env());
    scene.list_ada(&apis[0].github, LOGIN, usual);
    scene.list_ada(&apis[1].github, IMPOSTOR, other);
    let args = ["--agent", AGENT_NAME, DEVICES];
    let ran = run_with(&scene.ada.machine, &apis[1].env(), &args);
    assert!(ran.success, "{ran:?}");
    (scene, apis)
}

#[test]
fn an_agent_run_whose_github_passes_the_laptop_changes_nothing_later_runs_show() {
    let (scene, _apis) = after_agent(Listed::Nowhere, Listed::Authentication);

    let looks = [scene.own(), scene.own()];
    let warned: Vec<usize> = (looks.iter())
        .map(|shown| warnings_naming(shown, scene.laptop.id).len())
        .collect();
    assert_eq!(warned, [1, 0], "warnings of the laptop per run: {looks:?}");
    for shown in &looks {
        let lines = shown.shown();
        assert!(
            !lines.lines().any(|line| words(line).contains(&IMPOSTOR)),
            "the other address's login is shown: {looks:?}"
        );
    }
}

#[test]
fn an_agent_run_whose_github_fails_the_laptop_changes_nothing_later_runs_show() {
    let (scene, _apis) = after_agent(Listed::Authentication, Listed::Nowhere);

    for _ in 0..2 {
        let looked = scene.look();
        scene.assert_passed(&looked, LOGIN);
        assert!(
            !(plain_lines(&looked.own).iter()).any(|line| words(line).contains(&IMPOSTOR)),
            "the other address's login is shown: {looked:?}"
        );
    }
}
