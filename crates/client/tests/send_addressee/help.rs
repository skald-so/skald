//! The help of `skald send`: its addressee flags, and what an addressee does not do.

use crate::fixtures::{ALL, CHANNEL, SEND, TO};
use crate::machine::Machine;

/// `skald send --help` lists `--to`, `--all` and `--channel`, and its text of `--to`, from that
/// flag's line to the next flag's, says that an addressee does not make a message private.
#[test]
fn send_help_says_an_addressee_does_not_make_a_message_private() {
    let machine = Machine::new();

    let help = machine.start(&[SEND, "--help"]).finish();

    assert!(help.success, "{help:?}");
    let lines: Vec<&str> = help.stdout.lines().collect();
    for flag in [TO, ALL, CHANNEL] {
        assert!(
            lines.iter().any(|line| line.trim_start().starts_with(flag)),
            "no {flag}: {}",
            help.stdout
        );
    }
    let start = (lines.iter())
        .position(|line| line.trim_start().starts_with(TO))
        .expect("a line of --to");
    let of_to: String = (lines[start..].iter().enumerate())
        .take_while(|(at, line)| *at == 0 || !line.trim_start().starts_with('-'))
        .map(|(_, line)| format!("{line}\n"))
        .collect();
    assert!(
        of_to.to_lowercase().contains("private"),
        "the help of --to does not say an addressee does not make a message private: {of_to}"
    );
}
