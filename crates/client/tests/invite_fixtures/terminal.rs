//! Create and accept invite links at the CLI without printing the transcript.

use skald_crypto::InviteLink;
use zeroize::Zeroizing;

use crate::machine::Machine;
use crate::terminal::Terminal;

pub const ACCEPT: &str = "accept-invite";

/// Run the invite command and return the link it displayed, as typed and parsed.
/// If `web` is given, require the link to start at that address.
#[track_caller]
pub fn create_link(
    machine: &Machine,
    args: &[&str],
    web: Option<&str>,
) -> (Zeroizing<String>, InviteLink) {
    let shown = Terminal::start(machine, args)
        .with_private_output()
        .finish();
    assert!(shown.success, "skald invite failed");
    let mut words = shown.text.split_whitespace();
    if let Some(web) = web {
        let prefix = format!("{web}/invite/");
        let link = words
            .find(|word| word.starts_with(&prefix))
            .expect("the terminal shows the link at the web address");
        let parsed = InviteLink::parse(link).expect("a valid invite link");
        (Zeroizing::new(link.to_owned()), parsed)
    } else {
        words
            .find_map(|word| {
                let parsed = InviteLink::parse(word).ok()?;
                Some((Zeroizing::new(word.to_owned()), parsed))
            })
            .expect("skald invite shows a link")
    }
}

/// Start `skald accept-invite <extra>` at a terminal.
pub fn start_accepting(machine: &Machine, extra: &[&str]) -> Terminal {
    Terminal::start(machine, &[&[ACCEPT][..], extra].concat()).with_private_output()
}

/// Wait for the prompt for the invite link and return what the terminal showed so far.
#[track_caller]
pub fn asked_for_link(run: &mut Terminal) -> String {
    run.wait_for("a prompt for the invite link", |shown| {
        shown.to_lowercase().contains("link")
    })
}

/// Verify the space in the accept prompt, then confirm the invite.
#[track_caller]
pub fn confirm(run: &mut Terminal, names_space: impl FnOnce(&str) -> bool) {
    let summary = run.wait_for("a question to accept the invite", |shown| {
        shown.to_lowercase().contains("accept the invite?")
    });
    assert!(names_space(&summary), "the confirmation names the team");
    run.type_line("yes");
}
