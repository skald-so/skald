//! The stand-in answers as GitHub does, so a server that signs in against it signs in against
//! GitHub; its binary and control endpoints serve tests in other processes.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use skald_github_stand_in::{App, Endpoint, Fault, GitHub, User};

const CLIENT_ID: &str = "Ov23li0standin0test";
const CLIENT_SECRET: &str = "standin-secret-test-0123456789abcdef";
const CALLBACK: &str = "http://skald.test:8080/auth/github/callback";

struct Answer {
    status: u16,
    location: Option<String>,
    set_cookie: Option<String>,
    body: String,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|err| panic!("{}: {err}", self.body))
    }

    /// The query parameters of the redirect.
    fn back(&self) -> Vec<(String, String)> {
        let location = self.location.as_deref().expect("a redirect");
        let query = location.split_once('?').expect("a query").1;
        serde_urlencoded::from_str(query).expect("a query")
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .proxy(None)
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .build()
        .new_agent()
}

fn answer(mut response: ureq::http::Response<ureq::Body>) -> Answer {
    let header = |name| {
        response
            .headers()
            .get(name)
            .map(|value: &ureq::http::HeaderValue| value.to_str().expect("ASCII").to_owned())
    };
    let (location, set_cookie) = (header("location"), header("set-cookie"));
    Answer {
        status: response.status().as_u16(),
        location,
        set_cookie,
        body: response.body_mut().read_to_string().expect("read body"),
    }
}

fn get(url: &str, headers: &[(&str, &str)]) -> Answer {
    let mut request = agent().get(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    answer(request.call().expect("GET"))
}

fn start() -> GitHub {
    let github = GitHub::start(App {
        client_id: CLIENT_ID.to_owned(),
        client_secret: CLIENT_SECRET.to_owned(),
        callback: Some(CALLBACK.to_owned()),
    });
    github.put_user(User {
        email: Some("ada@example.test".to_owned()),
        name: Some("Ada".to_owned()),
        ..User::new(1001, "ada")
    });
    github
}

/// The cookie that signs a browser in to `github` as the user `id`, asking first with `ask`.
fn signed_in(github: &GitHub, id: u64, ask: bool) -> String {
    let url = if ask {
        github.asking_sign_in_url(id)
    } else {
        github.sign_in_url(id)
    };
    let answer = get(&url, &[]);
    assert_eq!(answer.status, 200);
    let cookie = answer.set_cookie.expect("a cookie");
    cookie.split(';').next().expect("a pair").to_owned()
}

fn authorize_url(github: &GitHub, redirect_uri: &str) -> String {
    let query = serde_urlencoded::to_string([
        ("client_id", CLIENT_ID),
        ("redirect_uri", redirect_uri),
        ("state", "s1"),
    ])
    .expect("a query");
    format!("{}/login/oauth/authorize?{query}", github.url())
}

/// A code for the user `id`, from authorize.
fn code(github: &GitHub, id: u64) -> String {
    let cookie = signed_in(github, id, false);
    let back = get(&authorize_url(github, CALLBACK), &[("cookie", &cookie)]);
    assert_eq!(back.status, 302);
    let params = back.back();
    assert_eq!(params[1], ("state".to_owned(), "s1".to_owned()));
    params[0].1.clone()
}

/// Exchanges `code` with the client secret `secret`, the parameters in a form body.
fn exchange(github: &GitHub, code: &str, secret: &str) -> Answer {
    let url = format!("{}/login/oauth/access_token", github.url());
    let form = [
        ("client_id", CLIENT_ID),
        ("client_secret", secret),
        ("code", code),
        ("redirect_uri", CALLBACK),
    ];
    answer(
        agent()
            .post(&url)
            .header("accept", "application/json")
            .send_form(form)
            .expect("POST"),
    )
}

fn user(github: &GitHub, authorization: &str) -> Answer {
    get(
        &format!("{}/user", github.url()),
        &[("authorization", authorization), ("user-agent", "test")],
    )
}

#[test]
fn authorize_goes_back_with_a_code_for_the_signed_in_user() {
    let github = start();
    let url = authorize_url(&github, CALLBACK);

    let unsigned = get(&url, &[]);
    assert_eq!((unsigned.status, unsigned.location), (200, None));

    let cookie = signed_in(&github, 1001, false);
    let back = get(&url, &[("cookie", &cookie)]);
    assert_eq!(back.status, 302);
    assert!(
        back.location
            .as_deref()
            .unwrap()
            .starts_with(&format!("{CALLBACK}?code="))
    );

    let asking = signed_in(&github, 1001, true);
    let page = get(&url, &[("cookie", &asking)]);
    assert_eq!(page.status, 200);
    assert!(
        page.body
            .contains(r#"id="authorize" href="http://skald.test:8080/auth/github/callback?code="#),
        "{}",
        page.body
    );
    assert!(page.body.contains("error=access_denied"), "{}", page.body);

    let recorded = github.requests_to(Endpoint::Authorize);
    assert_eq!(recorded.len(), 3);
    assert_eq!(recorded[0].params["redirect_uri"], CALLBACK);
}

#[test]
fn authorize_refuses_an_unknown_app_and_a_foreign_redirect() {
    let github = start();
    let cookie = signed_in(&github, 1001, false);
    let unknown = format!("{}/login/oauth/authorize?client_id=other", github.url());
    assert_eq!(get(&unknown, &[("cookie", &cookie)]).status, 404);

    for foreign in [
        "http://evil.test:8080/auth/github/callback",
        "http://skald.test:8081/auth/github/callback",
        "http://skald.test:8080/auth/github/callbackx",
        "https://skald.test:8080/auth/github/callback",
    ] {
        let back = get(&authorize_url(&github, foreign), &[("cookie", &cookie)]);
        assert!(
            back.location.as_deref().unwrap().starts_with(CALLBACK),
            "{foreign}"
        );
        assert_eq!(back.back()[0].1, "redirect_uri_mismatch", "{foreign}");
    }
    let below = get(
        &authorize_url(&github, &format!("{CALLBACK}/more")),
        &[("cookie", &cookie)],
    );
    assert_eq!(back_param(&below, "code").map(|code| code.len()), Some(20));
}

fn back_param(answer: &Answer, name: &str) -> Option<String> {
    answer
        .back()
        .into_iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

#[test]
fn token_endpoint_gives_one_token_per_code_to_the_app() {
    let github = start();
    let code = code(&github, 1001);

    let wrong = exchange(&github, &code, "another secret");
    assert_eq!(wrong.status, 200);
    assert_eq!(wrong.json()["error"], "incorrect_client_credentials");

    let granted = exchange(&github, &code, CLIENT_SECRET);
    let token = granted.json()["access_token"].as_str().unwrap().to_owned();
    assert!(token.starts_with("gho_"), "{token}");
    assert_eq!(github.tokens(), vec![token.clone()]);

    let again = exchange(&github, &code, CLIENT_SECRET);
    assert_eq!(again.json()["error"], "bad_verification_code");
    assert_eq!(
        exchange(&github, "0000", CLIENT_SECRET).json()["error"],
        "bad_verification_code"
    );
    assert_eq!(github.requests_to(Endpoint::Token).len(), 4);
}

#[test]
fn token_endpoint_takes_basic_credentials_and_answers_a_form_by_default() {
    let github = start();
    let code = code(&github, 1001);
    let url = format!("{}/login/oauth/access_token", github.url());
    let basic = format!(
        "Basic {}",
        base64_encode(&format!("{CLIENT_ID}:{CLIENT_SECRET}"))
    );
    let granted = answer(
        agent()
            .post(&url)
            .header("authorization", &basic)
            .send_form([("code", code.as_str()), ("redirect_uri", CALLBACK)])
            .expect("POST"),
    );
    let fields: Vec<(String, String)> = serde_urlencoded::from_str(&granted.body).expect("a form");
    assert_eq!(fields[0].0, "access_token", "{}", granted.body);

    let other = self::code(&github, 1001);
    let mismatch = answer(
        agent()
            .post(&url)
            .header("authorization", &basic)
            .send_form([
                ("code", other.as_str()),
                ("redirect_uri", "http://skald.test:8080/x"),
            ])
            .expect("POST"),
    );
    assert!(
        mismatch.body.contains("error=redirect_uri_mismatch"),
        "{}",
        mismatch.body
    );
}

/// RFC 7636, Appendix B: a verifier and its S256 challenge.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

/// A code for the user 1001 from an authorization request with `challenge` as `method`, if given.
fn code_with_challenge(github: &GitHub, challenge: &str, method: Option<&str>) -> String {
    let cookie = signed_in(github, 1001, false);
    let mut params = vec![
        ("client_id", CLIENT_ID),
        ("redirect_uri", CALLBACK),
        ("state", "s1"),
        ("code_challenge", challenge),
    ];
    params.extend(method.map(|method| ("code_challenge_method", method)));
    let query = serde_urlencoded::to_string(&params).expect("a query");
    let url = format!("{}/login/oauth/authorize?{query}", github.url());
    let back = get(&url, &[("cookie", &cookie)]);
    back_param(&back, "code").expect("a code")
}

/// Exchanges `code` with the verifier `verifier`, if given.
fn exchange_verified(github: &GitHub, code: &str, verifier: Option<&str>) -> Value {
    let url = format!("{}/login/oauth/access_token", github.url());
    let mut form = vec![
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
        ("code", code),
    ];
    form.extend(verifier.map(|verifier| ("code_verifier", verifier)));
    let sent = agent()
        .post(&url)
        .header("accept", "application/json")
        .send_form(form)
        .expect("POST");
    answer(sent).json()
}

#[test]
fn token_endpoint_takes_a_code_with_a_challenge_only_with_its_verifier() {
    let github = start();
    let s256 = Some("S256");
    for verifier in [None, Some(""), Some(CHALLENGE), Some(&VERIFIER[1..])] {
        let code = code_with_challenge(&github, CHALLENGE, s256);
        let refused = exchange_verified(&github, &code, verifier);
        assert_eq!(refused["error"], "bad_verification_code", "{verifier:?}");
    }
    let code = code_with_challenge(&github, CHALLENGE, s256);
    let granted = exchange_verified(&github, &code, Some(VERIFIER));
    assert!(granted["access_token"].is_string(), "{granted}");

    let plain = code_with_challenge(&github, VERIFIER, None);
    let refused = exchange_verified(&github, &plain, Some(CHALLENGE));
    assert_eq!(refused["error"], "bad_verification_code");
    let plain = code_with_challenge(&github, VERIFIER, Some("plain"));
    assert!(exchange_verified(&github, &plain, Some(VERIFIER))["access_token"].is_string());

    // Without a challenge, a code needs no verifier.
    let code = self::code(&github, 1001);
    assert!(exchange_verified(&github, &code, None)["access_token"].is_string());

    let cookie = signed_in(&github, 1001, false);
    let query = serde_urlencoded::to_string([
        ("client_id", CLIENT_ID),
        ("redirect_uri", CALLBACK),
        ("code_challenge", CHALLENGE),
        ("code_challenge_method", "S512"),
    ])
    .expect("a query");
    let url = format!("{}/login/oauth/authorize?{query}", github.url());
    let back = get(&url, &[("cookie", &cookie)]);
    assert_eq!(
        back_param(&back, "error").as_deref(),
        Some("invalid_request")
    );
}

fn base64_encode(text: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(text)
}

#[test]
fn user_endpoint_gives_the_token_s_user() {
    let github = start();
    let code = code(&github, 1001);
    let token = exchange(&github, &code, CLIENT_SECRET).json()["access_token"]
        .as_str()
        .unwrap()
        .to_owned();

    let ada = user(&github, &format!("Bearer {token}"));
    assert_eq!(ada.status, 200);
    let body = ada.json();
    assert_eq!(
        (body["id"].clone(), body["login"].clone()),
        (json!(1001), json!("ada"))
    );
    assert_eq!(body["email"], "ada@example.test");

    github.put_user(User::new(1001, "ada-renamed"));
    assert_eq!(
        user(&github, &format!("token {token}")).json()["login"],
        "ada-renamed"
    );
    assert_eq!(user(&github, "Bearer gho_unknown").status, 401);
    // ureq always sends a User-Agent: this request goes by hand.
    let address = github.url().strip_prefix("http://").unwrap();
    let mut stream = TcpStream::connect(address).expect("connect");
    write!(
        stream,
        "GET /user HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    )
    .expect("send");
    let mut anonymous = String::new();
    stream.read_to_string(&mut anonymous).expect("read");
    assert!(
        anonymous.starts_with("HTTP/1.1 403"),
        "GitHub requires a User-Agent: {anonymous}"
    );
}

#[test]
fn faults_play_until_set_again() {
    let github = start();
    let cookie = signed_in(&github, 1001, false);
    github.set_faults(&[Fault::AccessDenied]);
    let denied = get(&authorize_url(&github, CALLBACK), &[("cookie", &cookie)]);
    assert_eq!(
        back_param(&denied, "error").as_deref(),
        Some("access_denied")
    );
    assert_eq!(back_param(&denied, "state").as_deref(), Some("s1"));

    github.set_faults(&[]);
    let code = code(&github, 1001);
    github.set_faults(&[Fault::BadVerificationCode]);
    assert_eq!(
        exchange(&github, &code, CLIENT_SECRET).json()["error"],
        "bad_verification_code"
    );
    github.set_faults(&[Fault::TokenStatus { status: 503 }]);
    assert_eq!(exchange(&github, &code, CLIENT_SECRET).status, 503);
    github.set_faults(&[]);
    let token = exchange(&github, &code, CLIENT_SECRET).json()["access_token"]
        .as_str()
        .unwrap()
        .to_owned();
    github.set_faults(&[Fault::UserStatus { status: 500 }]);
    assert_eq!(user(&github, &format!("Bearer {token}")).status, 500);
    github.set_faults(&[]);
    assert_eq!(user(&github, &format!("Bearer {token}")).status, 200);
}

#[test]
fn binary_serves_the_control_endpoints() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_skald-github-stand-in"))
        .args(["--client-id", CLIENT_ID, "--client-secret", CLIENT_SECRET])
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the stand-in");
    let mut line = String::new();
    BufReader::new(child.stderr.take().unwrap())
        .read_line(&mut line)
        .expect("read stderr");
    let base = line
        .trim_end()
        .strip_prefix("skald-github-stand-in: listening on ")
        .unwrap_or_else(|| panic!("{line:?}"))
        .to_owned();
    let put = |path: &str, body: Value| {
        agent()
            .put(&format!("{base}{path}"))
            .header("content-type", "application/json")
            .send(body.to_string())
            .expect("PUT")
            .status()
            .as_u16()
    };

    assert_eq!(
        put(
            "/_stand-in/users",
            json!({ "id": 7, "login": "bob", "email": null, "name": null })
        ),
        204
    );
    let cookie = get(&format!("{base}/_stand-in/sign-in?user=7"), &[])
        .set_cookie
        .unwrap();
    let authorize = format!(
        "{base}/login/oauth/authorize?client_id={CLIENT_ID}&redirect_uri=http://x.test/cb&state=s"
    );
    let cookie = cookie.split(';').next().unwrap();
    assert_eq!(get(&authorize, &[("cookie", cookie)]).status, 302);
    assert_eq!(
        put("/_stand-in/faults", json!([{ "fault": "access_denied" }])),
        204
    );
    let denied = get(&authorize, &[("cookie", cookie)]);
    assert_eq!(
        back_param(&denied, "error").as_deref(),
        Some("access_denied")
    );

    let requests = get(&format!("{base}/_stand-in/requests"), &[]).json();
    assert_eq!(requests.as_array().map(Vec::len), Some(2), "{requests}");
    assert_eq!(requests[0]["endpoint"], "authorize");
    assert_eq!(
        get(&format!("{base}/_stand-in/tokens"), &[]).json(),
        json!([])
    );
    assert_eq!(
        get(&format!("{base}/_stand-in/sign-in?user=8"), &[]).status,
        404
    );
    let _ = child.kill();
    let _ = child.wait();
}
