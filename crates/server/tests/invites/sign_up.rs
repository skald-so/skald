//! A closed server can use a device authorization's invite proof to create one unlisted account.
//! The proof names its invite and the server-issued device code. Only the new account can read
//! and accept the invite. When an identity can sign in anyway, the invite stays available.

use serde_json::json;
use skald_crypto::{InviteSecret, SigningKey, sign_acceptance};
use skald_github_stand_in::User;
use skald_proto::{AcceptanceBody, Cbor, ChannelId, InviteBody, SpaceId, Unknown};

use crate::api::{ACME, Person, Session, assert_refused};
use crate::common::browser::{Browser, Page, SIGN_IN_GITHUB, Visit};
use crate::common::device_flow::{DeviceFlow, Grant, INTERVAL, Poll, text};
use crate::common::devices::{Device, Key, account_id, assert_taken, b64, master_request};
use crate::common::sign_in::Account;
use crate::rosters::{channel, first, next};
use crate::scene::{Made, acceptance, gets_no_invite, made, signed_acceptance, wait_past};
use crate::wire::now;

const LISTED: &str = "alice-spaces-standin,eve-spaces-standin,frank-spaces-standin,listed-invitee";
const INVITE_PROOF: &str = "/auth/device/invite";

struct Setup {
    flow: DeviceFlow,
    alice: Person,
    eve: Person,
    eve_id: u64,
    space: SpaceId,
    default: ChannelId,
}

impl Setup {
    fn new(id: u64, closed: bool) -> Self {
        let mut args = vec![INTERVAL, "1"];
        if closed {
            args.extend(["--sign-up", "closed", "--sign-up-github-logins", LISTED]);
        }
        let flow = DeviceFlow::start_with(&args);
        let alice = Person::new(&flow, id, "alice");
        let eve_id = id + 1;
        let eve = Person::new(&flow, eve_id, "eve");
        let space = alice.create_space(ACME);
        let initial = alice.sign(&first(space, ACME, &alice));
        assert_taken(&alice.session.store(space, &initial));
        let default = alice.session.channel(space);
        let mut revision = next(&initial, &alice.laptop);
        revision.channels.push(channel(default, &[&alice]));
        assert_taken(&alice.session.store(space, &alice.sign(&revision)));
        Self {
            flow,
            alice,
            eve,
            eve_id,
            space,
            default,
        }
    }

    fn invite(&self) -> Made {
        self.invite_with(|_| {})
    }

    fn invite_with(&self, change: impl FnOnce(&mut InviteBody)) -> Made {
        let invite = made(
            self.space,
            &self.alice,
            &self.alice.laptop,
            vec![(self.default, true)],
            change,
        );
        assert_taken(&self.alice.session.put_invite(self.space, &invite));
        invite
    }
}

/// Sign the invite ID and the code the server gave the CLI, using the protocol's byte strings.
fn proof(invite: &Made, grant: &Grant, key: &SigningKey) -> String {
    b64(&key.sign(
        "skald invite sign-up",
        &[invite.id.as_bytes(), grant.device_code.as_bytes()],
    ))
}

/// The machine attaches a proof before showing the verification page to the person.
#[track_caller]
fn attach(flow: &DeviceFlow, invite: &Made, grant: &Grant, signature: &str) {
    let body = json!({
        "device_code": grant.device_code,
        "invite": invite.id.to_string(),
        "proof": signature,
    });
    let response = flow.request(
        "put",
        INVITE_PROOF,
        &[],
        Some(body.to_string().as_bytes()),
        true,
    );
    assert_eq!(
        response.status,
        204,
        "attach invite proof: {}",
        text(&response)
    );
}

#[track_caller]
fn authorize(flow: &DeviceFlow, invite: &Made) -> Grant {
    let grant = flow.authorize();
    let signature = proof(invite, &grant, &invite.secret.invite_key());
    attach(flow, invite, &grant, &signature);
    grant
}

fn starts_sign_in(flow: &DeviceFlow, page: &Page, user: &User, grant: &Grant) -> Visit {
    flow.stand.github_session(page, user);
    let opened = page.goto_settled(&grant.verification_uri_complete);
    assert_eq!(opened.status, Some(200), "{opened:?}");
    assert_eq!(page.count(SIGN_IN_GITHUB), 1, "{}", page.text());
    page.click_settled(&format!("{SIGN_IN_GITHUB} >> nth=0"))
}

fn signs_in(flow: &DeviceFlow, browser: &Browser, user: &User, grant: &Grant) -> String {
    let page = browser.page();
    let visit = starts_sign_in(flow, &page, user, grant);
    assert_eq!(
        visit.status,
        Some(200),
        "{}: {:?}: {}",
        user.login,
        visit,
        page.text()
    );
    let token = flow.approve(&page, grant);
    assert_eq!(flow.account_of(&token).github_login, user.login);
    token
}

fn sign_up_refused(flow: &DeviceFlow, browser: &Browser, user: &User, grant: &Grant) {
    let page = browser.page();
    let visit = starts_sign_in(flow, &page, user, grant);
    assert_closed(flow, user, &page, &visit);
    assert_eq!(flow.wait(grant), Poll::Error("sign_up_closed".to_owned()));
}

#[track_caller]
fn assert_closed(flow: &DeviceFlow, user: &User, page: &Page, visit: &Visit) {
    assert_eq!(
        visit.status,
        Some(403),
        "{}: {visit:?}: {}",
        user.login,
        page.text()
    );
    assert!(
        page.text().to_lowercase().contains("sign-up is closed"),
        "{}",
        page.text()
    );
    flow.stand.assert_signed_out(page);
    assert!(
        flow.stand.rows_of(user).is_empty(),
        "{} acquired an account",
        user.login
    );
}

fn certify(session: &Session, account: &Account, master: &Key) {
    let device = Device::new("Invitee laptop");
    let cert = device.cert(account, master);
    let registration = device.registration();
    assert_taken(&session.call("post", "/devices", Some(&registration)));
    let request = master_request(account_id(account), master, &cert);
    assert_taken(&session.call("put", "/account/master", Some(&request)));
}

#[test]
fn an_invite_signs_up_one_account_and_only_that_account_reads_and_accepts_it() {
    let setup = Setup::new(3_817_710_001, true);
    let flow = &setup.flow;
    let browser = Browser::launch();
    let invite = setup.invite();
    let bob = flow.stand.user(3_817_710_003, "bob-unlisted-invitee");
    let chris = flow.stand.user(3_817_710_004, "chris-unlisted-invitee");
    assert!(flow.stand.rows_of(&bob).is_empty());
    assert_eq!(setup.eve.session.invite_response(invite.id).status, 200);

    let grant = authorize(flow, &invite);
    let second = authorize(flow, &invite);
    // A normal browser sign-in cannot use the proof attached to the pending device code.
    let page = browser.page();
    let visit = flow.stand.sign_in(&page, &bob);
    assert_closed(flow, &bob, &page, &visit);
    assert_eq!(setup.eve.session.invite_response(invite.id).status, 200);

    let token = signs_in(flow, &browser, &bob, &grant);
    let account = flow.account_of(&token);
    let session = Session::of_token(flow, &token);
    assert_eq!(session.invite_response(invite.id).status, 200);
    gets_no_invite(
        &setup.alice,
        invite.id,
        Some(&invite),
        "reserved by sign-up",
    );
    gets_no_invite(&setup.eve, invite.id, Some(&invite), "reserved by sign-up");
    assert!(setup.alice.session.waiting(setup.space).is_empty());
    assert_refused(
        &setup
            .eve
            .session
            .accept(invite.id, &acceptance(&invite, &setup.eve)),
        "another account's acceptance before the invitee accepts",
    );
    sign_up_refused(flow, &browser, &chris, &second);

    let master = Key::generate();
    certify(&session, &account, &master);
    let entry = sign_acceptance(
        &AcceptanceBody {
            invite: invite.id,
            account: account_id(&account),
            master: master.public(),
            handle: "bob".parse().expect("a handle"),
            unknown: Unknown::default(),
        },
        &invite.secret.invite_key(),
    )
    .expect("a signed acceptance")
    .to_cbor();
    assert_taken(&session.accept(invite.id, &entry));
    let waiting = setup.alice.session.waiting(setup.space);
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].acceptance, entry);
}

#[test]
fn two_sign_ups_started_together_leave_only_one_account_with_the_invite() {
    let setup = Setup::new(3_817_710_041, true);
    let flow = &setup.flow;
    let browser = Browser::launch();
    let invite = setup.invite();
    let bob = flow.stand.user(3_817_710_043, "bob-concurrent-invitee");
    let chris = flow.stand.user(3_817_710_044, "chris-concurrent-invitee");
    let attempts = [&bob, &chris].map(|user| {
        let grant = authorize(flow, &invite);
        let page = browser.page();
        let visit = page.goto(&flow.stand.github.asking_sign_in_url(user.id));
        assert_eq!(visit.status, Some(200), "{visit:?}");
        let opened = page.goto_settled(&grant.verification_uri_complete);
        assert_eq!(opened.status, Some(200), "{opened:?}");
        let provider = page.click_settled(&format!("{SIGN_IN_GITHUB} >> nth=0"));
        assert_eq!(provider.status, Some(200), "{provider:?}");
        (user, grant, page)
    });
    // Both browsers reach the provider's confirmation before either sends its callback.
    let clicks = attempts.each_ref().map(|(_, _, page)| (page, "#authorize"));
    let visits = browser.click_all(&clicks);
    let mut statuses: Vec<_> = visits.iter().map(|visit| visit.status).collect();
    statuses.sort_unstable();
    assert_eq!(statuses, [Some(200), Some(403)], "{visits:?}");

    gets_no_invite(&setup.eve, invite.id, Some(&invite), "reserved by sign-up");
    assert_refused(
        &setup
            .eve
            .session
            .accept(invite.id, &acceptance(&invite, &setup.eve)),
        "another account's acceptance",
    );
    for ((user, grant, page), visit) in attempts.into_iter().zip(visits) {
        if visit.status == Some(403) {
            assert_closed(flow, user, &page, &visit);
            assert_eq!(flow.wait(&grant), Poll::Error("sign_up_closed".to_owned()));
            continue;
        }
        let token = flow.approve(&page, &grant);
        let account = flow.account_of(&token);
        assert_eq!(account.github_login, user.login);
        let session = Session::of_token(flow, &token);
        assert_eq!(session.invite_response(invite.id).status, 200);
        let master = Key::generate();
        certify(&session, &account, &master);
        let body = AcceptanceBody {
            invite: invite.id,
            account: account_id(&account),
            master: master.public(),
            handle: "newcomer".parse().expect("a handle"),
            unknown: Unknown::default(),
        };
        let entry = signed_acceptance(&body, &invite.secret.invite_key());
        assert_taken(&session.accept(invite.id, &entry));
        let waiting: Vec<_> = setup
            .alice
            .session
            .waiting(setup.space)
            .into_iter()
            .map(|item| item.acceptance)
            .collect();
        assert_eq!(waiting, [entry]);
    }
}

#[test]
fn a_proof_for_another_key_or_device_code_leaves_the_invite_open() {
    let setup = Setup::new(3_817_710_011, true);
    let flow = &setup.flow;
    let browser = Browser::launch();
    let invite = setup.invite();
    let bob = flow.stand.user(3_817_710_013, "bob-unlisted-invitee");
    let chris = flow.stand.user(3_817_710_014, "chris-wrong-invite-proof");

    let right = flow.authorize();
    let other = flow.authorize();
    assert_ne!(right.device_code, other.device_code);
    let wrong_key = flow.authorize();
    let by_other_key = proof(&invite, &wrong_key, &InviteSecret::generate().invite_key());
    let signature = proof(&invite, &right, &invite.secret.invite_key());
    attach(flow, &invite, &wrong_key, &by_other_key);
    attach(flow, &invite, &other, &signature);

    for grant in [&wrong_key, &other] {
        sign_up_refused(flow, &browser, &chris, grant);
        assert_eq!(setup.eve.session.invite_response(invite.id).status, 200);
        assert!(setup.alice.session.waiting(setup.space).is_empty());
    }

    attach(flow, &invite, &right, &signature);
    signs_in(flow, &browser, &bob, &right);
}

#[test]
fn a_proof_of_an_ended_or_accepted_invite_creates_no_account() {
    let setup = Setup::new(3_817_710_021, true);
    let flow = &setup.flow;
    let browser = Browser::launch();
    let deleted = setup.invite();
    let accepted = setup.invite();
    let deleted_grant = authorize(flow, &deleted);
    let accepted_grant = authorize(flow, &accepted);
    let expires = now() + 10;
    let expired = setup.invite_with(|invite| invite.expires = expires);
    let expired_grant = authorize(flow, &expired);
    assert_taken(
        &setup
            .eve
            .session
            .accept(accepted.id, &acceptance(&accepted, &setup.eve)),
    );
    flow.stand
        .db
        .execute(&format!("DELETE FROM invites WHERE id = '{}'", deleted.id));
    let bob = flow.stand.user(3_817_710_023, "bob-ended-invitee");

    sign_up_refused(flow, &browser, &bob, &deleted_grant);
    sign_up_refused(flow, &browser, &bob, &accepted_grant);
    wait_past(expires);
    sign_up_refused(flow, &browser, &bob, &expired_grant);
}

fn assert_invite_unused(
    setup: &Setup,
    browser: &Browser,
    user: &User,
    existing: Option<&Person>,
    outsider: &Person,
) {
    let flow = &setup.flow;
    let invite = setup.invite();
    let grant = authorize(flow, &invite);
    let token = signs_in(flow, browser, user, &grant);
    if let Some(person) = existing {
        assert_eq!(account_id(&flow.account_of(&token)), person.account);
    }
    let response = outsider
        .session
        .accept(invite.id, &acceptance(&invite, outsider));
    assert_eq!(response.status, 201, "{}: {}", user.login, text(&response));
}

#[test]
fn a_proof_leaves_the_invite_unused_if_the_identity_can_sign_in_without_it() {
    let browser = Browser::launch();
    let open = Setup::new(3_817_710_031, false);
    let newcomer = open.flow.stand.user(3_817_710_033, "open-invitee");
    assert_invite_unused(&open, &browser, &newcomer, None, &open.eve);

    let closed = Setup::new(3_817_710_041, true);
    let listed = closed.flow.stand.user(3_817_710_043, "listed-invitee");
    assert_invite_unused(&closed, &browser, &listed, None, &closed.eve);

    // The provider's stable ID still belongs to the existing account after its login changes.
    let frank = Person::new(&closed.flow, 3_817_710_044, "frank");
    let existing = closed
        .flow
        .stand
        .user(closed.eve_id, "eve-existing-now-unlisted");
    assert_invite_unused(&closed, &browser, &existing, Some(&closed.eve), &frank);
}
