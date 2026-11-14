//! The space of most tests: `acme`, whose admin alice adds bob, carol and dave as members and the
//! channel `general` of alice, bob and carol. dave is in the space but not in the channel; eve,
//! outside the space, signs in too.

use skald_proto::{
    AccountId, ChannelId, ChannelMember, DeviceId, Header, NonEmpty, Role, RosterBody,
};

use crate::api::{Person, Team};
use crate::common::TestDb;
use crate::rosters::{channel, member, next};
use crate::wire::header;

pub struct Fixture {
    pub team: Team,
    pub eve: Person,
    pub general: ChannelId,
    /// The revisions stored so far, in order.
    pub revisions: Vec<Vec<u8>>,
}

impl Fixture {
    /// The fixture with revision 3 stored; GitHub user IDs from `github` on.
    pub fn new(github: u64) -> Self {
        let team = Team::new(github);
        let eve = Person::new(&team.flow, github + 4, "eve");
        let mut revisions = team.with_members().to_vec();
        let general = team.alice.session.channel(team.space);
        let mut body = next(&revisions[1], &team.alice.laptop);
        body.members.push(member(&team.dave, Role::Member));
        body.channels
            .push(channel(general, &[&team.alice, &team.bob, &team.carol]));
        let third = team.alice.sign(&body);
        team.stored_by(&team.alice, &third);
        revisions.push(third);
        Fixture {
            team,
            eve,
            general,
            revisions,
        }
    }

    /// Stores the next revision, signed by alice's laptop, with `change` applied to its body.
    pub fn revise(&mut self, change: impl FnOnce(&mut RosterBody)) {
        let last = self.revisions.last().expect("a revision");
        let mut body = next(last, &self.team.alice.laptop);
        change(&mut body);
        let revision = self.team.alice.sign(&body);
        self.team.stored_by(&self.team.alice, &revision);
        self.revisions.push(revision);
    }

    /// An `hpke` header in `general` of `acme` for `recipient`.
    pub fn to(&self, recipient: DeviceId) -> Header {
        header(self.team.space, self.general, recipient)
    }

    /// The test database.
    pub fn db(&self) -> &TestDb {
        &self.team.flow.stand.db
    }

    /// The certified laptops of alice, bob, carol, dave and eve.
    pub fn laptops(&self) -> [DeviceId; 5] {
        let t = &self.team;
        [&t.alice, &t.bob, &t.carol, &t.dave, &self.eve].map(|person| person.laptop.id)
    }
}

/// `body` with the `excluded` of `account`'s member entry set to `devices`.
pub fn exclude(body: &mut RosterBody, account: AccountId, devices: &[DeviceId]) {
    let mut members = body.members.clone().into_vec();
    let entry = members
        .iter_mut()
        .find(|entry| entry.account == account)
        .expect("a member");
    entry.excluded = NonEmpty::from_vec(devices.to_vec());
    body.members = NonEmpty::from_vec(members).expect("members");
}

/// `body` with the members of the channel `id` set to `accounts`.
pub fn channel_members(body: &mut RosterBody, id: ChannelId, accounts: &[AccountId]) {
    let entry = body
        .channels
        .iter_mut()
        .find(|entry| entry.id == id)
        .expect("the channel");
    entry.members = accounts
        .iter()
        .map(|&account| ChannelMember {
            account,
            history: false,
        })
        .collect();
}
