//! Invites the CLI refuses: an expiry over 30 days and a channel the space lacks, with nothing
//! stored; output to a pipe and a run as an agent, sending nothing; a device of a member who is no
//! admin, sending no request about invites.

use skald_proto::Role;

use crate::machine::Machine;
use crate::terminal::Terminal;
use crate::world::{
    ACME, ADD_RULE, ADMIN, AGENT, GLOBEX, MEMBER, World, assert_nothing_sent, assert_role, create,
    dir_of, members, next_by, path, run, set_role, store,
};
use crate::{
    Admin, CHANNEL, DAYS, invite_args, invite_at_terminal, invite_requests, issued_since,
    shows_a_link,
};

/// The agent the tests run the command as.
const AGENT_NAME: &str = "builder";

/// Fails unless `admin`'s machine sent no request about invites from `since` on, so the server
/// issued no invite ID and stored no invite.
#[track_caller]
fn assert_nothing_stored(admin: &Admin, since: std::time::Instant) {
    let cloud = admin.cloud();
    assert_eq!(invite_requests(cloud, since), Vec::<String>::new());
    assert_eq!(issued_since(cloud, since), Vec::new(), "invite IDs issued");
}

/// `skald invite acme --days 31` at a terminal fails, saying `30`, shows no link and sends no
/// request about invites.
#[test]
fn expiry_over_30_days_is_refused_with_nothing_stored() {
    let admin = Admin::new((6_418_302_001, "ada-invites-for-too-long"));
    let since = admin.cloud().relay.mark();

    let refused = invite_at_terminal(&admin.ada.machine, &[DAYS, "31"]);

    assert!(!refused.success, "{}", refused.text);
    assert!(
        refused.text.contains("30"),
        "it does not say why: {}",
        refused.text
    );
    assert!(!shows_a_link(&refused.text), "{}", refused.text);
    assert_nothing_stored(&admin, since);
}

/// `skald invite acme --channel <globex's default channel>` at a terminal fails, naming the
/// channel, shows no link and sends no request about invites.
#[test]
fn channel_the_space_lacks_is_refused_with_nothing_stored() {
    let admin = Admin::new((6_418_302_101, "ada-invites-to-globex"));
    let globex = create(admin.cloud(), &admin.ada, GLOBEX);
    let elsewhere = globex.body().channels[0].id.to_string();
    let since = admin.cloud().relay.mark();

    let refused = invite_at_terminal(&admin.ada.machine, &[CHANNEL, &elsewhere]);

    assert!(!refused.success, "{}", refused.text);
    assert!(
        refused.text.contains(&elsewhere),
        "it does not name the channel: {}",
        refused.text
    );
    assert!(!shows_a_link(&refused.text), "{}", refused.text);
    assert_nothing_stored(&admin, since);
}

/// `skald invite acme` with its output a pipe fails, saying it needs a terminal, shows no link and
/// sends no request at all.
#[test]
fn output_to_a_pipe_is_refused_and_nothing_is_sent() {
    let admin = Admin::new((6_418_302_201, "ada-pipes-an-invite"));
    let since = admin.cloud().relay.mark();

    let refused = admin.ada.machine.start(&invite_args(&[])).finish();

    assert!(!refused.success, "{refused:?}");
    assert!(
        refused.shown().to_lowercase().contains("terminal"),
        "it does not say why: {refused:?}"
    );
    assert!(!shows_a_link(&refused.shown()), "{refused:?}");
    assert_nothing_sent(admin.cloud(), since);
}

/// Ada's machine binds `~/work/acme` to `acme`. There, at a terminal, `skald --agent builder invite
/// acme` fails, saying it is the person's command, which an agent cannot run, shows no link and
/// sends no request at all.
#[test]
fn as_an_agent_at_a_terminal_it_refuses_and_sends_nothing() {
    let admin = Admin::new((6_418_302_301, "ada-agent-invites"));
    let machine: &Machine = &admin.ada.machine;
    let folder = dir_of(machine, "work/acme");
    let bound = run(machine, &[ADD_RULE, path(&folder), ACME]);
    assert!(bound.success, "{bound:?}");
    let since = admin.cloud().relay.mark();

    let args = [&[AGENT, AGENT_NAME][..], &invite_args(&[])].concat();
    let refused = Terminal::start_in(machine, &folder, &args).finish();

    assert!(!refused.success, "{}", refused.text);
    let said = refused.text.to_lowercase();
    for part in ["skald invite", "person's command", "an agent cannot run"] {
        assert!(
            said.contains(part),
            "it does not say {part:?}: {}",
            refused.text
        );
    }
    assert!(!shows_a_link(&refused.text), "{}", refused.text);
    assert_nothing_sent(admin.cloud(), since);
}

/// Ada's machine created `acme`; its revision 3, signed by her device, makes Bob an admin, and
/// revision 4, signed by Bob's device, makes Ada a member, which her device accepts. `skald invite
/// acme` at a terminal on her machine fails, saying `admin`, shows no link and sends no request
/// about invites.
#[test]
fn device_of_a_member_who_is_no_admin_refuses_and_sends_nothing() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(6_418_302_401, "ada-no-longer-admin", "ada-workstation");
    let bob = world.sign_up(6_418_302_402, "bob-the-admin", "bob-workstation");
    let second = create(cloud, &ada, ACME);
    let third = next_by(&second, &ada, |list| list.push(bob.member(Role::Admin)));
    store(cloud, &ada.token, &third);
    let fourth = next_by(&third, &bob, |list| {
        set_role(list, ada.account, Role::Member)
    });
    store(cloud, &bob.token, &fourth);
    assert_role(&members(&ada.machine, ACME), &ada.handle, MEMBER);
    let since = cloud.relay.mark();

    let refused = invite_at_terminal(&ada.machine, &[]);

    assert!(!refused.success, "{}", refused.text);
    assert!(
        refused.text.to_lowercase().contains(ADMIN),
        "it does not say why: {}",
        refused.text
    );
    assert!(!shows_a_link(&refused.text), "{}", refused.text);
    assert_eq!(invite_requests(cloud, since), Vec::<String>::new());
    assert_eq!(issued_since(cloud, since), Vec::new(), "invite IDs issued");
}
