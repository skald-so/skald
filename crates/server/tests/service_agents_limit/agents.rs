//! What the tests share: the setting and its problem, `acme` with service agents on alice's
//! `service` devices, revisions stored on any instance of its server, and the space's usage.

use serde_json::Value;
use skald_proto::{DeviceKind, RosterBody, ServiceAgent, SpaceId, Unknown};

use crate::api::{Person, Session, assert_refused};
use crate::common::device_flow::{json, text};
use crate::common::devices::assert_taken;
use crate::common::{Response, Server, TestDb};
use crate::fixture::Fixture;
use crate::rosters::{name, next};

/// The setting, as an argument and as an environment variable.
pub const LIMIT: &str = "--service-agents-limit";
pub const LIMIT_ENV: &str = "SKALD_SERVICE_AGENTS_LIMIT";
/// The problem of a revision over the limit.
pub const OVER_AGENTS: &str = "https://skald.so/problems/service-agents-limit";

/// The space's usage: `GET` with a member's session.
pub fn usage_path(space: SpaceId) -> String {
    format!("/spaces/{space}/usage")
}

/// A service agent entry `ci/bot-<n>` on a new `service` device of `person`'s account, certified
/// by its laptop and written to `db`.
pub fn agent_of(person: &Person, db: &TestDb, n: usize) -> ServiceAgent {
    let device = person.certified_by(db, &person.laptop, DeviceKind::Service, &format!("Bot {n}"));
    ServiceAgent {
        device: device.id,
        owner: name("ci"),
        agent: name(&format!("bot-{n}")),
        unknown: Unknown::default(),
    }
}

/// The fixture's space `acme`, its revisions stored so far, and the service devices of alice's
/// account written so far.
pub struct Acme {
    pub f: Fixture,
    /// Service devices written so far; each new entry gets a device of its own.
    written: usize,
}

impl Acme {
    /// The fixture's space on a server with the extra arguments `args`; GitHub user IDs from
    /// `github` on.
    pub fn with(github: u64, args: &[&str]) -> Self {
        Acme {
            f: Fixture::with(github, args),
            written: 0,
        }
    }

    pub fn space(&self) -> SpaceId {
        self.f.team.space
    }

    /// A service agent entry on a new `service` device of alice's account, as [`agent_of`] gives
    /// it.
    pub fn new_agent(&mut self) -> ServiceAgent {
        self.written += 1;
        agent_of(&self.f.team.alice, self.f.db(), self.written)
    }

    /// The service agent entries of the latest stored revision.
    pub fn agents(&self) -> usize {
        self.body().service_agents.len()
    }

    /// The body of the revision after the latest stored one, signed by alice's laptop.
    fn body(&self) -> RosterBody {
        let last = self.f.revisions.last().expect("a revision");
        next(last, &self.f.team.alice.laptop)
    }

    /// The revision after the latest stored one, with `change` applied to its body, signed by
    /// alice's laptop.
    pub fn signed(&self, change: impl FnOnce(&mut RosterBody)) -> Vec<u8> {
        let mut body = self.body();
        change(&mut body);
        self.f.team.alice.sign(&body)
    }

    /// Alice's session on `server`, another instance on the space's database, or on the fixture's
    /// own if `None`.
    pub fn alice_on(&self, server: Option<&Server>) -> Session {
        let session = &self.f.team.alice.session;
        server.map_or_else(|| session.clone(), |server| session.on(server))
    }

    /// Stores, from alice's session on `server` as [`Acme::alice_on`] picks it, the next revision
    /// with `change` applied; gives the answer, keeping the revision when the server took it.
    pub fn propose(
        &mut self,
        server: Option<&Server>,
        change: impl FnOnce(&mut RosterBody),
    ) -> Response {
        let revision = self.signed(change);
        let response = self.alice_on(server).store(self.space(), &revision);
        if matches!(response.status, 200 | 201) {
            self.f.revisions.push(revision);
        }
        response
    }

    /// As [`Acme::propose`], adding `count` new service agents; the server must take it.
    #[track_caller]
    pub fn add_agents(&mut self, server: Option<&Server>, count: usize) {
        let added: Vec<ServiceAgent> = (0..count).map(|_| self.new_agent()).collect();
        let before = self.agents();
        let response = self.propose(server, |body| body.service_agents.extend(added));
        assert_taken(&response);
        assert_eq!(self.agents(), before + count, "the agents stored");
    }

    /// Checks that the server refuses, as over the limit, a revision from alice's session on
    /// `server` that adds `count` new service agents, and keeps the space's revisions as they were;
    /// `what` names the case.
    #[track_caller]
    pub fn assert_refused_adding(&mut self, server: Option<&Server>, count: usize, what: &str) {
        let added: Vec<ServiceAgent> = (0..count).map(|_| self.new_agent()).collect();
        let revision = self.signed(|body| body.service_agents.extend(added));
        let session = self.alice_on(server);
        assert_over_limit(&session.store(self.space(), &revision), what);
        assert_over_limit(
            &session.store(self.space(), &revision),
            &format!("{what}, again"),
        );
        assert_eq!(
            session.revisions(self.space()),
            self.f.revisions,
            "{what}: the stored revisions"
        );
    }

    /// The space's service agents as its usage gives them to `session`, a member's on any instance:
    /// `{"count": <n>, "limit": <n>}`.
    #[track_caller]
    pub fn usage_of_agents(&self, session: &Session) -> Value {
        let response = session.call("get", &usage_path(self.space()), None);
        assert_eq!(response.status, 200, "{}", text(&response));
        let usage = json(&response);
        usage
            .get("service_agents")
            .unwrap_or_else(|| panic!("no service_agents in the usage: {usage}"))
            .clone()
    }
}

/// Checks that `response` refuses a revision as over the service agents limit: `403` with its
/// problem; `what` names the case.
#[track_caller]
pub fn assert_over_limit(response: &Response, what: &str) {
    assert_refused(response, what);
    let shown = text(response);
    assert_eq!(response.status, 403, "{what}: {shown}");
    assert_eq!(json(response)["type"], OVER_AGENTS, "{what}: {shown}");
}
