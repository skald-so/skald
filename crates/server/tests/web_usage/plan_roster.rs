//! Signed roster revisions enforce each space's effective member and service-agent limits.

use serde_json::json;
use skald_proto::{ChannelMember, NonEmpty, Role, RosterBody, ServiceAgent};

use crate::api::{Person, Team, assert_refused};
use crate::common::Response;
use crate::common::device_flow::{json as body_of, text};
use crate::common::devices::{Device, assert_taken, b64};
use crate::invites::{acceptance, gets_no_invite, made};
use crate::plans::{CLOUD, entitle_team, remove_entitlement};
use crate::rosters::{channel, member, name, next, service_agent};
use crate::upload::issued_by;
use crate::usage::space_usage_path;

const MEMBER_LIMIT: &str = "https://skald.so/problems/members-limit";
const SERVICE_AGENT_LIMIT: &str = "https://skald.so/problems/service-agents-limit";

fn signed(team: &Team, previous: &[Vec<u8>], change: impl FnOnce(&mut RosterBody)) -> Vec<u8> {
    let mut body = next(
        previous.last().expect("stored revision"),
        &team.alice.laptop,
    );
    change(&mut body);
    team.alice.sign(&body)
}

#[track_caller]
fn store(team: &Team, kept: &mut Vec<Vec<u8>>, revision: Vec<u8>) {
    let response = team.alice.session.store(team.space, &revision);
    assert_eq!(response.status, 201, "{}", text(&response));
    kept.push(revision);
    assert_eq!(team.alice.session.revisions(team.space), *kept);
}

#[track_caller]
fn assert_quota_refusal(response: &Response, what: &str, problem_type: &str) {
    assert_refused(response, what);
    assert_eq!(response.status, 403, "{what}: {}", text(response));
    let problem = body_of(response);
    assert_eq!(problem["type"], problem_type, "{what}: {}", text(response));
    assert_eq!(problem["status"], 403, "{what}: {}", text(response));
}

#[track_caller]
fn assert_agent_refusal(response: &Response) {
    assert_quota_refusal(
        response,
        "adding an agent above the plan limit",
        SERVICE_AGENT_LIMIT,
    );
}

/// Makes a service device through the same public setup as a member creating a service agent.
#[track_caller]
fn new_agent(owner: &Person, index: usize) -> ServiceAgent {
    let device = Device::new(&format!("Bot {index}"));
    let mut registration = device.registration();
    registration["kind"] = json!("service");
    assert_taken(&owner.session.call("post", "/devices", Some(&registration)));

    let cert = issued_by(
        &device,
        owner.account,
        skald_proto::DeviceKind::Service,
        &owner.laptop,
    );
    let path = format!("/devices/{}/cert", device.id);
    assert_taken(
        &owner
            .session
            .call("put", &path, Some(&json!({ "cert": b64(&cert) }))),
    );

    let mut agent = service_agent(&device);
    agent.agent = name(&format!("bot-{index}"));
    agent
}

#[track_caller]
fn assert_agent_usage(team: &Team, count: u64, limit: u64) {
    let response = team
        .alice
        .session
        .call("get", &space_usage_path(team.space), None);
    assert_eq!(response.status, 200, "{}", text(&response));
    assert_eq!(
        body_of(&response)["service_agents"],
        json!({ "count": count, "limit": limit })
    );
}

#[test]
fn a_fourth_member_waits_for_team_when_admitted_by_invite() {
    let team = Team::with(3_226_930_001, CLOUD);
    let (alice, bob, carol, dave) = (&team.alice, &team.bob, &team.carol, &team.dave);
    let mut kept = team.with_members().to_vec();
    let general = alice.session.channel(team.space);
    let with_channel = signed(&team, &kept, |body| {
        body.channels.push(channel(general, &[alice, bob, carol]));
    });
    store(&team, &mut kept, with_channel);

    let invite = made(
        team.space,
        alice,
        &alice.laptop,
        vec![(general, true)],
        |_| {},
    );
    assert_taken(&alice.session.put_invite(team.space, &invite));
    assert_taken(&dave.session.accept(invite.id, &acceptance(&invite, dave)));
    let waiting = alice.session.waiting(team.space);
    assert_eq!(waiting.len(), 1);
    let admission = signed(&team, &kept, |body| {
        let mut invited = member(dave, Role::Member);
        invited.invite = Some(invite.id);
        body.members.push(invited);
        body.channels[0].members.push(ChannelMember {
            account: dave.account,
            history: true,
        });
    });

    assert_quota_refusal(
        &alice.session.store(team.space, &admission),
        "a fourth Free member",
        MEMBER_LIMIT,
    );
    assert_eq!(alice.session.revisions(team.space), kept);
    assert_eq!(alice.session.waiting(team.space), waiting);
    assert!(!dave.session.spaces().contains_key(&team.space.to_string()));

    entitle_team(&team.flow.stand.db, team.space);
    store(&team, &mut kept, admission);
    assert!(dave.session.spaces().contains_key(&team.space.to_string()));
    assert!(alice.session.waiting(team.space).is_empty());
    gets_no_invite(dave, invite.id, Some(&invite), "the revision admitted Dave");
}

#[test]
fn free_allows_three_service_agents_and_team_allows_ten() {
    let team = Team::with(3_226_930_011, CLOUD);
    let alice = &team.alice;
    let mut kept = team.with_members().to_vec();
    let mut agents = (1..=11).map(|index| new_agent(alice, index));
    let three = signed(&team, &kept, |body| {
        body.service_agents.extend(agents.by_ref().take(3))
    });
    store(&team, &mut kept, three);
    assert_agent_usage(&team, 3, 3);

    let fourth = signed(&team, &kept, |body| {
        body.service_agents
            .push(agents.next().expect("fourth agent"));
    });
    assert_agent_refusal(&alice.session.store(team.space, &fourth));
    assert_eq!(alice.session.revisions(team.space), kept);
    assert_agent_usage(&team, 3, 3);

    entitle_team(&team.flow.stand.db, team.space);
    store(&team, &mut kept, fourth);
    let ten = signed(&team, &kept, |body| {
        body.service_agents.extend(agents.by_ref().take(6))
    });
    store(&team, &mut kept, ten);
    assert_agent_usage(&team, 10, 10);

    let eleventh = signed(&team, &kept, |body| {
        body.service_agents
            .push(agents.next().expect("eleventh agent"));
    });
    assert_agent_refusal(&alice.session.store(team.space, &eleventh));
    assert_eq!(alice.session.revisions(team.space), kept);
    assert_agent_usage(&team, 10, 10);
}

#[test]
fn a_lowered_plan_preserves_over_limit_rosters_and_only_allows_edits_or_reductions() {
    let team = Team::with(3_226_930_021, CLOUD);
    let (alice, bob, dave) = (&team.alice, &team.bob, &team.dave);
    let eve = Person::new(&team.flow, 3_226_930_025, "eve");
    let db = &team.flow.stand.db;
    entitle_team(db, team.space);
    let mut kept = team.with_members().to_vec();
    let five = signed(&team, &kept, |body| {
        body.members.push(member(dave, Role::Member));
        body.members.push(member(&eve, Role::Member));
        body.service_agents
            .extend((1..=5).map(|index| new_agent(alice, index)));
    });
    store(&team, &mut kept, five.clone());

    remove_entitlement(db, team.space);
    assert_eq!(alice.session.revisions(team.space), kept);
    assert_agent_usage(&team, 5, 3);
    let repeated = alice.session.store(team.space, &five);
    assert_eq!(repeated.status, 200, "{}", text(&repeated));
    assert_eq!(alice.session.revisions(team.space), kept);

    let edited = signed(&team, &kept, |body| {
        let mut members = body.members.clone().into_vec();
        members
            .iter_mut()
            .find(|entry| entry.account == bob.account)
            .expect("Bob's membership")
            .role = Role::Member;
        body.members = NonEmpty::from_vec(members).expect("a nonempty roster");
        body.service_agents[0].agent = name("renamed-bot");
    });
    store(&team, &mut kept, edited);
    let reduced = signed(&team, &kept, |body| {
        let mut members = body.members.clone().into_vec();
        members.retain(|entry| entry.account != eve.account);
        body.members = NonEmpty::from_vec(members).expect("a nonempty roster");
        body.service_agents.truncate(4);
    });
    store(&team, &mut kept, reduced);
    assert_agent_usage(&team, 4, 3);

    let regrow_members = signed(&team, &kept, |body| {
        body.members.push(member(&eve, Role::Member))
    });
    assert_quota_refusal(
        &alice.session.store(team.space, &regrow_members),
        "adding a member above a lowered limit",
        MEMBER_LIMIT,
    );
    assert_eq!(alice.session.revisions(team.space), kept);

    let regrow_agents = signed(&team, &kept, |body| {
        body.service_agents.push(new_agent(alice, 6))
    });
    assert_agent_refusal(&alice.session.store(team.space, &regrow_agents));
    assert_eq!(alice.session.revisions(team.space), kept);
    assert_agent_usage(&team, 4, 3);
}
