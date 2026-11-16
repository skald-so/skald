//! Channel members without earlier history ("Access"): the server gives the sessions of an account
//! whose `history` is false in the latest revision only the links whose `prev` is at or after the
//! epoch whose Commit added the account, and the wraps and entries of that epoch and later ones.
//! The account's devices that join later read from that epoch too; a revision that sets `history`
//! true gives every epoch; an account removed from the channel and added again starts from the
//! epoch that added it again.
//!
//! bob, a member of every epoch, uploads every item; dave is the account added later.

use skald_crypto::{MlsDevice, SigningKey, StateKey};
use skald_proto::{AccountId, Cbor, Cert, ChannelMember, DeviceId, DeviceKind};

use crate::common::devices::{Device, write_devices};
use crate::kit::{
    ENTRIES, Item, LINKS, WRAPS, assert_history, assert_none_given, bob_rotates, entry, formed,
    link, revise_members, stores, wrap,
};
use crate::scene::{Scene, removing};
use crate::wire::now;

/// What bob uploaded: the items held back from a member without earlier history, and those it gets.
#[derive(Default)]
struct Uploaded {
    held: Vec<Item>,
    given: Vec<Item>,
}

impl Uploaded {
    /// bob uploads `item`, which the server stores and holds back from a member without earlier
    /// history.
    #[track_caller]
    fn held(&mut self, s: &mut Scene, item: Item, what: &str) {
        stores(&mut s.bob, &s.fixture, &item, what);
        self.held.push(item);
    }

    /// bob uploads `item`, which the server stores; a member without earlier history gets it.
    #[track_caller]
    fn given(&mut self, s: &mut Scene, item: Item, what: &str) {
        stores(&mut s.bob, &s.fixture, &item, what);
        self.given.push(item);
    }

    /// Every item uploaded.
    fn all(&self) -> Vec<&Item> {
        self.held.iter().chain(&self.given).collect()
    }

    /// The items a member without earlier history gets.
    fn since_the_join(&self) -> Vec<&Item> {
        self.given.iter().collect()
    }
}

/// A wrap of a key of its own of the channel's epoch `epoch` to dave's recovery key, signed by
/// bob's laptop.
fn bobs_wrap(s: &Scene, epoch: u64) -> Item {
    let t = &s.fixture.team;
    wrap(s.fixture.general, epoch, &t.dave, &t.bob)
}

/// Checks that `session` gets exactly `items` of the channel's history, byte for byte.
#[track_caller]
fn assert_gets(s: &Scene, session: &crate::api::Session, items: &[&Item], what: &str) {
    let of = |kind: &str| -> Vec<&Item> {
        (items.iter().copied())
            .filter(|item| {
                matches!(
                    (kind, item),
                    (LINKS, Item::Link(_)) | (WRAPS, Item::Wrap(_)) | (ENTRIES, Item::Entry(..))
                )
            })
            .collect()
    };
    let [links, wraps, entries] = [LINKS, WRAPS, ENTRIES].map(of);
    assert_history(session, &s.fixture, [&links, &wraps, &entries], what);
}

/// Stores the next revision with `account` in the channel with `history`, or out of it if `None`.
fn set_history(s: &mut Scene, account: AccountId, history: Option<bool>) {
    revise_members(s, |members| {
        members.retain(|member| member.account != account);
        if let Some(history) = history {
            members.push(ChannelMember { account, history });
        }
    });
}

/// alice adds `device`, whose MLS state is `mls`, among the devices of `admitted`: the server
/// stores her Commit, she uploads her GroupInfo of the new epoch and sends the Welcome to `device`,
/// which joins from it; bob and carol process the Commit. Gives the new epoch.
#[track_caller]
fn alice_adds(s: &mut Scene, mls: &mut MlsDevice, device: DeviceId, admitted: &[Cert]) -> u64 {
    let package = mls.key_package(now()).expect("a KeyPackage");
    let package = package.as_ref().to_vec();
    let f = &s.fixture;
    let (commit, welcome) = s.alice.commits_with_welcome(f, |alices, channel| {
        alices.add_devices(channel, &[&package], admitted, now())
    });
    let welcome = welcome.expect("a Welcome");
    s.alice.stores(f, &welcome, &[device]);
    let joined = (mls.join(&welcome, now(), |_| true)).expect("joins from the Welcome");
    assert_eq!(joined, f.general, "the channel joined");
    for peer in [&mut s.bob, &mut s.carol] {
        peer.process(f, &commit);
    }
    (s.alice.mls.epoch(f.general)).expect("alice's epoch")
}

/// A second `client` device of dave's account, certified by dave's master key and set up in the
/// database, joins the channel: alice adds it. Gives the new epoch.
#[track_caller]
fn daves_phone_joins(s: &mut Scene) -> u64 {
    let dave = &s.fixture.team.dave;
    let phone = Device::new("Phone");
    let cert = phone.cert_as(dave.account, DeviceKind::Client, &dave.master);
    write_devices(
        s.fixture.db(),
        dave.account,
        DeviceKind::Client,
        &[(&phone, Some(&cert))],
    );
    let signing = SigningKey::from_bytes(&phone.signing.secret);
    let mut mls = MlsDevice::new(&signing, &StateKey::generate());
    let mut admitted = s.admitted();
    admitted.push(Cert::from_cbor(&cert).expect("a certificate"));
    alice_adds(s, &mut mls, phone.id, &admitted)
}

/// The scene with the channel's group formed at epoch 1, then: bob's empty Commit, epoch 2; alice
/// adds dave, epoch 3, dave's join epoch; bob's empty Commit, epoch 4. bob uploads, as each epoch
/// is current, items of every epoch, links to epochs before the join epoch among them, and entries
/// of the previous epoch.
fn dave_joins_at_epoch_3(github: u64, history: bool) -> (Scene, Uploaded) {
    let (mut s, _) = formed(github);
    let dave = s.fixture.team.dave.account;
    set_history(&mut s, dave, Some(history));
    let mut up = Uploaded::default();
    let g = s.fixture.general;

    assert_eq!(bob_rotates(&mut s), 2, "bob's empty Commit starts epoch 2");
    up.held(&mut s, link(g, 2, 1), "a link of epoch 2");
    let [wrap_1, wrap_2] = [1, 2].map(|epoch| bobs_wrap(&s, epoch));
    up.held(&mut s, wrap_1, "a wrap of epoch 1");
    up.held(&mut s, wrap_2, "a wrap of epoch 2");
    up.held(&mut s, entry(g, 1), "an entry of epoch 1");
    up.held(&mut s, entry(g, 2), "an entry of epoch 2");

    s.add_dave();
    let epoch = s.alice.mls.epoch(g).expect("alice's epoch");
    assert_eq!(epoch, 3, "dave joins at 3");
    up.held(&mut s, link(g, 3, 2), "the join epoch's link");
    up.held(&mut s, entry(g, 2), "an entry of epoch 2 in epoch 3");
    let wrap_3 = bobs_wrap(&s, 3);
    up.given(&mut s, wrap_3, "a wrap of epoch 3");
    up.given(&mut s, entry(g, 3), "an entry of epoch 3");

    assert_eq!(bob_rotates(&mut s), 4, "bob's empty Commit starts epoch 4");
    up.held(&mut s, link(g, 4, 2), "a link of epoch 4 to 2");
    up.held(&mut s, link(g, 4, 1), "a link of epoch 4 to 1");
    up.given(&mut s, link(g, 4, 3), "a link of epoch 4 to 3");
    let wrap_4 = bobs_wrap(&s, 4);
    up.given(&mut s, wrap_4, "a wrap of epoch 4");
    up.given(&mut s, entry(g, 3), "an entry of epoch 3 in epoch 4");
    up.given(&mut s, entry(g, 4), "an entry of epoch 4");
    (s, up)
}

#[test]
fn with_history_false_a_member_gets_its_join_epoch_on_and_only_links_from_it() {
    let (mut s, up) = dave_joins_at_epoch_3(3_160_310_001, false);
    let what = "dave, history false";
    assert_gets(&s, &s.dave.session, &up.since_the_join(), what);

    // carol, history false too, has been in the group since epoch 1: she gets every epoch.
    let carol = s.fixture.team.carol.account;
    set_history(&mut s, carol, Some(false));
    let what = "carol, a member since epoch 1";
    assert_gets(&s, &s.carol.session, &up.all(), what);
    assert_gets(&s, &s.alice.session, &up.all(), "alice, the creator");
}

#[test]
fn a_device_of_the_account_that_joins_later_gets_the_history_from_the_accounts_join() {
    let (mut s, mut up) = dave_joins_at_epoch_3(3_160_310_011, false);
    let g = s.fixture.general;
    assert_eq!(daves_phone_joins(&mut s), 5, "dave's phone joins at 5");
    up.held(&mut s, link(g, 5, 2), "a link of epoch 5 to 2");
    up.given(&mut s, link(g, 5, 4), "a link of epoch 5 to 4");
    up.given(&mut s, entry(g, 5), "an entry of epoch 5");
    let wrap_5 = bobs_wrap(&s, 5);
    up.given(&mut s, wrap_5, "a wrap of epoch 5");

    let what = "dave's account, its phone in the group";
    assert_gets(&s, &s.dave.session, &up.since_the_join(), what);
}

#[test]
fn with_history_true_a_member_gets_every_epoch_until_a_revision_sets_it_false() {
    let (mut s, up) = dave_joins_at_epoch_3(3_160_310_021, true);
    assert_gets(&s, &s.dave.session, &up.all(), "dave, history true");

    let dave = s.fixture.team.dave.account;
    set_history(&mut s, dave, Some(false));
    let what = "dave, once a revision sets history false";
    assert_gets(&s, &s.dave.session, &up.since_the_join(), what);
}

#[test]
fn once_a_revision_sets_history_true_a_member_gets_every_epoch() {
    let (mut s, up) = dave_joins_at_epoch_3(3_160_310_031, false);
    let what = "dave, history false";
    assert_gets(&s, &s.dave.session, &up.since_the_join(), what);

    let dave = s.fixture.team.dave.account;
    set_history(&mut s, dave, Some(true));
    let what = "dave, once a revision sets history true";
    assert_gets(&s, &s.dave.session, &up.all(), what);
}

#[test]
fn removed_and_added_again_with_history_false_a_member_gets_nothing_before_the_second_join() {
    let (mut s, _) = formed(3_160_310_041);
    let g = s.fixture.general;
    let dave = s.fixture.team.dave.account;
    set_history(&mut s, dave, Some(false));
    let mut up = Uploaded::default();

    // dave's first join, at epoch 2.
    s.add_dave();
    up.held(&mut s, entry(g, 2), "an entry of epoch 2");
    let wrap_2 = bobs_wrap(&s, 2);
    up.held(&mut s, wrap_2, "a wrap of epoch 2");
    assert_eq!(bob_rotates(&mut s), 3, "bob's empty Commit starts epoch 3");
    up.held(&mut s, link(g, 3, 2), "a link of epoch 3 to 2");
    up.held(&mut s, entry(g, 3), "an entry of epoch 3");

    // A revision without dave in the channel, and alice's Commit removing dave's laptop: epoch 4.
    set_history(&mut s, dave, None);
    let removal = s.alice.commits(&s.fixture, removing(&[s.dave.id]));
    for peer in [&mut s.bob, &mut s.carol] {
        assert_eq!(peer.process(&s.fixture, &removal), 4, "the Remove's epoch");
    }
    up.held(&mut s, link(g, 4, 3), "a link of epoch 4");
    up.held(&mut s, entry(g, 4), "an entry of epoch 4");
    assert_none_given(&s.dave.session, &s.fixture, "dave, out of the channel");

    // A revision with dave again, history false, and alice adding dave's laptop again: epoch 5.
    set_history(&mut s, dave, Some(false));
    let (mut laptop, id, admitted) = (s.dave.twin(), s.dave.id, s.admitted());
    let epoch = alice_adds(&mut s, &mut laptop, id, &admitted);
    assert_eq!(epoch, 5, "dave joins again at 5");
    s.dave.mls = laptop;
    up.held(&mut s, link(g, 5, 4), "the second join epoch's link");
    up.held(&mut s, entry(g, 4), "an entry of epoch 4 in epoch 5");
    up.given(&mut s, entry(g, 5), "an entry of epoch 5");
    let wrap_5 = bobs_wrap(&s, 5);
    up.given(&mut s, wrap_5, "a wrap of epoch 5");

    assert_eq!(bob_rotates(&mut s), 6, "bob's empty Commit starts epoch 6");
    up.held(&mut s, link(g, 6, 3), "a link of epoch 6 to 3");
    up.given(&mut s, link(g, 6, 5), "a link of epoch 6 to 5");
    up.given(&mut s, entry(g, 6), "an entry of epoch 6");

    let what = "dave, added again with history false";
    assert_gets(&s, &s.dave.session, &up.since_the_join(), what);
}
