//! Each text of signing in, linking and recovery is one short line: at a terminal, on a local page
//! and in an MCP tool's result, each line holds at most [`SHORT`] characters, each link and each ID
//! counted as one.

use crate::agents::{Agents, CLAUDE};
use crate::at_terminal::AGENT as AGENT_NAME;
use crate::browser_first::{recovered, shown_recovery_keys};
use crate::code_first::linked_by_code;
use crate::devices::second_machine;
use crate::fixtures::fronted_with;
use crate::linking::{OWN_DEVICE, YES, link_with, typed};
use crate::machine::{DEVICES, Machine};
use crate::mcp_client::{CLIENT, MCP, Session};
use crate::pages::{Opener, confirm_on_page, link_on_page};
use crate::prompt::UNVERIFIED;
use crate::world::{AGENT, World, run};
use crate::{Fresh, assert_short_lines, until};

/// The host names of Ada's machines.
const WORKSTATION: &str = "ada-workstation";
const DESKTOP: &str = "ada-desktop";
const LAPTOP: &str = "ada-laptop";

/// Ada's machine signs up at a terminal on her new account, showing the browser sign-in and a link
/// code; she approves it in the browser and presses Enter once it shows the recovery key. Each line
/// it showed, and each of her `skald devices` then, is short.
#[test]
fn signing_up_says_each_text_in_one_short_line() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = cloud.user(4_193_501_001, "ada-signs-up-in-short-lines");
    let mut fresh = Fresh::start(cloud, WORKSTATION, &[]);

    fresh.approve_as(&world, &ada);
    shown_recovery_keys(cloud, &mut fresh);
    fresh.run().enter();
    let shown = fresh.finish();
    assert!(shown.success, "{}", shown.text);
    let devices = run(&fresh.machine, &[DEVICES]);

    assert_short_lines("skald sign-in on a new account", &shown.text);
    assert_short_lines("skald devices on the first device", &devices.shown());
}

/// Ada's account has her first machine and a desktop. Her new machine is linked by its code: each
/// line of its sign-in and of her `skald link` is short, as is each line of `skald devices` and of
/// `skald recv` on the desktop, which learns of the new device.
#[test]
fn linking_by_code_says_each_text_in_one_short_line() {
    let (world, _front) = fronted_with(|_| {});
    let cloud = &world.cloud;
    let ada = world.sign_up(4_193_502_001, "ada-links-in-short-lines", WORKSTATION);
    let desktop = second_machine(&world, &ada, DESKTOP);

    let linked = linked_by_code(cloud, &ada, LAPTOP, |_| {});
    let devices = run(&desktop.machine, &[DEVICES]);
    let received = run(&desktop.machine, &["recv"]);

    assert_short_lines("skald sign-in linked by its code", &linked.signed_in.text);
    assert_short_lines("skald link", &linked.link.text);
    assert_short_lines("skald devices on another device", &devices.shown());
    assert_short_lines("skald recv on another device", &received.shown());
}

/// Ada's new machine runs `skald sign-in --with-recovery-key`, naming her first device lost; she
/// approves it and types the key: each line it showed is short.
#[test]
fn recovery_says_each_text_in_one_short_line() {
    let world = World::start();
    let shown = recovered(&world, (4_193_503_001, "ada-recovers-in-short-lines"));

    assert!(shown.success, "{}", shown.text);
    assert_short_lines("skald sign-in --with-recovery-key", &shown.text);
}

/// In a talk of Ada and Bob, Ada's new machine signs in at a terminal and she approves it in the
/// browser, so it waits for its code. On her first machine, `skald devices`, the same as an agent,
/// and the MCP server's `inbox` say it waits: each of their lines is short. Linked by its code,
/// each line the new machine showed is short.
#[test]
fn the_waiting_device_is_said_in_one_short_line() {
    let agents = Agents::new(
        [4_193_504_001, 4_193_504_002],
        ["ada-waits-in-short-lines", "bob-reads-short-lines"],
    );
    let talk = &agents.talk;
    let (world, ada) = (&talk.world, &talk.ada);
    let cloud = &world.cloud;
    let mut fresh = Fresh::start(cloud, LAPTOP, &[]);
    fresh.approve_as(world, &ada.user);
    until("the new device signed in", || fresh.signed_in(cloud));

    let mut person = String::new();
    until("the prompt on the first machine", || {
        person = run(&ada.machine, &[DEVICES]).shown();
        person.to_lowercase().contains(UNVERIFIED)
    });
    let agent = run(&ada.machine, &[AGENT, AGENT_NAME, DEVICES]).shown();
    let command = &mut (ada.machine).command_in(&agents.folder, &[AGENT, CLAUDE, MCP]);
    let inbox = Session::start(command, CLIENT).inbox();
    let code = fresh.code(cloud);
    let linked = link_with(ada, &typed(&code), LAPTOP, YES);
    assert!(linked.success, "skald link failed: {}", linked.text);
    let shown = fresh.finish();
    assert!(shown.success, "{}", shown.text);

    assert_short_lines("skald devices", &person);
    assert_short_lines("skald devices as an agent", &agent);
    assert!(inbox.to_lowercase().contains(UNVERIFIED), "{inbox}");
    assert_short_lines("the MCP inbox", &inbox);
    assert_short_lines("skald sign-in", &shown.text);
}

/// Ada's new machine runs `skald sign-in --browser` as an agent, and her first machine `skald link
/// --browser`, on which she enters the code and confirms: each line of both pages and both outputs
/// is short.
#[test]
fn the_pages_say_each_text_in_one_short_line() {
    let (world, _front) = fronted_with(|_| {});
    let ada = world.sign_up(4_193_505_001, "ada-pages-in-short-lines", WORKSTATION);
    let machine = Machine::new().with_host_name(LAPTOP);
    let (run, _opener, paged, code) =
        crate::on_page::on_its_page(&world, &machine, &crate::at_terminal::as_agent(&[]));
    let sign_in_page = paged.page.text();

    let at_first = Opener::on(&ada.machine);
    let (linking, page) = link_on_page(&world, &ada, &at_first, OWN_DEVICE);
    let link_page = page.page.text();
    let linked = confirm_on_page(linking, &page, &typed(&code), LAPTOP);
    let link_page_after = page.page.text();
    let finished = run.finish();
    assert!(finished.success, "{finished:?}");

    assert_short_lines("the page of skald sign-in", &sign_in_page);
    assert_short_lines("the page of skald link", &link_page);
    assert_short_lines("the page of skald link, linked", &link_page_after);
    assert_short_lines("skald link --browser", &linked.shown());
    assert_short_lines("skald sign-in --browser", &finished.shown());
}
