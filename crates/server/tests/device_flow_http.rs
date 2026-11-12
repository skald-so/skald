//! The device flow as plain HTTP shows it: malformed requests of a machine, a server without GitHub
//! sign-in or with its database down, the verification page's refusals, and which session a token
//! opens; the limit of wrong user codes. Every response is checked against the OpenAPI spec in
//! `api/openapi.json`.

mod common;

use std::sync::Barrier;
use std::thread;

use common::device_flow::{
    CLIENT_ID, DEVICE_AUTHORIZATION, DeviceFlow, GRANT_TYPE, TOKEN, json, text,
};
use common::sign_in::{SESSION, START};
use common::spec::{operation_violations, spec};
use common::{Response, Server, TestDb, get_url, post_form};

/// The verification page.
const PAGE: &str = "/device";
/// The most wrong user codes an account may type within the window.
const WRONG_CODE_LIMIT: usize = 10;
/// A user code in the codes' form that the server never issued in these tests.
const WRONG_CODE: &str = "ZZZZ-ZZZZ";

/// `POST <path>` of `fields` to `flow`'s server, stating the protocol version, checked against the
/// spec.
fn post_stated(flow: &DeviceFlow, path: &str, fields: &[(&str, &str)]) -> Response {
    let version = skald_proto::PROTOCOL_VERSION.to_string();
    let url = format!("{}{path}", flow.stand.server.base());
    let response = post_form(&url, &[(skald_proto::VERSION_HEADER, &version)], fields);
    flow.conforms("post", path, &response);
    response
}

/// Checks that `response` is an OAuth error `error`, not to be cached.
fn assert_oauth_error(response: &Response, error: &str) {
    assert_eq!(response.status, 400, "{}", text(response));
    assert_eq!(json(response)["error"], error, "{}", text(response));
    assert_eq!(response.header("cache-control"), Some("no-store"));
}

/// `POST /device` of `fields` to `flow`'s server with the request headers `headers`, checked
/// against the spec.
fn decide(flow: &DeviceFlow, headers: &[(&str, &str)], fields: &[(&str, &str)]) -> Response {
    let url = format!("{}{PAGE}", flow.stand.server.base());
    let response = post_form(&url, headers, fields);
    flow.conforms("post", PAGE, &response);
    response
}

#[test]
fn malformed_machine_requests_are_refused() {
    let flow = DeviceFlow::start();
    let device_code = flow.authorize().device_code;

    for (fields, error) in [
        (vec![], "invalid_request"),
        (vec![("client_id", "other")], "invalid_client"),
        (
            vec![("client_id", CLIENT_ID), ("client_id", CLIENT_ID)],
            "invalid_request",
        ),
    ] {
        let response = post_stated(&flow, DEVICE_AUTHORIZATION, &fields);
        assert_oauth_error(&response, error);
    }

    let code = device_code.as_str();
    for (fields, error) in [
        (
            vec![("grant_type", GRANT_TYPE), ("device_code", code)],
            "invalid_request",
        ),
        (
            vec![
                ("grant_type", GRANT_TYPE),
                ("device_code", code),
                ("client_id", "other"),
            ],
            "invalid_client",
        ),
        (
            vec![("device_code", code), ("client_id", CLIENT_ID)],
            "invalid_request",
        ),
        (
            vec![
                ("grant_type", "authorization_code"),
                ("device_code", code),
                ("client_id", CLIENT_ID),
            ],
            "unsupported_grant_type",
        ),
        (
            vec![("grant_type", GRANT_TYPE), ("client_id", CLIENT_ID)],
            "invalid_request",
        ),
        (
            vec![
                ("grant_type", GRANT_TYPE),
                ("device_code", code),
                ("device_code", code),
                ("client_id", CLIENT_ID),
            ],
            "invalid_request",
        ),
    ] {
        let response = post_stated(&flow, TOKEN, &fields);
        assert_oauth_error(&response, error);
    }
    // None of them counted as a poll.
    flow.assert_pending(&flow.authorize());
}

#[test]
fn device_flow_is_off_without_github_settings() {
    let db = TestDb::create();
    let server = Server::start(&db);
    let spec = spec();
    let version = skald_proto::PROTOCOL_VERSION.to_string();
    let stated = [(skald_proto::VERSION_HEADER, version.as_str())];
    let fields = [("client_id", CLIENT_ID)];

    for (method, path, response) in [
        (
            "post",
            DEVICE_AUTHORIZATION,
            post_form(
                &format!("{}{DEVICE_AUTHORIZATION}", server.base()),
                &stated,
                &fields,
            ),
        ),
        (
            "post",
            TOKEN,
            post_form(&format!("{}{TOKEN}", server.base()), &stated, &fields),
        ),
        ("get", PAGE, server.get(PAGE)),
        (
            "post",
            PAGE,
            post_form(&format!("{}{PAGE}", server.base()), &[], &[]),
        ),
    ] {
        assert_eq!(response.status, 404, "{method} {path}: {}", text(&response));
        assert_eq!(
            json(&response)["type"],
            "https://skald.so/problems/github-sign-in-off"
        );
        let violations = operation_violations(&spec, method, path, &response);
        assert_eq!(violations, Vec::<String>::new());
    }
}

#[test]
fn verification_page_without_a_session_links_to_the_sign_in() {
    let flow = DeviceFlow::start();
    let grant = flow.authorize();
    let local =
        grant
            .verification_uri_complete
            .replacen(&flow.stand.public, flow.stand.server.base(), 1);
    let response = get_url(&local, &[]);
    assert_eq!(response.status, 200, "{}", text(&response));
    flow.conforms("get", PAGE, &response);
    let page = String::from_utf8_lossy(&response.body);
    let to = serde_urlencoded::to_string([(
        "return_to",
        format!("{PAGE}?user_code={}", grant.user_code),
    )])
    .expect("a query");
    assert!(page.contains(&format!("href=\"{START}?{to}\"")), "{page}");
    for header in ["content-security-policy", "cache-control"] {
        assert!(response.header(header).is_some(), "{header}");
    }
    let policy = response
        .header("content-security-policy")
        .unwrap_or_default();
    assert!(policy.contains("frame-ancestors 'none'"), "{policy}");
}

#[test]
fn decisions_from_elsewhere_or_without_a_session_are_refused() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_664_601, "ada-origin-standin");
    let cookie = flow.web_cookie(&ada);
    let grant = flow.authorize();
    let origin = flow.stand.public.clone();
    let approve = [
        ("user_code", grant.user_code.as_str()),
        ("decision", "approve"),
    ];

    for headers in [
        vec![("cookie", cookie.as_str())],
        vec![("cookie", cookie.as_str()), ("origin", "null")],
        vec![("cookie", cookie.as_str()), ("origin", "http://other.test")],
        vec![("origin", origin.as_str())],
        vec![
            ("origin", origin.as_str()),
            ("cookie", "skald_session=unknown"),
        ],
    ] {
        let response = decide(&flow, &headers, &approve);
        assert_eq!(response.status, 403, "{headers:?}: {}", text(&response));
    }
    let headers = [("cookie", cookie.as_str()), ("origin", origin.as_str())];
    for decision in [vec![], vec![("decision", "maybe")]] {
        let mut fields = vec![("user_code", grant.user_code.as_str())];
        fields.extend(decision);
        let response = decide(&flow, &headers, &fields);
        assert_eq!(response.status, 400, "{fields:?}: {}", text(&response));
    }
    flow.assert_pending(&grant);

    // The person's own decision, typed in lowercase without the hyphen, counts.
    let typed = grant.user_code.replace('-', "").to_lowercase();
    let response = decide(
        &flow,
        &headers,
        &[("user_code", &typed), ("decision", "approve")],
    );
    assert_eq!(response.status, 200, "{}", text(&response));
    assert!(text(&response).contains("Approved"), "{}", text(&response));
    let token = flow.token(&grant);
    assert_eq!(flow.account_of(&token).github_login, ada.login);

    let denied = flow.authorize();
    let response = decide(
        &flow,
        &headers,
        &[("user_code", &denied.user_code), ("decision", "deny")],
    );
    assert_eq!(response.status, 200, "{}", text(&response));
    assert!(text(&response).contains("Denied"), "{}", text(&response));
}

#[test]
fn web_and_device_sessions_open_only_their_own_kind() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_664_602, "ada-kinds-standin");
    let cookie = flow.web_cookie(&ada);
    let grant = flow.authorize();
    let headers = [
        ("cookie", cookie.as_str()),
        ("origin", flow.stand.public.as_str()),
    ];
    let approve = [
        ("user_code", grant.user_code.as_str()),
        ("decision", "approve"),
    ];
    assert_eq!(decide(&flow, &headers, &approve).status, 200);
    let token = flow.token(&grant);

    // A web session's token is no bearer token, and a device session's token is no cookie.
    let web_token = cookie.split_once('=').expect("name=value").1;
    flow.assert_refused(Some(&format!("Bearer {web_token}")));
    let version = skald_proto::PROTOCOL_VERSION.to_string();
    let as_cookie = format!("skald_session={token}");
    let response = get_url(
        &format!("{}{SESSION}", flow.stand.server.base()),
        &[
            (skald_proto::VERSION_HEADER, &version),
            ("cookie", &as_cookie),
        ],
    );
    assert_eq!(response.status, 401, "{}", text(&response));
    // The scheme's case does not count.
    let response = flow.session(Some(&format!("bearer {token}")));
    assert_eq!(response.status, 200, "{}", text(&response));
}

#[test]
fn database_failures_answer_500() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_664_603, "ada-down-standin");
    let cookie = flow.web_cookie(&ada);
    let grant = flow.authorize();
    let origin = flow.stand.public.clone();

    flow.stand.db.set_reachable(false);
    let authorized = post_stated(&flow, DEVICE_AUTHORIZATION, &[("client_id", CLIENT_ID)]);
    let polled = post_stated(
        &flow,
        TOKEN,
        &[
            ("grant_type", GRANT_TYPE),
            ("device_code", &grant.device_code),
            ("client_id", CLIENT_ID),
        ],
    );
    let session = flow.session(Some("Bearer unknown"));
    for response in [&authorized, &polled, &session] {
        assert_eq!(response.status, 500, "{}", text(response));
        assert_eq!(
            json(response)["type"],
            "https://skald.so/problems/server-error"
        );
    }
    let page = get_url(
        &format!("{}{PAGE}", flow.stand.server.base()),
        &[("cookie", &cookie)],
    );
    flow.conforms("get", PAGE, &page);
    let decided = decide(
        &flow,
        &[("cookie", &cookie), ("origin", &origin)],
        &[("user_code", &grant.user_code), ("decision", "approve")],
    );
    for response in [&page, &decided] {
        assert_eq!(response.status, 500, "{}", text(response));
        assert!(response.content_type.starts_with("text/html"));
    }

    // The failed poll did not count.
    flow.stand.db.set_reachable(true);
    flow.assert_pending(&grant);
}

/// The number of wrong user codes `flow`'s database holds.
fn wrong_codes(flow: &DeviceFlow) -> usize {
    let rows = flow.stand.db.rows();
    rows.iter()
        .filter(|row| row.starts_with("wrong_user_codes: "))
        .count()
}

/// The person with the request headers `headers` types `user_code` on the verification page and
/// approves it; gives the answer's status.
fn approve(flow: &DeviceFlow, headers: &[(&str, &str)], user_code: &str) -> u16 {
    decide(
        flow,
        headers,
        &[("user_code", user_code), ("decision", "approve")],
    )
    .status
}

#[test]
fn wrong_codes_hold_codes_back_only_within_their_window() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_664_604, "ada-window-standin");
    let cookie = flow.web_cookie(&ada);
    let headers = [
        ("cookie", cookie.as_str()),
        ("origin", flow.stand.public.as_str()),
    ];
    let grant = flow.authorize();

    for _ in 0..WRONG_CODE_LIMIT {
        assert_eq!(approve(&flow, &headers, WRONG_CODE), 400);
    }
    assert_eq!(approve(&flow, &headers, &grant.user_code), 429);

    // 14 of the window's 15 minutes later, all of them still count.
    let db = &flow.stand.db;
    db.execute("UPDATE wrong_user_codes SET typed_at = typed_at - interval '14 minutes'");
    assert_eq!(approve(&flow, &headers, &grant.user_code), 429);
    // Once the first of them is past the window, fewer than the limit remain.
    db.execute(
        "UPDATE wrong_user_codes SET typed_at = typed_at - interval '61 seconds'
         WHERE typed_at = (SELECT min(typed_at) FROM wrong_user_codes)",
    );
    assert_eq!(approve(&flow, &headers, &grant.user_code), 200);
    assert_eq!(flow.account_of(&flow.token(&grant)).github_login, ada.login);
}

#[test]
fn wrong_codes_at_once_count_at_most_one_past_the_limit() {
    /// How many wrong codes go out at once, and how many times.
    const AT_ONCE: usize = 30;
    const ROUNDS: usize = 10;
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_664_605, "ada-burst-standin");
    let cookie = flow.web_cookie(&ada);
    let headers = [
        ("cookie", cookie.as_str()),
        ("origin", flow.stand.public.as_str()),
    ];
    let burst = |send: &(dyn Fn() -> u16 + Sync)| -> Vec<u16> {
        let barrier = Barrier::new(AT_ONCE);
        thread::scope(|scope| {
            let sent: Vec<_> = (0..AT_ONCE)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        send()
                    })
                })
                .collect();
            sent.into_iter()
                .map(|answer| answer.join().expect("a request"))
                .collect()
        })
    };
    // Requests at once open the server's database connections, so that the burst's decisions
    // run side by side.
    let health = format!("{}/health", flow.stand.server.base());
    burst(&|| get_url(&health, &[]).status);

    for round in 0..ROUNDS {
        flow.stand.db.execute("DELETE FROM wrong_user_codes");
        for _ in 1..WRONG_CODE_LIMIT {
            assert_eq!(approve(&flow, &headers, WRONG_CODE), 400, "round {round}");
        }
        let mut statuses = burst(&|| approve(&flow, &headers, WRONG_CODE));
        statuses.sort_unstable();
        let mut expected = vec![429; AT_ONCE];
        expected[0] = 400;
        assert_eq!(statuses, expected, "round {round}");
        assert_eq!(wrong_codes(&flow), WRONG_CODE_LIMIT, "round {round}");
    }
}
