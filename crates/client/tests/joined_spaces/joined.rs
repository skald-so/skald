//! What the tests share: Ada's first machine and her second, linked by hand before the spaces the
//! tests follow; the anchors objects a compromised server withholds or adds; and what the second
//! machine's runs show.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use skald_crypto::{SigningKey, sign_anchors};
use skald_proto::{AccountId, Anchors, AnchorsBody, Cbor as _, DeviceId, Roster, SpaceId, Unknown};

use crate::anchors::held;
use crate::cloud::Cloud;
use crate::devices::second_machine;
use crate::fixtures::fronted;
use crate::front::Front;
use crate::kit::{create_space, latest, received, rosters};
use crate::machine::{Finished, Machine};
use crate::talk::ADA_HOST;
use crate::world::{
    ACME, Person, World, holds_word, next_by, next_with, plain_lines, serve_revisions, spaces,
    store, warnings,
};

/// The host names of Ada's second machine and of her third device, their certificates' `name`.
pub const LAPTOP: &str = "ada-laptop";
pub const TABLET: &str = "ada-tablet";

/// The request whose answer gives the account's anchors objects, as its line starts.
pub const GIVE_ANCHORS: &str = "GET /account/anchors ";

/// Ada's account: her first machine, which signed up, and her second, linked by hand.
pub struct Account {
    pub world: World,
    pub front: Front,
    pub ada: Person,
    pub laptop: Person,
}

impl Account {
    /// GitHub user `id` with `login` signs up on Ada's first machine, which creates the spaces
    /// `before`; then her second machine is linked by hand, so it follows those, and runs `skald
    /// recv` once.
    pub fn new(id: u64, login: &str, before: &[&str]) -> Self {
        let (world, front) = fronted();
        let ada = world.sign_up(id, login, ADA_HOST);
        for name in before {
            create_space(&world.cloud, &ada, name);
        }
        let laptop = second_machine(&world, &ada, LAPTOP);
        received(&laptop.machine);
        Account {
            world,
            front,
            ada,
            laptop,
        }
    }

    pub fn cloud(&self) -> &Cloud {
        &self.world.cloud
    }

    /// A third device of Ada's account, linked by hand, whose machine never runs.
    pub fn tablet(&self) -> Person {
        second_machine(&self.world, &self.ada, TABLET)
    }
}

/// An anchors object of `account` by `device`, signed with `key`, anchoring each space of `anchors`
/// at the revision given with it and naming no personal space.
pub fn anchors_object(
    account: AccountId,
    (device, key): (DeviceId, &SigningKey),
    anchors: &[(SpaceId, &Roster)],
) -> Anchors {
    let body = AnchorsBody {
        account,
        device,
        anchors: (anchors.iter())
            .map(|(space, revision)| (*space, revision.hash()))
            .collect(),
        personal: None,
        unknown: Unknown::default(),
    };
    sign_anchors(&body, key).expect("an anchors object")
}

/// `anchors` with its signature broken.
pub fn with_broken_signature(anchors: &Anchors) -> Anchors {
    let mut signature = *anchors.signature();
    signature[0] ^= 1;
    Anchors::from_parts(anchors.body_bytes().to_vec(), signature).expect("an anchors object")
}

/// From now on the relay gives `added` among the account's anchors objects, after those the server
/// holds, as a compromised server would.
pub fn give_anchors_too(cloud: &Cloud, added: &[Anchors]) {
    let added: Vec<Value> = (added.iter())
        .map(|object| Value::String(URL_SAFE_NO_PAD.encode(object.to_cbor())))
        .collect();
    cloud.relay.rewrite(GIVE_ANCHORS, move |answer| {
        let mut given: Value = serde_json::from_str(answer).unwrap_or(Value::Null);
        if let Some(list) = given["anchors"].as_array_mut() {
            list.extend(added.iter().cloned());
        }
        given.to_string()
    });
}

/// The relay withholding the account's anchors objects, as a compromised server would: while on,
/// it gives none of them.
#[derive(Clone)]
pub struct Withheld {
    on: Arc<AtomicBool>,
    /// How many answers it has emptied.
    emptied: Arc<AtomicUsize>,
}

impl Withheld {
    /// From now on the relay gives no anchors object, until [`Withheld::stop`].
    pub fn start(cloud: &Cloud) -> Self {
        let withheld = Withheld {
            on: Arc::new(AtomicBool::new(true)),
            emptied: Arc::new(AtomicUsize::new(0)),
        };
        let rule = withheld.clone();
        cloud.relay.rewrite(GIVE_ANCHORS, move |answer| {
            if !rule.on.load(Ordering::SeqCst) {
                return answer.to_owned();
            }
            let mut given: Value = serde_json::from_str(answer).unwrap_or(Value::Null);
            given["anchors"] = json!([]);
            rule.emptied.fetch_add(1, Ordering::SeqCst);
            given.to_string()
        });
        withheld
    }

    /// How many answers the relay has emptied so far.
    pub fn emptied(&self) -> usize {
        self.emptied.load(Ordering::SeqCst)
    }

    /// From now on the relay gives the anchors objects as the server does.
    pub fn stop(&self) {
        self.on.store(false, Ordering::SeqCst);
    }
}

/// Whether `listed`, what `skald spaces` showed, lists the space `name` as followed.
pub fn follows(listed: &Finished, name: &str) -> bool {
    holds_word(&plain_lines(listed), name)
}

/// The machine's next run: `skald recv`, then `skald spaces`, each failing the test unless it
/// succeeds. Gives what `skald spaces` showed and the warnings of both runs, in lowercase.
#[track_caller]
pub fn next_run(machine: &Machine) -> (Finished, Vec<String>) {
    let ran = received(machine);
    let listed = spaces(machine);
    let said = [warnings(&ran), warnings(&listed)].concat();
    (listed, said)
}

/// Whether one of `warnings`, in lowercase, names the space `name` with the ID `space`.
pub fn warn_of(warnings: &[String], (name, space): (&str, SpaceId)) -> bool {
    let id = space.to_string().to_lowercase();
    (warnings.iter()).any(|line| line.contains(name) || line.contains(&id))
}

/// The hash of the revision of `space` that the anchors object the server holds of `person`'s
/// device names, if the server holds one that names `space`.
pub fn anchored(cloud: &Cloud, person: &Person, space: SpaceId) -> Option<[u8; 32]> {
    (held(cloud, &person.token).into_iter())
        .find(|object| object.device == person.device)
        .and_then(|object| object.anchors.get(&space).copied())
}

/// Ada's third device is linked, and her second machine runs. Her first machine creates `acme`, so
/// its anchors object names `acme` at revision 2; a revision 3, signed as her first device, is
/// stored. From then on the relay gives an anchors object of her third device, valid and signed
/// with its key, that names `acme` at another revision 3, a fork of revision 2, and gives that fork
/// among `acme`'s revisions: the two anchors of `acme` are off one chain. Her first machine must
/// not run again. Gives `acme`.
pub fn anchors_off_one_chain(account: &Account) -> SpaceId {
    let (cloud, ada) = (account.cloud(), &account.ada);
    let tablet = account.tablet();
    // The second machine warns of the new device at this run, before `acme` exists.
    received(&account.laptop.machine);
    let acme = create_space(cloud, ada, ACME).id;
    let second = latest(cloud, &ada.token, acme);
    assert_eq!(
        anchored(cloud, ada, acme),
        Some(second.hash()),
        "the first device's anchors object does not name revision 2 of acme"
    );
    let third = next_by(&second, ada, |_| {});
    store(cloud, &ada.token, &third);
    let fork = next_with(&second, (ada.device, &ada.key), |body| {
        body.timestamp += 1_000;
    });
    assert_ne!(fork.hash(), third.hash(), "not a fork");
    let forged = anchors_object(ada.account, (tablet.device, &tablet.key), &[(acme, &fork)]);
    give_anchors_too(cloud, &[forged]);
    let mut given = rosters(cloud, &ada.token, acme);
    given.push(fork);
    serve_revisions(cloud, acme, &given.iter().collect::<Vec<_>>());
    acme
}
