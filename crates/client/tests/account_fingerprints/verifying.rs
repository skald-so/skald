//! A device marks an account verified only on its person's yes at a terminal, and shows it in
//! later runs, one-shot and through the helper; a no, a pipe and an agent mark nothing.

use crate::helped::Helped;
use crate::terminal::Terminal;
use crate::world::{ACME, MEMBERS};
use crate::{AGENT, CLAUDE, FINGERPRINT, NO, Pair, YES, answered_at_terminal, fingerprints_in};

/// Bob answers yes for Ada at a terminal, one-shot: `skald members` shows her verified in each
/// later run, one-shot and through a helper.
#[test]
fn a_yes_marks_the_account_verified_in_later_one_shot_and_helper_runs() {
    let pair = Pair::new(
        [4_181_103_001, 4_181_103_002],
        ["ada-verified-one-shot", "bob-verifies-one-shot"],
    );
    let machine = &pair.bob.machine;
    pair.assert_ada_on(machine, false);

    pair.bob_verifies_ada();

    pair.assert_ada_on(machine, true);
    pair.assert_ada_on(machine, true);
    let helped = Helped::new(machine);
    let through_helper = helped.run(&[MEMBERS, ACME]);
    assert!(through_helper.success, "{through_helper:?}");
    helped.one_helper();
    pair.assert_ada(&through_helper, true);
}

/// With a helper running on Bob's machine, his yes for Ada at a terminal goes through it:
/// `skald members` shows her verified through the helper, and one-shot once the helper is gone.
#[test]
fn a_yes_beside_a_running_helper_marks_the_account_verified() {
    let pair = Pair::new(
        [4_181_104_001, 4_181_104_002],
        ["ada-verified-by-a-helper", "bob-verifies-by-a-helper"],
    );
    let helped = Helped::new(&pair.bob.machine);
    let before = helped.run(&[MEMBERS, ACME]);
    assert!(before.success, "{before:?}");
    pair.assert_ada(&before, false);
    let helper = helped.one_helper();

    pair.bob_verifies_ada();

    let after = helped.run(&[MEMBERS, ACME]);
    assert!(after.success, "{after:?}");
    pair.assert_ada(&after, true);
    helped.kill(helper);
    helped.assert_none();
    pair.assert_ada_on(&pair.bob.machine, true);
}

/// Bob answers no for Ada at a terminal: she is not verified; a yes after it verifies her.
#[test]
fn a_no_marks_nothing() {
    let pair = Pair::new(
        [4_181_105_001, 4_181_105_002],
        ["ada-not-verified-on-a-no", "bob-answers-no"],
    );

    let declined = answered_at_terminal(&pair.bob.machine, pair.ada(), NO);

    pair.assert_shows_adas(&declined.text);
    pair.assert_unmarked_until_bobs_yes();
}

/// Bob's `skald fingerprint acme <Ada>` with `yes` piped into it shows her fingerprint and marks
/// nothing; a yes at a terminal after it verifies her.
#[test]
fn a_yes_from_a_pipe_marks_nothing() {
    let pair = Pair::new(
        [4_181_106_001, 4_181_106_002],
        ["ada-not-verified-by-a-pipe", "bob-pipes-a-yes"],
    );

    let mut piped = pair
        .bob
        .machine
        .start_asking(&[FINGERPRINT, ACME, pair.ada()]);
    piped.answer(&format!("{YES}\n"));
    let piped = piped.finish();

    pair.assert_shows_adas(&piped.shown());
    pair.assert_unmarked_until_bobs_yes();
}

/// As an agent in a folder bound to `acme`, at a terminal where `yes` is typed, `skald
/// fingerprint acme <Ada>` shows her fingerprint and marks nothing; Bob's own yes after it
/// verifies her.
#[test]
fn an_agent_marks_nothing() {
    let pair = Pair::new(
        [4_181_107_001, 4_181_107_002],
        ["ada-not-verified-by-an-agent", "bob-whose-agent-says-yes"],
    );
    let folder = pair.bobs_acme_folder();

    let args = [AGENT, CLAUDE, FINGERPRINT, ACME, pair.ada()];
    let mut agent = Terminal::start_in(&pair.bob.machine, &folder, &args);
    agent.wait_for("Ada's fingerprint", |shown| {
        fingerprints_in(shown).contains(&pair.adas)
    });
    agent.type_line(YES);
    let agent = agent.finish();

    pair.assert_shows_adas(&agent.text);
    pair.assert_unmarked_until_bobs_yes();
}
