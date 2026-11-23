//! The username is typed only by the person at a terminal: through a pipe, or as an agent, the
//! command refuses and sends nothing.

use crate::deletion::{DELETE_ACCOUNT, TERMINAL, assert_kept, assert_sent_nothing};
use crate::fixtures::fronted;
use crate::linking::says;
use crate::talk::ADA_HOST;
use crate::terminal::Terminal;
use crate::world::AGENT;

/// The agent the test runs the command as.
const AGENT_NAME: &str = "claude-d3l3t3";

/// Ada's `skald delete-account` through a pipe, given her username on its input, fails, says it
/// needs a terminal, sends no request that deletes the account and starts no device authorization;
/// her account and her machine's state stay.
#[test]
fn through_a_pipe_it_refuses_and_sends_nothing() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(6_120_003_001, "ada-pipes-her-name", ADA_HOST);
    let since = cloud.relay.mark();

    let mut piped = ada.machine.start_asking(&[DELETE_ACCOUNT]);
    piped.answer(&format!("{}\n", ada.handle));
    let piped = piped.finish();

    assert!(!piped.success, "{piped:?}");
    assert!(says(&piped.shown(), &[TERMINAL]), "{piped:?}");
    assert_sent_nothing(cloud, since, "through a pipe");
    assert_kept(cloud, &ada);
}

/// `skald --agent <name> delete-account` at a terminal on Ada's machine fails, saying it is the
/// person's command, which an agent cannot run; it sends no request that deletes the account and
/// starts no device authorization, and her account and her machine's state stay.
#[test]
fn as_an_agent_it_refuses_and_sends_nothing() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(6_120_003_101, "ada-runs-a-deleting-agent", ADA_HOST);
    let since = cloud.relay.mark();

    let run = Terminal::start(&ada.machine, &[AGENT, AGENT_NAME, DELETE_ACCOUNT]);
    // An agent's run asks nothing; a username typed anyway changes nothing.
    run.type_line(ada.handle.as_str());
    let refused = run.finish();

    assert!(!refused.success, "{}", refused.text);
    let said = refused.text.to_lowercase();
    assert!(
        said.contains(&format!("skald {DELETE_ACCOUNT}"))
            && said.contains("person's command")
            && said.contains("an agent cannot run"),
        "{}",
        refused.text
    );
    assert_sent_nothing(cloud, since, "as an agent");
    assert_kept(cloud, &ada);
}
