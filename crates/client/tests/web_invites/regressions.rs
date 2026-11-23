use serde_json::Value;
use skald_crypto::InviteLink;
use skald_server::{Listed, SignUp};

use crate::browser::{Answer, Browser, Page};
use crate::cloud::Cloud;
use crate::spa::{Spa, answers_without_secrets, wait_until};
use crate::world::{HOST, World};
use crate::{
    ACME, DEVICE, Harness, PHONE, create, create_invite, create_with_member, keys, main_text,
    open_access, open_members, sign_up_on,
};

/// Checks the recorded API answers since `from` and gives the index for the next screen.
#[track_caller]
fn check_invite_answers(
    page: &Page,
    url: &str,
    link: &InviteLink,
    from: usize,
    status: u16,
) -> usize {
    let is_listing = |answer: &Answer| answer.method == "GET" && answer.url == url;
    wait_until(page, "the browser's answer to GET invites", |page| {
        page.answers().iter().skip(from).any(is_listing)
    });
    let answers = answers_without_secrets(page);
    let secret = link.secret.to_bytes();
    let key = link.secret.invite_key().public_key();
    for answer in &answers[from..] {
        let body = answer.body.as_bytes();
        assert!(
            !keys::holds(body, secret.as_ref()) && !keys::holds(body, &key),
            "a browser API answer carried the invite secret or key"
        );
        if is_listing(answer) {
            assert_eq!(answer.status, status, "GET {url}");
            let body: Value =
                serde_json::from_str(&answer.body).expect("a JSON answer to GET invites");
            if status == 200 {
                assert_metadata_only(&body, link);
            } else {
                assert!(
                    body.get("invites").is_none(),
                    "a member received an invite list"
                );
            }
        }
    }
    answers.len()
}

#[track_caller]
fn assert_metadata_only(body: &Value, link: &InviteLink) {
    const FIELDS: [&str; 7] = [
        "invite",
        "role",
        "username",
        "device_name",
        "created_at",
        "expires_at",
        "acceptance_waiting",
    ];
    assert_eq!(body.as_object().map(|body| body.len()), Some(1));
    let invites = body["invites"].as_array().expect("an invites list");
    assert_eq!(invites.len(), 1);
    let listed = invites[0].as_object().expect("invite metadata");
    assert!(
        listed.len() == FIELDS.len() && FIELDS.iter().all(|field| listed.contains_key(*field)),
        "the list must contain only the open invite's metadata"
    );
    let invite = link.invite.to_string();
    assert!(
        listed["invite"].as_str() == Some(invite.as_str()),
        "the browser did not receive the issued invite's metadata"
    );
}

#[test]
fn members_and_access_receive_only_invite_metadata_for_an_admin() {
    let Harness { world, spa } = Harness::start();
    let [(admin, ada), (member, bob)] = [
        (3_819_813_001, "ada-invite-answers", DEVICE),
        (3_819_813_002, "bob-invite-answers", "bob-answers-laptop"),
    ]
    .map(|(id, login, device)| {
        let page = world.browser.page();
        let person = sign_up_on(&world, &page, (id, login), device);
        (page, person)
    });
    let roster = create_with_member(&world.cloud, &ada, ACME, &bob);
    let url = spa.at(&format!("/spaces/{}/invites", roster.body().space));
    let link = InviteLink::parse(&create_invite(&ada, &spa)).expect("an invite link");

    for (page, status) in [(&admin, 200), (&member, 403)] {
        page.record_answers();
        open_members(page, &spa, false);
        let answered = check_invite_answers(page, &url, &link, 0, status);
        open_access(page);
        check_invite_answers(page, &url, &link, answered, status);
    }
}

#[test]
fn an_invite_page_explains_account_creation_when_sign_up_is_closed() {
    const OWNER: &str = "ada-closed-invite-page";
    let browser = Browser::launch();
    let spa = Spa::start(&browser, HOST);
    let cloud = Cloud::fronted_with(HOST, spa.port, 120, 1, |settings| {
        settings.sign_up = SignUp::Closed(Listed {
            github_logins: vec![OWNER.to_owned()],
            ..Listed::default()
        });
    });
    spa.serve(&browser, &cloud.relay.url);
    let world = World { cloud, browser };
    let owner = world.sign_up(3_819_814_001, OWNER, DEVICE);
    create(&world.cloud, &owner, ACME);
    let link = create_invite(&owner, &spa);
    let page = world.browser.page_of_width(PHONE);
    assert!(page.cookies().is_empty());
    assert_eq!(page.goto(&link).status, Some(200));
    wait_until(&page, "the invite page", |page| page.count("main h1") == 1);
    let instruction = main_text(&page).to_lowercase();
    for phrase in ["sign-up is closed", "create your account", "this link"] {
        assert!(instruction.contains(phrase), "missing {phrase:?}");
    }
}
