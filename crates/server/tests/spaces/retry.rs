//! A retry after a lost answer from an admin whose revision took its account out of the space.

use skald_proto::NonEmpty;

use crate::api::{Team, assert_refused};
use crate::common::devices::assert_taken;
use crate::rosters::next;

impl Team {
    /// Stores revisions 1 and 2 of `with_members`, then revision 3, signed by alice's laptop, which
    /// leaves bob and carol; gives the three.
    pub fn alice_left(&self) -> Vec<Vec<u8>> {
        let alice = &self.alice;
        let stored = self.with_members();
        let mut body = next(&stored[1], &alice.laptop);
        let others = body
            .members
            .iter()
            .filter(|member| member.account != alice.account);
        body.members = NonEmpty::from_vec(others.cloned().collect()).expect("a member");
        let third = alice.sign(&body);
        self.stored_by(alice, &third);
        [stored.to_vec(), vec![third]].concat()
    }
}

#[test]
fn an_admin_that_took_its_account_out_retries_its_revision_and_gets_nothing_else() {
    let team = Team::new(3_120_860_001);
    let (alice, bob) = (&team.alice, &team.bob);
    let stored = team.alice_left();
    assert_taken(&alice.session.store(team.space, &stored[2]));
    assert_eq!(bob.session.revisions(team.space), stored);
    assert!(!alice.session.spaces().contains_key(&team.space.to_string()));
    assert_refused(&alice.session.revisions_response(team.space), "revisions");
    let fourth = alice.sign(&next(&stored[2], &alice.laptop));
    assert_refused(&alice.session.store(team.space, &fourth), "revision 4");
    assert_eq!(bob.session.revisions(team.space), stored);
}
