//! The page's recipes as snippets: every marked block is one the tests run, each takes only the two
//! secrets under their names, the Docker example's image is pinned, and the page links to what a
//! leaked secret allows.

use crate::page::{
    ACTIONS, Block, DOCKER, DOCKERFILE, LEAKED_CI_SECRET, PAGE, STEPS, block, blocks_of, compact,
    from_secrets, install_command, install_instruction, read, step_of,
};
use crate::service_kit::{KEY_VARIABLE, TOKEN_VARIABLE};

/// The page's marked blocks are those of the steps the tests run, one block each: none of another
/// step, none without a step, no step without its block and none with two.
#[test]
fn every_marked_block_of_the_page_is_one_the_tests_run() {
    let blocks = blocks_of(&read(PAGE));

    let other: Vec<&Block> = blocks
        .iter()
        .filter(|block| !STEPS.contains(&block.step.as_str()))
        .collect();
    assert!(
        other.is_empty(),
        "{PAGE}: marked blocks the tests do not run: {other:?}"
    );
    for step in STEPS {
        let count = blocks.iter().filter(|block| block.step == step).count();
        assert_eq!(count, 1, "{PAGE}: blocks of the step {step}");
    }
}

/// The page links to the threat model's "A leaked CI secret".
#[test]
fn the_page_links_to_what_a_leaked_ci_secret_allows() {
    let page = read(PAGE);

    assert!(
        page.contains(&format!("]({LEAKED_CI_SECRET})")),
        "{PAGE} does not link to {LEAKED_CI_SECRET}"
    );
}

/// The GitHub Actions step sets the two secrets and nothing else, each from the repository's
/// secret of the same name; it uses no other expression; and its script installs `skald` by
/// README's command.
#[test]
fn the_github_actions_step_takes_only_the_two_secrets_under_their_names() {
    let yaml = block(ACTIONS);
    let step = step_of(&yaml);

    let env: Vec<(String, String)> = step
        .env
        .iter()
        .map(|(name, value)| (name.clone(), compact(value)))
        .collect();
    let wanted: Vec<(String, String)> = [KEY_VARIABLE, TOKEN_VARIABLE]
        .iter()
        .map(|name| ((*name).to_owned(), compact(&from_secrets(name))))
        .collect();
    let mut sorted = env.clone();
    sorted.sort();
    assert_eq!(sorted, wanted, "the step's env:\n{yaml}");
    assert_eq!(
        yaml.matches("${{").count(),
        2,
        "the step uses an expression besides the two secrets:\n{yaml}"
    );
    if let Some(shell) = &step.shell {
        assert_eq!(shell, "bash", "the step's shell");
    }
    let install = install_command();
    assert!(
        step.run.lines().any(|line| line.contains(&install)),
        "the step's script does not install skald by README's command {install:?}:\n{}",
        step.run
    );
}

/// The Docker example's image is `debian:13-slim` pinned by its digest, and a `RUN` instruction of
/// it installs `skald` by README's command; its commands build that image and run it on a volume,
/// passing the two secrets.
#[test]
fn the_docker_example_pins_debian_13_slim_and_installs_skald_by_readme_s_command() {
    let dockerfile = block(DOCKERFILE);
    let commands = block(DOCKER);

    let from: Vec<&str> = dockerfile
        .lines()
        .map(str::trim)
        .filter(|line| line.to_ascii_uppercase().starts_with("FROM "))
        .collect();
    let [from] = from.as_slice() else {
        panic!("the Dockerfile has not one FROM:\n{dockerfile}");
    };
    let image = from.split_whitespace().nth(1).unwrap_or_default();
    let digest = image
        .strip_prefix("debian:13-slim@sha256:")
        .unwrap_or_else(|| panic!("the image is not debian:13-slim pinned by digest: {from}"));
    assert!(
        digest.len() == 64
            && digest
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "not a SHA-256 digest: {from}"
    );
    assert!(
        install_instruction(&dockerfile, &install_command()).is_some(),
        "no RUN instruction of the Dockerfile installs skald by README's command:\n{dockerfile}"
    );
    for word in [
        "docker build",
        "docker run",
        KEY_VARIABLE,
        TOKEN_VARIABLE,
        "deploy-bot",
    ] {
        assert!(
            commands.contains(word),
            "the Docker example's commands lack {word:?}:\n{commands}"
        );
    }
    assert!(
        ["--volume ", "-v ", "--mount "]
            .iter()
            .any(|flag| commands.contains(flag)),
        "the Docker example runs its container on no volume:\n{commands}"
    );
}
