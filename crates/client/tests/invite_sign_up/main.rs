//! A fresh CLI at a terminal signs up on a closed server from an invite link. GitHub's stand-in
//! signs the person in through headless Chrome. The server holds their signed acceptance, and
//! the recording relay and the database hold neither the invite secret nor its private key.

#![allow(dead_code)]

#[path = "../recovery_key/at_terminal.rs"]
mod at_terminal;
#[path = "../recovery_key/grants.rs"]
mod grants;
#[path = "../history_common/mod.rs"]
mod history_common;
#[path = "../link_codes/linking.rs"]
mod linking;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
use grants as terminal_grants;

use skald_crypto::{InviteLink, check_acceptance};
use skald_proto::{Acceptance, Cbor as _, SpaceId};
use skald_server::{Listed, SignUp};

use history_common::*;

use browser::{APPROVE, DENY, SIGN_IN_GITHUB, SUBMIT};
use cloud::Providers;
use machine::Machine;
use terminal::Terminal;
use world::{ACME, Person, assert_secret_nowhere, create};

const ADA: &str = "ada-invites-a-new-account";
const CAROL: &str = "carol-already-signed-in";
const BOB_HOST: &str = "bob-fresh-machine";

fn make_invite(ada: &Person) -> InviteLink {
    let issued = Terminal::start(&ada.machine, &["invite", ACME]).finish();
    assert!(issued.success, "{}", issued.text);
    issued
        .text
        .split_whitespace()
        .find_map(|word| InviteLink::parse(word).ok())
        .expect("skald invite shows a link")
}

fn open_invite(machine: &Machine, link: &InviteLink) -> Terminal {
    let mut run = Terminal::start(machine, &["accept-invite"]);
    run.wait_for("a prompt for the invite link", |shown| {
        shown.to_lowercase().contains("link")
    });
    run.type_line(&link.shown());
    run
}

fn confirm_invite(run: &mut Terminal) {
    let summary = run.wait_for("a question to accept the invite", |shown| {
        shown.to_lowercase().contains("accept the invite?")
    });
    assert!(
        summary.split_whitespace().any(|word| word == ACME),
        "{summary}"
    );
    run.type_line("yes");
}

fn assert_accepted(
    cloud: &cloud::Cloud,
    ada: &Person,
    space: SpaceId,
    link: &InviteLink,
    newcomer: &Person,
) {
    let (status, body) = cloud.get_as(&format!("/spaces/{space}/acceptances"), &ada.token);
    assert_eq!(status, 200, "{body}");
    let waiting = body["acceptances"].as_array().expect("waiting acceptances");
    assert_eq!(waiting.len(), 1, "{body}");
    let bytes = cloud::unb64(waiting[0]["acceptance"].as_str().expect("an acceptance"));
    let entry = Acceptance::from_cbor(&bytes).expect("an acceptance that decodes");
    let body = entry.body();
    assert_eq!(body.invite, link.invite, "{body:?}");
    assert_eq!(body.account, newcomer.account, "{body:?}");
    assert_eq!(body.master, newcomer.master, "{body:?}");
    assert_eq!(body.handle, newcomer.handle, "{body:?}");
    check_acceptance(&entry, &link.secret.invite_key().public_key())
        .expect("the acceptance is signed by the invite key");
}

#[test]
fn an_unlisted_person_signs_up_from_the_terminal_and_accepts() {
    unlisted_person_signs_up_and_accepts(Providers::GitHub);
}

#[test]
fn with_corporate_enabled_an_unlisted_github_user_can_still_sign_up_with_an_invite() {
    unlisted_person_signs_up_and_accepts(Providers::All);
}

fn unlisted_person_signs_up_and_accepts(providers: Providers) {
    let (world, _front) = fixtures::fronted_for(providers, |settings| {
        settings.sign_up = SignUp::Closed(Listed {
            github_logins: vec![ADA.to_owned()],
            ..Listed::default()
        });
    });
    let cloud = &world.cloud;
    let ada = world.sign_up(7_305_410_001, ADA, "ada-inviting-laptop");
    let space = create(cloud, &ada, ACME).body().space;
    let link = make_invite(&ada);
    let bob = cloud.user(7_305_410_002, "bob-unlisted-invitee");
    let fresh = Machine::new().with_host_name(BOB_HOST);
    let (grants, tokens) = (cloud.grants().len(), cloud.tokens().len());

    let mut run = open_invite(&fresh, &link);
    let grant = grants::shown_grant(cloud, &mut run, grants);
    // The complete verification URL carries the user code into the GitHub sign-in.
    let complete = format!("{}?user_code={}", grant.verification_uri, grant.user_code);
    let page = world.browser.page();
    let stand_in = page.goto(&cloud.github.sign_in_url(bob.id));
    assert_eq!(stand_in.status, Some(200), "{stand_in:?}");
    let device = page.goto_settled(&complete);
    assert_eq!(device.status, Some(200), "{device:?}");
    let signed_in = page.click_settled(&format!("{SIGN_IN_GITHUB} >> nth=0"));
    assert_eq!(signed_in.status, Some(200), "{}", page.text());
    if page.count(APPROVE) == 0 {
        let submitted = page.click(SUBMIT);
        assert_eq!(submitted.status, Some(200), "{}", page.text());
    }
    let approved = page.click(APPROVE);
    assert_eq!(approved.status, Some(200), "{}", page.text());

    let recovery = at_terminal::shown_key(&mut run);
    run.wait_to_show("Press Enter");
    run.enter();
    confirm_invite(&mut run);
    let finished = run.finish();
    let shown = finished.text.replace(link.shown().as_str(), "");
    assert!(finished.success, "{shown}");
    assert_eq!(at_terminal::keys_in(&shown), [recovery]);
    assert_eq!(cloud.grants().len(), grants + 1, "one device authorization");
    assert_eq!(cloud.tokens().len(), tokens + 1, "one device sign-in");

    let bob = world.person(bob, fresh, BOB_HOST);
    assert_eq!(cloud.login_of(&bob.token), bob.user.login);
    assert_accepted(cloud, &ada, space, &link, &bob);
    assert_secret_nowhere(
        cloud,
        &bob.machine,
        link.secret.to_bytes().as_ref(),
        link.secret.invite_key().to_bytes().as_ref(),
    );
}

#[test]
fn an_already_signed_in_device_accepts_without_signing_in_again() {
    let (world, _front) = fixtures::fronted_with(|settings| {
        settings.sign_up = SignUp::Closed(Listed {
            github_logins: vec![ADA.to_owned(), CAROL.to_owned()],
            ..Listed::default()
        });
    });
    let cloud = &world.cloud;
    let ada = world.sign_up(7_305_410_011, ADA, "ada-inviting-laptop");
    let carol = world.sign_up(7_305_410_012, CAROL, "carol-laptop");
    let space = create(cloud, &ada, ACME).body().space;
    let link = make_invite(&ada);
    let since = cloud.relay.mark();

    let mut run = open_invite(&carol.machine, &link);
    confirm_invite(&mut run);
    let finished = run.finish();
    assert!(
        finished.success,
        "{}",
        finished.text.replace(link.shown().as_str(), "")
    );
    assert_eq!(cloud.count_sent(since, "POST /auth/device "), 0);
    assert_eq!(cloud.count_sent(since, "PUT /auth/device/invite "), 0);
    assert_eq!(cloud.count_sent(since, "POST /auth/token "), 0);
    assert_accepted(cloud, &ada, space, &link, &carol);
}

#[test]
fn a_fresh_device_discovers_the_public_api_from_the_invites_web_address() {
    let public_front = front::Front::bind();
    let public = format!("http://localhost:{}", public_front.port());
    let (mut world, _web_front) = fixtures::fronted_with(|settings| {
        settings.sign_up = SignUp::Closed(Listed {
            github_logins: vec![ADA.to_owned()],
            ..Listed::default()
        });
        let sign_in = settings.sign_in.as_mut().expect("sign-in settings");
        sign_in.web_url = Some(sign_in.public_url.clone());
        sign_in.public_url = public.parse().expect("a public URL");
    });
    public_front.forward_to(world.cloud.relay.address());
    let web = std::mem::replace(&mut world.cloud.public, public.clone());
    world.cloud.url.clone_from(&public);
    let cloud = &world.cloud;
    let ada = world.sign_up(7_305_410_021, ADA, "ada-with-separate-origins");
    let space = create(cloud, &ada, ACME).body().space;
    let link = make_invite(&ada);
    assert_eq!(link.web, web);
    let bob = cloud.user(7_305_410_022, "bob-on-a-separate-origin");
    let fresh = Machine::new().with_host_name(BOB_HOST);
    let since = cloud.relay.mark();
    let grants = cloud.grants().len();

    let mut run = open_invite(&fresh, &link);
    let mut grant = grants::shown_grant(cloud, &mut run, grants);
    assert_eq!(grant.verification_uri, format!("{web}/device"));
    grant.verification_uri = format!("{}?user_code={}", grant.verification_uri, grant.user_code);
    cloud.decide(&world.browser.page(), &bob, &grant, APPROVE);
    at_terminal::shown_key(&mut run);
    run.wait_to_show("Press Enter");
    run.enter();
    confirm_invite(&mut run);
    let finished = run.finish();
    assert!(
        finished.success,
        "{}",
        finished.text.replace(link.shown().as_str(), "")
    );

    let sent = cloud.relay.sent_since(since);
    for (request, origin) in [
        ("get /server ", &web),
        ("put /auth/device/invite ", &public),
    ] {
        let host = origin.strip_prefix("http://").expect("an HTTP origin");
        assert!(
            sent.iter().any(|sent| {
                sent.head.starts_with(request)
                    && sent
                        .head
                        .lines()
                        .any(|header| header.strip_prefix("host: ") == Some(host))
            }),
            "expected {request} at {origin}"
        );
    }
    let bob = world.person(bob, fresh, BOB_HOST);
    assert_accepted(cloud, &ada, space, &link, &bob);
}

#[test]
fn after_browser_denial_a_fresh_device_can_link_and_accept() {
    let (world, _front) = fixtures::fronted_with(|_| {});
    let cloud = &world.cloud;
    let ada = world.sign_up(7_305_410_031, ADA, "ada-inviting-laptop");
    let carol = world.sign_up(7_305_410_032, CAROL, "carol-workstation");
    let space = create(cloud, &ada, ACME).body().space;
    let link = make_invite(&ada);
    let (grants, tokens) = (cloud.grants().len(), cloud.tokens().len());
    let host = "carol-new-laptop";
    let fresh = Machine::new().with_host_name(host);

    let mut run = open_invite(&fresh, &link);
    let shown = run.wait_for("a link code", |shown| !linking::codes_in(shown).is_empty());
    let code = linking::codes_in(&shown).pop().expect("a link code");
    let grant = grants::shown_grant(cloud, &mut run, grants);
    cloud.decide(&world.browser.page(), &carol.user, &grant, DENY);
    run.wait_to_show("The code still works.");
    let linked = linking::link_with(&carol, &code, host, linking::YES);
    assert!(linked.success, "{}", linked.text);
    let device = cloud.new_device(&carol.token, &[carol.device]);
    let cert = cloud.cert(&carol.token, device).expect("a certificate");
    assert_eq!(cert.body().issuer, Some(carol.device));

    confirm_invite(&mut run);
    let accepted = run.finish();
    assert!(
        accepted.success,
        "{}",
        accepted.text.replace(link.shown().as_str(), "")
    );
    assert_eq!(
        cloud.tokens().len(),
        tokens,
        "the browser gave no device session"
    );
    assert_accepted(cloud, &ada, space, &link, &carol);
}
