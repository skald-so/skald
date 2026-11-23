//! An invite to two channels on a closed server, from the admin's terminal to a new account,
//! linked devices, peer messages, and the admin's Members page.

#![allow(dead_code)]

#[path = "../../../server/tests/web_sessions/access.rs"]
mod access;
#[path = "../recovery_key/at_terminal.rs"]
mod at_terminal;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
#[path = "../recovery_key/grants.rs"]
mod grants;
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../link_codes/linking.rs"]
mod linking;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../mcp_inbox/client.rs"]
mod mcp;
mod privacy;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../../../server/tests/common/spa.rs"]
mod spa;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
#[path = "../roster_revisions/world.rs"]
mod world;
use grants as terminal_grants;

use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use skald_proto::ChannelId;
use skald_server::{Listed, SignUp};

use browser::{APPROVE, Browser, Page};
use cloud::{Cloud, WAIT};
use machine::{Finished, Machine};
use privacy::Invite;
use spa::{Spa, wait_until};
use terminal::Terminal;
use world::{ACME, HOST, dir_of, path, words};

const ADA: &str = "ada-invite-journey";
const BOB: &str = "bob-invite-journey";
const HOSTS: [&str; 4] = [
    "ada-workstation",
    "ada-laptop",
    "bob-workstation",
    "bob-laptop",
];

#[test]
fn a_new_account_joins_by_link_and_both_accounts_agents_exchange_peer_messages() {
    privacy::with_server_log(journey);
}

fn journey(secret_file: &Path) {
    let browser = Browser::launch();
    let spa = Spa::start(&browser, HOST);
    let closed = SignUp::Closed(Listed {
        github_logins: vec![ADA.to_owned()],
        ..Listed::default()
    });
    let cloud = Cloud::fronted_with(HOST, spa.port, 120, 1, |settings| settings.sign_up = closed);
    spa.serve(&browser, &cloud.relay.url);

    let admin = browser.page_of_width(360);
    let newcomer = browser.page_of_width(360);
    for page in [&admin, &newcomer] {
        page.record_requests();
    }
    let ada = cloud.user(8_001_901_001, ADA);
    let bob = cloud.user(8_001_901_002, BOB);
    let (ada1, signed_in) = cloud.first_machine(&admin, &ada, HOSTS[0]);
    assert!(signed_in.success, "the listed admin signs in");
    let ada2 = link_device(&cloud, &ada1, HOSTS[1]);

    command(&ada1, &["create-space", ACME]);
    let default = channel_ids(&command(&ada1, &["channels", ACME]).stdout);
    let [default] = default.as_slice() else {
        panic!("the new space has one default channel");
    };
    command(&ada1, &["create-channel", ACME]);
    let channels = channel_ids(&command(&ada1, &["channels", ACME]).stdout);
    let [first, other] = channels.as_slice() else {
        panic!("the new space has two channels");
    };
    assert_eq!(first, default);
    assert_ne!(other, default);

    let issued =
        Terminal::start(&ada1, &["invite", ACME, "--channel", &other.to_string()]).finish();
    assert!(issued.success, "the admin creates the invite");
    let invite = Invite::from_terminal(&issued.text, &spa, secret_file);
    invite.check_database(&cloud, "after the admin issued it");
    open_members(&admin, &spa);
    wait_until(&admin, "the pending invite in Members", |page| {
        page.count("main >> role=button[name=/^revoke member invite link/i]") == 1
    });

    let visit = newcomer.goto(invite.link());
    assert_eq!(visit.status, Some(200), "the invite page opens");
    wait_until(
        &newcomer,
        "the terminal instruction on the invite page",
        |page| page.text().contains("skald accept-invite"),
    );

    let bob1 = Machine::new().with_host_name(HOSTS[2]);
    let before = cloud.grants().len();
    let mut accept = Terminal::start(&bob1, &["accept-invite"]);
    accept.wait_for("a prompt for the invite link", |text| {
        text.to_lowercase().contains("link")
    });
    accept.type_line(invite.link());
    let mut grant = grants::shown_grant(&cloud, &mut accept, before);
    // The link the terminal shows carries the authorization into GitHub sign-in.
    grant.verification_uri = format!("{}?user_code={}", grant.verification_uri, grant.user_code);
    accept.wait_to_show(&grant.verification_uri);
    invite.check_database(&cloud, "before approving the newcomer's sign-in");
    cloud.decide(&newcomer, &bob, &grant, APPROVE);
    accept.wait_to_show("Press Enter");
    accept.enter();
    let confirmation = accept.wait_for("the invite confirmation", |text| {
        text.to_lowercase().contains("accept the invite?")
    });
    assert!(
        words(&confirmation).contains(&ACME),
        "the confirmation names the team"
    );
    accept.type_line("yes");
    assert!(
        accept.finish().success,
        "the new account accepts the invite"
    );
    let bob2 = link_device(&cloud, &bob1, HOSTS[3]);
    invite.check_database(&cloud, "with the newcomer waiting");

    let admitted = command(&ada1, &["recv"]).shown();
    let announcement = format!("admitted {BOB} to the space {ACME}");
    assert!(
        admitted.contains(&announcement),
        "the inviting device admits the newcomer"
    );

    wait_for_groups([&ada1, &ada2, &bob1, &bob2], [*default, *other]);

    let ada = Agent::bind(&ada2, "review-k2v9rq");
    let bob = Agent::bind(&bob2, "build-r7f2w9");
    for agent in [&ada, &bob] {
        agent.run(&invite, &["recv"]);
        agent.check_tools(&invite);
    }
    ada.send_to(&invite, BOB, &bob, "Please check the release notes.");
    bob.receives(
        &invite,
        HOSTS[1],
        *default,
        "Please check the release notes.",
    );
    bob.send_to(&invite, ADA, &ada, "The release notes are ready.");
    ada.receives(&invite, HOSTS[3], *default, "The release notes are ready.");

    open_members(&admin, &spa);
    wait_until(
        &admin,
        "the newcomer as a member and the empty invite list",
        |page| {
            let rows = access::rows_holding(page, BOB);
            rows.len() == 1
                && words(&rows[0].to_lowercase()).contains(&"member")
                && page.count("main >> text=No invite links.") == 1
        },
    );
    assert_eq!(
        admin.evaluate("[innerWidth, document.documentElement.scrollWidth]"),
        serde_json::json!([360, 360]),
        "Members fits a narrow screen"
    );

    invite.check_database(&cloud, "after admission");
    invite.check_requests(&cloud, [&admin, &newcomer]);
}

#[track_caller]
fn command(machine: &Machine, args: &[&str]) -> Finished {
    let run = world::run(machine, args);
    assert!(run.success, "skald {} failed", args[0]);
    run
}

fn link_device(cloud: &Cloud, first: &Machine, host: &str) -> Machine {
    let next = Machine::new().with_host_name(host);
    let mut sign_in = Terminal::start(&next, &["sign-in", "--server", &cloud.url]);
    let shown = sign_in.wait_for("the new device's link code", |text| {
        !linking::codes_in(text).is_empty()
    });
    let codes = linking::codes_in(&shown);
    let [code] = codes.as_slice() else {
        panic!("the new device shows one link code");
    };

    let mut link = Terminal::start(first, &["link"]);
    link.wait_to_show(linking::OWN_DEVICE);
    link.type_line(&linking::typed(code));
    link.wait_for("the new device's name and kind", |text| {
        linking::says(text, &[host, "client"])
    });
    link.type_line("yes");
    assert!(link.finish().success, "the first device links {host}");
    assert!(
        sign_in.finish().success,
        "{host} signs in with its link code"
    );
    next
}

fn channel_ids(listed: &str) -> Vec<ChannelId> {
    listed.lines().filter_map(channel_id).collect()
}

fn channel_id(line: &str) -> Option<ChannelId> {
    line.split_whitespace().next()?.parse().ok()
}

fn wait_for_groups(machines: [&Machine; 4], channels: [ChannelId; 2]) {
    let deadline = Instant::now() + WAIT;
    loop {
        for machine in machines {
            command(machine, &["recv"]);
        }
        let missing: Vec<_> = machines
            .into_iter()
            .zip(HOSTS)
            .filter_map(|(machine, name)| (!has_both_groups(machine, channels)).then_some(name))
            .collect();
        if missing.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{missing:?} did not get both channels with all four devices"
        );
        thread::sleep(Duration::from_millis(200));
    }
}

fn has_both_groups(machine: &Machine, channels: [ChannelId; 2]) -> bool {
    let listed = world::run(machine, &["channels", ACME]);
    listed.success
        && channels.into_iter().all(|id| {
            listed
                .stdout
                .lines()
                .find(|line| channel_id(line) == Some(id))
                .and_then(|row| row.split_once("devices: "))
                .is_some_and(|(_, devices)| {
                    HOSTS
                        .into_iter()
                        .all(|host| devices.split(", ").any(|device| device == host))
                })
        })
}

fn open_members(page: &Page, spa: &Spa) {
    let visit = page.goto(&spa.at("/members"));
    assert_eq!(visit.status, Some(200), "Members opens");
    access::press(page, None, "button", "^menu$");
    access::press(page, None, "button", "^space\\b");
    access::press(page, None, "[role=option]", "^acme\\b");
    access::press(page, None, "button", "^menu$");
}

struct Agent<'a> {
    machine: &'a Machine,
    folder: PathBuf,
    name: &'static str,
}

impl<'a> Agent<'a> {
    fn bind(machine: &'a Machine, name: &'static str) -> Self {
        let folder = dir_of(machine, "work/acme");
        command(machine, &["add-rule", path(&folder), ACME]);
        Self {
            machine,
            folder,
            name,
        }
    }

    #[track_caller]
    fn run(&self, invite: &Invite, args: &[&str]) -> Finished {
        let all = [&["--agent", self.name][..], args].concat();
        let run = self.machine.start_in(&self.folder, &all).finish();
        invite.assert_clean(run.stdout.as_bytes(), "agent stdout");
        invite.assert_clean(run.stderr.as_bytes(), "agent stderr");
        assert!(run.success, "skald --agent {} failed", args[0]);
        run
    }

    fn send_to(&self, invite: &Invite, account: &str, recipient: &Agent, text: &str) {
        self.run(
            invite,
            &[
                "send",
                "--to",
                &format!("{account}/{}", recipient.name),
                text,
            ],
        );
    }

    fn receives(&self, invite: &Invite, sender_device: &str, channel: ChannelId, text: &str) {
        let deadline = Instant::now() + WAIT;
        let channel = channel.to_string();
        loop {
            let shown = self.run(invite, &["recv", "--json"]);
            let objects = lines::json_objects(&shown.stdout);
            if let Some(message) = objects
                .iter()
                .find(|item| item["text"].as_str() == Some(text))
            {
                assert_eq!(
                    message["level"].as_str(),
                    Some("peer"),
                    "the message is labelled peer"
                );
                assert_eq!(
                    message["channel"].as_str(),
                    Some(channel.as_str()),
                    "the message used the default channel"
                );
                assert_eq!(
                    message["device_name"].as_str(),
                    Some(sender_device),
                    "the other account's device sent the message"
                );
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{} did not receive the peer message",
                self.name
            );
            thread::sleep(Duration::from_millis(200));
        }
    }

    fn check_tools(&self, invite: &Invite) {
        let mut session = mcp::session_in(
            self.machine,
            &self.folder,
            &["--agent", self.name],
            mcp::CLIENT,
        );
        let listed = session.tools();
        let (stdout, stderr) = session.finish();
        invite.assert_clean(&stdout, "MCP stdout");
        invite.assert_clean(&stderr, "MCP stderr");
        let tools = listed["tools"].as_array().expect("an MCP tool list");
        let names: Vec<_> = tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("a tool name"))
            .collect();
        assert!(names.contains(&mcp::INBOX) && names.contains(&mcp::SEND_TOOL));
        assert!(
            tools
                .iter()
                .all(|tool| !tool.to_string().to_lowercase().contains("invit")),
            "MCP advertises an invite capability"
        );
    }
}
