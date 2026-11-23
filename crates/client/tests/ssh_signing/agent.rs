//! A key ssh-agent holds signs, without a terminal and as an agent, where an encrypted key file is
//! not used. Unix only, as the tests' ssh-agent (`ssh_agent.rs`).

use crate::ssh_agent::Agent;
use crate::{
    ADA_GITHUB, ADA_LOGIN, Kind, PASSPHRASE, Setup, SshKey, assert_asks_nothing,
    assert_member_sees_passed, bob_with, keys_dir, new_key,
};

/// `setup` with ssh-agent holding a new key it lists as a signing key of Ada's GitHub user, beside
/// the keys `authentication`; gives the key.
fn holding(agent: &Agent, mut setup: Setup, authentication: &[&SshKey]) -> (Setup, SshKey) {
    setup.machine = agent.serving(setup.machine);
    let held = new_key(&keys_dir(&setup.machine).join("held"), Kind::Ed25519, "");
    setup.list(authentication, &[&held]);
    agent.hold(&held);
    (setup, held)
}

/// A key only the machine's ssh-agent holds, which GitHub lists as a signing key of Ada's GitHub
/// user, and an unlisted key file in her SSH directory: signing in without a terminal, her first
/// device's certificate carries a signature by the agent's key, the run says so, and Bob's device
/// checks it as passed.
#[test]
fn without_a_terminal_a_key_ssh_agent_holds_signs() {
    let agent = Agent::start();
    let (setup, held) = holding(&agent, Setup::new(""), &[]);

    let signed = setup.sign_up(&[]);

    signed.assert_signed((ADA_GITHUB, ADA_LOGIN), &held, &[&signed.key]);
    let bob = bob_with(&signed.world, &signed.api.url(), &signed.ada);
    assert_member_sees_passed(&bob, signed.ada.device, ADA_LOGIN);
}

/// A listed key the machine's ssh-agent holds, and a listed encrypted key file in Ada's SSH
/// directory: signing in as an agent at a terminal, the certificate carries a signature by the
/// agent's key, not by the file's, and nothing asks for a passphrase.
#[test]
fn as_an_agent_a_key_ssh_agent_holds_signs_and_an_encrypted_key_file_is_not_used() {
    let agent = Agent::start();
    let setup = Setup::new(PASSPHRASE);
    let file = setup.key.clone();
    let (setup, held) = holding(&agent, setup, &[&file]);

    let signed = setup.sign_up_at_terminal(true, &[]);

    signed.assert_signed((ADA_GITHUB, ADA_LOGIN), &held, &[&file]);
    assert_asks_nothing(&signed.shown);
}
