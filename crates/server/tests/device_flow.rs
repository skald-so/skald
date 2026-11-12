//! The device flow (RFC 8628) on the server: a machine without a browser asks for a code and polls,
//! and its person approves or denies the code on the verification page in headless Chrome, signed
//! in with GitHub through the stand-in. Every response to the machine is checked against the
//! OpenAPI spec in `api/openapi.json`.

mod common;

use std::collections::BTreeSet;
use std::sync::Barrier;
use std::thread;

use common::browser::Browser;
use common::device_flow::{
    APPROVE, CLIENT_ID, CODE_FIELD, DENY, DEVICE_AUTHORIZATION, DeviceFlow, GRANT_TYPE, Grant,
    INTERVAL, LIFETIME, Poll, SUBMIT, TOKEN, answer, assert_ok, assert_refused_page, json, pause,
    text,
};
use common::sign_in::SignIn;
use common::{get_url, post_form, stored_forms};

/// The host of the server's public address in the browser tests.
const HOST: &str = "skald.test";
/// The most wrong codes the page takes before it refuses codes for a while.
const WRONG_CODE_LIMIT: usize = 10;

#[test]
fn device_authorization_gives_codes_and_the_verification_page() {
    let flow = DeviceFlow::start_with(&[LIFETIME, "120", INTERVAL, "3"]);
    let first = flow.authorize();
    let second = flow.authorize();

    for grant in [&first, &second] {
        assert_eq!(grant.expires_in, 120, "{grant:?}");
        assert_eq!(grant.interval, 3, "{grant:?}");
        let page = format!("{}/", flow.stand.public);
        assert!(grant.verification_uri.starts_with(&page), "{grant:?}");
        assert!(
            grant
                .verification_uri_complete
                .starts_with(&grant.verification_uri),
            "{grant:?}"
        );
        assert!(
            grant.verification_uri_complete.contains(&grant.user_code),
            "{grant:?}"
        );
        assert!(!grant.verification_uri.contains(&grant.user_code));
        let typed: Vec<char> = grant
            .user_code
            .chars()
            .filter(|c| !matches!(c, '-' | ' '))
            .collect();
        assert!(
            (6..=9).contains(&typed.len()) && typed.iter().all(char::is_ascii_alphanumeric),
            "short to type: {grant:?}"
        );
        assert!(grant.device_code.len() >= 22, "{grant:?}");
    }
    assert_ne!(first.device_code, second.device_code);
    assert_ne!(first.user_code, second.user_code);
    assert_ne!(first.device_code, first.user_code);
}

#[test]
fn codes_expire_within_minutes_by_default() {
    let flow = DeviceFlow::start_with(&[]);
    let grant = flow.authorize();
    assert!((60..=900).contains(&grant.expires_in), "{grant:?}");
    assert!(grant.interval >= 1, "{grant:?}");
}

#[test]
fn approved_code_gives_one_token_that_acts_as_the_approving_account() {
    let browser = Browser::launch();
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_664_501, "ada-device-standin");
    let page = browser.page();
    let account = flow.signed_in(&page, &ada);
    let grant = flow.authorize();

    // The person types the code at the address without it.
    let visit = flow.open(&page, &grant.verification_uri);
    assert_eq!(visit.status, Some(200), "{visit:?}");
    assert_eq!(
        page.values(CODE_FIELD),
        vec![String::new()],
        "an empty field"
    );
    page.fill(CODE_FIELD, &grant.user_code);
    if page.count(APPROVE) == 0 {
        assert_eq!(page.click(SUBMIT).status, Some(200));
    }
    // It tells them it signs a machine in to their account.
    let told = page.text();
    assert!(told.contains(&ada.login), "{told}");
    assert!(
        told.to_lowercase().contains("machine") || told.to_lowercase().contains("device"),
        "{told}"
    );
    assert_ok(&flow.decide(&page, APPROVE));

    let token = flow.token(&grant);
    assert_eq!(flow.account_of(&token), account);
    // The browser's own session is unchanged.
    assert_eq!(flow.stand.account(&page), account);

    // Once given, the token is not given again.
    pause(grant.interval);
    assert!(matches!(flow.poll(&grant.device_code), Poll::Error(_)));
    assert_eq!(flow.account_of(&token), account);
}

#[test]
fn code_in_the_address_counts_only_after_the_persons_approval() {
    let browser = Browser::launch();
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_664_502, "ada-complete-standin");
    let page = browser.page();
    let account = flow.signed_in(&page, &ada);
    let grant = flow.authorize();

    // The code is filled in; opening the address approves nothing.
    flow.open(&page, &grant.verification_uri_complete);
    let shown = page.values(CODE_FIELD).contains(&grant.user_code)
        || page.text().contains(&grant.user_code);
    assert!(shown, "the page shows the code: {}", page.text());
    flow.assert_pending(&grant);
    flow.reach_decision(&page, &grant);
    flow.assert_pending(&grant);

    assert_ok(&flow.decide(&page, APPROVE));
    assert_eq!(flow.account_of(&flow.token(&grant)), account);
}

#[test]
fn person_without_a_session_signs_in_with_github_on_the_page() {
    let browser = Browser::launch();
    let flow = DeviceFlow::start();
    let bob = flow.stand.user(3_120_664_503, "bob-newcomer-standin");
    let grant = flow.authorize();

    // Signed in to GitHub only: the page signs them in with GitHub, at once or by its link or
    // button, creating their account.
    let page = browser.page();
    flow.stand.assert_signed_out(&page);
    flow.stand.github_session(&page, &bob);
    let mut visit = flow.open(&page, &grant.verification_uri_complete);
    let sign_in = "a:has-text('sign in'), button:has-text('sign in')";
    if page.count(sign_in) > 0 {
        visit = page.click(&format!("{sign_in} >> nth=0"));
    }
    assert!(
        visit
            .navigations
            .iter()
            .any(|url| url.contains("/auth/github")),
        "the page signs in with GitHub: {visit:?}"
    );
    let account = flow.stand.account(&page);
    assert_eq!(account.github_login, bob.login);
    flow.assert_pending(&grant);

    assert_eq!(flow.account_of(&flow.approve(&page, &grant)), account);
}

#[test]
fn two_polls_at_once_give_one_token() {
    let browser = Browser::launch();
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_664_504, "ada-twopolls-standin");
    let page = browser.page();
    let account = flow.signed_in(&page, &ada);
    let grant = flow.authorize();
    assert_ok(&flow.enter_and_decide(&page, &grant, APPROVE));

    pause(grant.interval);
    let barrier = Barrier::new(2);
    let answers: Vec<Poll> = thread::scope(|scope| {
        let polls: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    flow.poll(&grant.device_code)
                })
            })
            .collect();
        polls
            .into_iter()
            .map(|poll| poll.join().expect("a poll"))
            .collect()
    });
    let tokens: Vec<&String> = answers
        .iter()
        .filter_map(|answer| match answer {
            Poll::Token(token) => Some(token),
            Poll::Error(_) => None,
        })
        .collect();
    assert_eq!(tokens.len(), 1, "{answers:?}");
    assert_eq!(flow.account_of(tokens[0]), account);

    pause(grant.interval + 5);
    assert!(matches!(flow.poll(&grant.device_code), Poll::Error(_)));
}

#[test]
fn two_accounts_approving_at_once_bind_the_code_to_one() {
    let browser = Browser::launch();
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_664_505, "ada-race-standin");
    let bob = flow.stand.user(3_120_664_506, "bob-race-standin");
    let ada_page = browser.page();
    let bob_page = browser.page();
    let ada_account = flow.signed_in(&ada_page, &ada);
    let bob_account = flow.signed_in(&bob_page, &bob);
    let grant = flow.authorize();

    flow.reach_decision(&ada_page, &grant);
    flow.reach_decision(&bob_page, &grant);
    let visits = browser.click_all(&[(&ada_page, APPROVE), (&bob_page, APPROVE)]);
    let ok: Vec<bool> = visits
        .iter()
        .map(|visit| {
            visit
                .status
                .is_some_and(|status| (200..300).contains(&status))
        })
        .collect();
    let winner = match ok.as_slice() {
        [true, false] => {
            assert_refused_page(&visits[1]);
            ada_account
        }
        [false, true] => {
            assert_refused_page(&visits[0]);
            bob_account
        }
        _ => panic!("exactly one approval counts: {visits:?}"),
    };
    assert_eq!(flow.account_of(&flow.token(&grant)), winner);
}

#[test]
fn polling_answers_pending_and_slow_down_when_too_fast() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "2"]);
    let grant = flow.authorize();
    assert_eq!(grant.interval, 2);

    flow.assert_pending(&grant);
    let too_fast = flow.poll(&grant.device_code);
    assert_eq!(too_fast, Poll::Error("slow_down".to_owned()));

    // At the interval plus the 5 s a client adds after `slow_down`, it is pending again.
    pause(grant.interval + 5);
    assert_eq!(
        flow.poll(&grant.device_code),
        Poll::Error("authorization_pending".to_owned())
    );
}

#[test]
fn denied_code_gives_no_token() {
    let browser = Browser::launch();
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_664_507, "ada-deny-standin");
    let page = browser.page();
    flow.signed_in(&page, &ada);
    let grant = flow.authorize();

    flow.reach_decision(&page, &grant);
    assert_ok(&flow.decide(&page, DENY));
    assert_eq!(flow.wait(&grant), Poll::Error("access_denied".to_owned()));
    pause(grant.interval);
    assert!(matches!(flow.poll(&grant.device_code), Poll::Error(_)));

    // A denied code cannot be approved afterwards.
    assert_refused_page(&flow.enter_and_decide(&page, &grant, APPROVE));
    pause(grant.interval);
    assert!(matches!(flow.poll(&grant.device_code), Poll::Error(_)));
}

#[test]
fn expired_code_gives_no_token_and_is_refused_on_the_page() {
    let browser = Browser::launch();
    let flow = DeviceFlow::start_with(&[LIFETIME, "8", INTERVAL, "1"]);
    let ada = flow.stand.user(3_120_664_508, "ada-expired-standin");
    let page = browser.page();
    flow.signed_in(&page, &ada);

    // Within its lifetime a code works.
    let quick = flow.authorize();
    assert_eq!(quick.expires_in, 8);
    flow.enter_and_approve(&page, &quick);

    let slow = flow.authorize();
    pause(slow.expires_in + 1);
    assert_refused_page(&flow.enter_and_decide(&page, &slow, APPROVE));
    assert_eq!(
        flow.poll(&slow.device_code),
        Poll::Error("expired_token".to_owned())
    );

    // A code approved in time but polled too late gives no token either.
    let late = flow.authorize();
    assert_ok(&flow.enter_and_decide(&page, &late, APPROVE));
    pause(late.expires_in + 1);
    assert_eq!(
        flow.poll(&late.device_code),
        Poll::Error("expired_token".to_owned())
    );
}

#[test]
fn used_user_code_is_refused_on_the_page() {
    let browser = Browser::launch();
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_664_509, "ada-used-standin");
    let bob = flow.stand.user(3_120_664_510, "bob-used-standin");
    let ada_page = browser.page();
    let bob_page = browser.page();
    let ada_account = flow.signed_in(&ada_page, &ada);
    flow.signed_in(&bob_page, &bob);
    let grant = flow.authorize();

    assert_ok(&flow.enter_and_decide(&ada_page, &grant, APPROVE));
    // Approved, not yet polled: the code works no more, for anyone.
    assert_refused_page(&flow.enter_and_decide(&bob_page, &grant, APPROVE));
    assert_refused_page(&flow.enter_and_decide(&ada_page, &grant, APPROVE));
    assert_refused_page(&flow.enter_and_decide(&bob_page, &grant, DENY));
    assert_eq!(flow.account_of(&flow.token(&grant)), ada_account);

    // Polled: still refused.
    assert_refused_page(&flow.enter_and_decide(&bob_page, &grant, APPROVE));
}

#[test]
fn reused_or_unknown_device_code_gives_no_token() {
    let browser = Browser::launch();
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_664_511, "ada-reuse-standin");
    let page = browser.page();
    flow.signed_in(&page, &ada);
    let grant = flow.authorize();
    flow.enter_and_approve(&page, &grant);

    for _ in 0..2 {
        pause(grant.interval + 5);
        assert!(matches!(flow.poll(&grant.device_code), Poll::Error(_)));
    }
    let unknown = format!("{}x", grant.device_code);
    for device_code in [unknown.as_str(), "", grant.user_code.as_str()] {
        assert!(
            matches!(flow.poll(device_code), Poll::Error(_)),
            "{device_code:?}"
        );
    }
}

#[test]
fn wrong_codes_past_the_limit_lock_the_page() {
    let browser = Browser::launch();
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_664_512, "ada-guess-standin");
    let page = browser.page();
    flow.signed_in(&page, &ada);
    let grants: Vec<Grant> = (0..4).map(|_| flow.authorize()).collect();

    // Before any wrong code, the right one works.
    flow.enter_and_approve(&page, &grants[0]);

    // Wrong codes in the codes' own alphabet and form.
    let issued: BTreeSet<&str> = grants.iter().map(|g| g.user_code.as_str()).collect();
    let alphabet: Vec<char> = issued
        .iter()
        .flat_map(|code| code.chars())
        .filter(char::is_ascii_alphanumeric)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut wrong = Vec::new();
    for shift in 1.. {
        if wrong.len() == WRONG_CODE_LIMIT {
            break;
        }
        let guess: String = grants[1]
            .user_code
            .chars()
            .enumerate()
            .map(|(at, c)| match alphabet.iter().position(|&a| a == c) {
                Some(index) => alphabet[(index + shift * (at + 1)) % alphabet.len()],
                None => c,
            })
            .collect();
        if !issued.contains(guess.as_str()) && !wrong.contains(&guess) {
            wrong.push(guess);
        }
    }
    for guess in &wrong {
        let grant = Grant {
            user_code: guess.clone(),
            ..grants[1].clone()
        };
        assert_refused_page(&flow.enter_and_decide(&page, &grant, APPROVE));
    }

    // Past the limit, the right code is refused too.
    assert_refused_page(&flow.enter_and_decide(&page, &grants[1], APPROVE));
    assert_refused_page(&flow.open_and_decide(&page, &grants[2], APPROVE));
    for grant in &grants[1..] {
        flow.assert_pending(grant);
    }
}

#[test]
fn approval_from_another_site_counts_for_nothing() {
    let browser = Browser::launch();
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_664_513, "ada-csrf-standin");
    let page = browser.page();
    let account = flow.signed_in(&page, &ada);
    let other = format!("http://other.test:{}/", browser.echo(false));
    // The machine's owner, not the person, started this flow and lures the person.
    let grant = flow.authorize();

    // A link from another site to the address with the code.
    page.goto(&other);
    page.navigate_by(&format!(
        r#"setTimeout(() => {{ location.href = {}; }}, 0)"#,
        serde_json::to_string(&grant.verification_uri_complete).expect("JSON")
    ));
    flow.assert_pending(&grant);

    // The page's own approval, sent from another site: as a form, and as a link.
    flow.reach_decision(&page, &grant);
    let approval = page.submission(APPROVE);
    let fields = serde_json::to_string(&approval.fields).expect("JSON");
    page.goto(&other);
    page.navigate_by(&format!(
        r#"setTimeout(() => {{
            const form = document.createElement("form");
            form.method = {method};
            form.action = {action};
            for (const [name, value] of {fields}) {{
                const field = document.createElement("input");
                field.type = "hidden";
                field.name = name;
                field.value = value;
                form.append(field);
            }}
            document.body.append(form);
            form.submit();
        }}, 0)"#,
        method = serde_json::to_string(&approval.method).expect("JSON"),
        action = serde_json::to_string(&approval.action).expect("JSON"),
    ));
    flow.assert_pending(&grant);
    let query = serde_urlencoded::to_string(&approval.fields).expect("a query");
    let link = format!(
        "{}?{query}",
        approval.action.split('?').next().unwrap_or_default()
    );
    page.goto(&other);
    page.navigate_by(&format!(
        r#"setTimeout(() => {{ location.href = {}; }}, 0)"#,
        serde_json::to_string(&link).expect("JSON")
    ));
    flow.assert_pending(&grant);

    // The person's own approval still works.
    assert_eq!(flow.account_of(&flow.approve(&page, &grant)), account);
}

#[test]
fn request_without_a_token_or_with_an_unknown_one_is_refused() {
    let browser = Browser::launch();
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_664_514, "ada-bearer-standin");
    let page = browser.page();
    let account = flow.signed_in(&page, &ada);
    let grant = flow.authorize();
    let token = flow.enter_and_approve(&page, &grant);
    assert_eq!(flow.account_of(&token), account);

    let mut flipped = token.clone().into_bytes();
    let last = flipped.last_mut().expect("a token is not empty");
    *last = if *last == b'A' { b'B' } else { b'A' };
    let flipped = String::from_utf8(flipped).expect("ASCII");
    let refused = [
        None,
        Some("Bearer".to_owned()),
        Some("Bearer ".to_owned()),
        Some(format!("Bearer {flipped}")),
        Some(format!("Bearer {token}x")),
        Some(format!("Bearer {}", &token[..token.len() / 2])),
        Some("Bearer 0123456789abcdef0123456789abcdef0123456789a".to_owned()),
        Some(format!("Bearer {}", grant.device_code)),
        Some(format!("Bearer {}", grant.user_code)),
        Some(format!("Basic {token}")),
        Some(token.clone()),
    ];
    for authorization in &refused {
        flow.assert_refused(authorization.as_deref());
    }
}

#[test]
fn session_tokens_are_unguessable_and_stored_only_hashed() {
    let browser = Browser::launch();
    let flow = DeviceFlow::start();
    let mut tokens = Vec::new();
    for (id, login) in [
        (3_120_664_515, "ada-hash-standin"),
        (3_120_664_516, "bob-hash-standin"),
    ] {
        let user = flow.stand.user(id, login);
        let page = browser.page();
        let account = flow.signed_in(&page, &user);
        for _ in 0..2 {
            let token = flow.enter_and_approve(&page, &flow.authorize());
            assert_eq!(flow.account_of(&token), account);
            tokens.push(token);
        }
    }
    let distinct: BTreeSet<&String> = tokens.iter().collect();
    assert_eq!(distinct.len(), tokens.len(), "{tokens:?}");
    for token in &tokens {
        assert!(token.len() >= 22, "{token:?}");
    }

    let rows = flow.stand.db.rows();
    for token in &tokens {
        for form in stored_forms(token) {
            let found: Vec<_> = rows.iter().filter(|row| row.contains(&form)).collect();
            assert!(found.is_empty(), "{token:?} as {form:?} in {found:?}");
        }
    }
}

#[test]
fn device_flow_requests_state_the_protocol_version() {
    let flow = DeviceFlow::start();
    let base = flow.stand.server.base().to_owned();
    let grant = flow.authorize();
    let poll = [
        ("grant_type", GRANT_TYPE),
        ("device_code", grant.device_code.as_str()),
        ("client_id", CLIENT_ID),
    ];
    let authorize = [("client_id", CLIENT_ID)];
    let too_new = (skald_proto::PROTOCOL_VERSION.get() + 1).to_string();

    for (path, fields) in [(DEVICE_AUTHORIZATION, &authorize[..]), (TOKEN, &poll[..])] {
        for (headers, problem) in [
            (vec![], "protocol-version-invalid"),
            (
                vec![(skald_proto::VERSION_HEADER, "two")],
                "protocol-version-invalid",
            ),
            (
                vec![(skald_proto::VERSION_HEADER, too_new.as_str())],
                "protocol-too-new",
            ),
        ] {
            let response = post_form(&format!("{base}{path}"), &headers, fields);
            assert_eq!(response.status, 400, "POST {path}: {}", text(&response));
            assert_eq!(
                json(&response)["type"],
                format!("https://skald.so/problems/{problem}"),
                "POST {path}"
            );
            assert!(
                response.header("skald-protocol-latest").is_some(),
                "POST {path}"
            );
            flow.conforms("post", path, &response);
        }
    }
    // The refused poll did not count: the code is still pending.
    flow.assert_pending(&grant);

    // The verification page, as a browser opens it, states no version.
    let path = page_path(&flow.stand, &grant.verification_uri);
    for url in [&grant.verification_uri, &grant.verification_uri_complete] {
        let local = url.replacen(&flow.stand.public, &base, 1);
        let response = get_url(&local, &[]);
        assert!(response.status < 400, "GET {url}: {}", text(&response));
        flow.conforms("get", &path, &response);
    }
}

#[test]
fn device_flow_responses_conform_to_spec() {
    let browser = Browser::launch();
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_664_517, "ada-spec-standin");
    let page = browser.page();
    flow.signed_in(&page, &ada);
    let base = flow.stand.server.base().to_owned();

    // Every answer to the machine is checked against the spec as it comes.
    let approved = flow.authorize();
    let denied = flow.authorize();
    flow.assert_pending(&approved);
    let too_fast = flow.poll(&approved.device_code);
    assert_eq!(too_fast, Poll::Error("slow_down".to_owned()));
    assert_ok(&flow.enter_and_decide(&page, &approved, APPROVE));
    flow.reach_decision(&page, &denied);
    assert_ok(&flow.decide(&page, DENY));
    let token = flow.token(&approved);
    assert_eq!(flow.wait(&denied), Poll::Error("access_denied".to_owned()));
    pause(approved.interval + 5);
    let reused = flow.poll_response(&approved.device_code);
    assert!(matches!(answer(&reused), Poll::Error(_)));
    flow.account_of(&token);
    flow.assert_refused(None);
    flow.assert_refused(Some("Bearer unknown"));

    // The verification page, with and without the code, signed in or not.
    let path = page_path(&flow.stand, &approved.verification_uri);
    let cookies: Vec<String> = page
        .cookies_of(HOST)
        .iter()
        .map(|cookie| cookie.pair())
        .collect();
    let cookie = cookies.join("; ");
    let fresh = flow.authorize();
    for url in [&fresh.verification_uri, &fresh.verification_uri_complete] {
        let local = url.replacen(&flow.stand.public, &base, 1);
        for headers in [vec![], vec![("cookie", cookie.as_str())]] {
            let response = get_url(&local, &headers);
            flow.conforms("get", &path, &response);
        }
    }
}

/// The path of the verification page at `url`, under `stand`'s public address.
fn page_path(stand: &SignIn, url: &str) -> String {
    let path = url
        .strip_prefix(&stand.public)
        .unwrap_or_else(|| panic!("{url} is under {}", stand.public));
    path.split('?').next().unwrap_or_default().to_owned()
}
