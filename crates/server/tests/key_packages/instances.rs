//! Two requests at once never get one KeyPackage, also through two instances on one database.

use std::sync::Barrier;
use std::thread;

use crate::common::devices::assert_taken;
use crate::fixture::{Stand, fresh, hand_out_one, last_resort, mls, upload};

/// How many times the tests race two requests.
const ROUNDS: usize = 8;

/// Alice's laptop has its last-resort KeyPackage and one other left. Bob through the fixture's
/// instance and carol through another ask for one at the same moment: one gets the other
/// KeyPackage and the other the last-resort one, never both the same other one.
#[test]
fn two_requests_at_once_through_two_instances_never_get_one_key_package() {
    let stand = Stand::new(3_130_050_001);
    let team = &stand.team;
    let laptop = team.alice.laptop.id;
    let second = team.flow.stand.instance(&[], &[]);
    let (bob, carol) = (team.bob.session.clone(), team.carol.session.on(&second));
    let mut device = mls(&team.alice.laptop);
    let last = last_resort(&mut device);
    assert_taken(&upload(&team.alice.session, laptop, &[], Some(&last)));

    for round in 0..ROUNDS {
        let other = fresh(&mut device, 1).remove(0);
        assert_taken(&upload(
            &team.alice.session,
            laptop,
            std::slice::from_ref(&other),
            None,
        ));
        let barrier = Barrier::new(2);
        let (by_bob, by_carol) = thread::scope(|scope| {
            let ask = |session| {
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    hand_out_one(session, laptop)
                })
            };
            let (bob, carol) = (ask(&bob), ask(&carol));
            (
                bob.join().expect("bob's request"),
                carol.join().expect("carol's request"),
            )
        });

        let mut got = [by_bob, by_carol].map(|one| one.expect("a KeyPackage for each"));
        got.sort();
        let mut expected = [other, last.clone()];
        expected.sort();
        assert_eq!(
            got, expected,
            "round {round}: the other one and the last-resort one"
        );
    }
}
