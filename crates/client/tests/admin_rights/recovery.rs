//! A device linked with the recovery key has its account's admin rights: with every device of the
//! space's only admin wiped, a new machine linked with the key makes a member an admin, and the
//! members' devices accept the revision.

use skald_proto::Role;

use crate::edits::last_two;
use crate::kit::rosters;
use crate::machine::Machine;
use crate::recovering::{LOST, NEW_HOST, new_device, recovered, sign_up};
use crate::roles::{RoleCommand, assert_roles, role_set};
use crate::talk::{BOB_HOST, CAROL_HOST};
use crate::world::{ACME, World, create, members, next_by, store};

/// The host name of Ada's first machine.
const FIRST_HOST: &str = "ada-wiped-laptop";

/// Ada's first machine, made at a terminal with a recovery key, created `acme`, and its device
/// signed the revision that adds Bob and Carol as members; it accepts it, so its anchors object
/// names it. Then the machine is wiped: it runs no more. A new machine of Ada's links with the
/// recovery key, naming the first device lost, and its `skald make-admin acme <bob>` signs the next
/// revision with Bob an admin alone, which Bob's and Carol's devices accept.
#[test]
fn device_linked_with_the_recovery_key_makes_a_member_an_admin() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = sign_up(&world, (4_301_107_001, "ada-wiped-all"), FIRST_HOST);
    let bob = world.sign_up(4_301_107_002, "bob-saved-by-key", BOB_HOST);
    let carol = world.sign_up(4_301_107_003, "carol-sees-recovery", CAROL_HOST);
    let first = &ada.person;
    let created = create(cloud, first, ACME);
    let space = created.body().space;
    let with_them = next_by(&created, first, |members| {
        members.push(bob.member(Role::Member));
        members.push(carol.member(Role::Member));
    });
    store(cloud, &first.token, &with_them);
    members(&first.machine, ACME);
    let machine = Machine::new().with_host_name(NEW_HOST);
    recovered(&world, &ada, &machine, &[(LOST, first.device)]);
    let new = new_device(cloud, first, &[first.device]);
    let held = rosters(cloud, &bob.token, space).len();

    role_set(&machine, RoleCommand::MakeAdmin, ACME, &bob);

    let (previous, next) = last_two(cloud, &bob.token, space, held);
    let roles = RoleCommand::MakeAdmin.roles(first.account, bob.account);
    assert_roles(cloud, (&previous, &next), new, &roles, &[&bob, &carol]);
}
