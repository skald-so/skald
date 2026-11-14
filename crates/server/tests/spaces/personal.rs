//! The account's personal space: the server gives each account one, created at its first request,
//! named from the account's username, numbered as a username is when another space has that name.

use std::sync::Barrier;
use std::thread;

use serde_json::json;
use skald_proto::SpaceId;

use crate::api::{Person, Session, assert_refused, space_of};
use crate::common::accounts::{NAME_MAX, Who};
use crate::common::device_flow::{DeviceFlow, json, text};
use crate::common::devices::assert_taken;
use crate::rosters::first;

/// The session's account's personal space: `POST`.
pub const PERSONAL: &str = "/account/personal-space";

/// An OIDC provider's issuer, for a sign-in method of a provider other than GitHub.
const GOOGLE: &str = "https://accounts.google.com";

impl Session {
    /// `POST /account/personal-space`.
    pub fn personal_response(&self) -> crate::common::Response {
        self.call("post", PERSONAL, None)
    }

    /// The personal space of this session's account, which the server must give: its ID and name.
    pub fn personal(&self) -> (SpaceId, String) {
        let response = self.personal_response();
        assert_taken(&response);
        let name = json(&response)["name"]
            .as_str()
            .unwrap_or_else(|| panic!("no name in {}", text(&response)))
            .to_owned();
        (space_of(&response), name)
    }

    /// The session's account, as `GET /session` gives it.
    pub fn who(&self) -> Who {
        Who::of(&self.call("get", "/session", None))
    }

    /// Sets the account's username to `username`, which the server must take.
    pub fn rename(&self, username: &str) {
        let body = json!({ "username": username });
        let response = self.call("put", "/account/username", Some(&body));
        assert!(
            matches!(response.status, 200 | 204),
            "{username}: {}",
            text(&response)
        );
    }
}

/// Fails unless `session`'s account gets the space `space` named `name`, and no other space with
/// that name.
fn assert_lists(session: &Session, space: SpaceId, name: &str) {
    let spaces = session.spaces();
    assert_eq!(spaces.get(&space.to_string()), Some(&name.to_owned()));
    let named = spaces.values().filter(|listed| *listed == name).count();
    assert_eq!(named, 1, "{name}: {spaces:?}");
}

/// The first request creates the account's personal space, named the account's username, which the
/// account owns: it gets it and stores its revision 1. Every later request gives the same space,
/// also after the username changes; another account gets another one.
#[test]
fn personal_space_is_created_at_the_first_request_and_the_same_one_is_given_after() {
    let flow = DeviceFlow::start();
    let alice = Person::new(&flow, 3_120_880_001, "alice");
    let username = alice.session.who().username;

    let created = alice.session.personal_response();
    assert_eq!(created.status, 201, "{}", text(&created));
    let (space, name) = alice.session.personal();
    assert_eq!(space, space_of(&created), "another space");
    assert_eq!(name, username);
    assert_lists(&alice.session, space, &name);

    let revision = alice.sign(&first(space, &name, &alice));
    assert_taken(&alice.session.store(space, &revision));
    assert_eq!(alice.session.personal(), (space, name.clone()));
    alice.session.rename("alice-renamed");
    assert_eq!(alice.session.personal(), (space, name.clone()));
    assert_eq!(alice.session.revisions(space), vec![revision]);
    assert_lists(&alice.session, space, &name);
    assert_eq!(alice.session.spaces().len(), 1, "one space");

    let bob = Person::new(&flow, 3_120_880_002, "bob");
    let (other, _) = bob.session.personal();
    assert_ne!(other, space, "bob got alice's personal space");
    assert!(!bob.session.spaces().contains_key(&space.to_string()));
}

/// Of requests at once from one account, each gets the same personal space, and the account has
/// one.
#[test]
fn requests_at_once_give_one_personal_space() {
    const AT_ONCE: usize = 4;
    let flow = DeviceFlow::start();
    for round in 0..3 {
        let person = Person::new(&flow, 3_120_880_011 + round, &format!("eve{round}"));
        let ready = Barrier::new(AT_ONCE);
        let given: Vec<(SpaceId, String)> = thread::scope(|scope| {
            let running: Vec<_> = (0..AT_ONCE)
                .map(|_| {
                    let (session, ready) = (person.session.clone(), &ready);
                    scope.spawn(move || {
                        ready.wait();
                        session.personal()
                    })
                })
                .collect();
            running
                .into_iter()
                .map(|thread| thread.join().expect("a request at once"))
                .collect()
        });
        assert!(
            given.iter().all(|one| *one == given[0]),
            "round {round}: {given:?}"
        );
        let spaces = person.session.spaces();
        assert_eq!(spaces.len(), 1, "round {round}: {spaces:?}");
        assert_lists(&person.session, given[0].0, &given[0].1);
    }
}

/// Two accounts whose usernames give one space name get two names: the first free of `-2`, `-3`
/// and on is appended, within a `name`'s length, as for a username. A space another account
/// created takes a name just as a personal space does, and a personal space's name is taken for
/// other spaces.
#[test]
fn personal_spaces_whose_names_are_taken_are_numbered() {
    let flow = DeviceFlow::start();
    let alice = Person::new(&flow, 3_120_880_021, "alice");
    let bob = Person::new(&flow, 3_120_880_022, "bob");
    let carol = Person::new(&flow, 3_120_880_023, "carol");
    let dave = Person::new(&flow, 3_120_880_024, "dave");

    // Alice's username, once her personal space has it, goes to bob.
    let (_, first_name) = alice.session.personal();
    assert_eq!(first_name, alice.session.who().username);
    alice.session.rename("alice-renamed");
    bob.session.rename(&first_name);
    let (_, numbered) = bob.session.personal();
    assert_eq!(numbered, format!("{first_name}-2"));
    assert_refused(
        &carol.session.create(&first_name),
        "a personal space's name",
    );

    // Team spaces of other accounts took carol's username and its `-2`.
    let carols = carol.session.who().username;
    alice.create_space(&carols);
    alice.create_space(&format!("{carols}-2"));
    assert_eq!(carol.session.personal().1, format!("{carols}-3"));

    // The number fits within a `name`'s length.
    let longest = "d".repeat(NAME_MAX);
    dave.session.rename(&longest);
    alice.create_space(&longest);
    let (space, name) = dave.session.personal();
    assert_eq!(name, format!("{}-2", "d".repeat(NAME_MAX - 2)));
    assert_lists(&dave.session, space, &name);
}

/// An account whose GitHub sign-in method is replaced with one of another provider has no GitHub
/// login: its personal space is named from its username.
#[test]
fn account_without_a_github_sign_in_method_gets_its_personal_space_named_from_its_username() {
    let flow = DeviceFlow::start();
    let ada = Person::new(&flow, 3_120_880_031, "ada");
    ada.session.rename("ada-of-google");
    flow.stand.db.execute(&format!(
        "UPDATE sign_in_methods SET provider = '{GOOGLE}', subject = '108204371559281736433',
                login = NULL
         WHERE account_id = '{}'",
        ada.account
    ));
    let who = ada.session.who();
    assert_eq!(who.github_login, None, "{who:?}");

    let (space, name) = ada.session.personal();
    assert_eq!(name, "ada-of-google");
    assert_lists(&ada.session, space, &name);
}

#[test]
fn personal_space_needs_a_session() {
    let flow = DeviceFlow::start();
    let response = Session::none(&flow).personal_response();
    assert_eq!(response.status, 401, "{}", text(&response));
}
