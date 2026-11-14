//! The responses of linking and of the account's sign-in methods conform to the OpenAPI spec in
//! `api/openapi.json`, and its GitHub sign-in start names the PKCE challenge.

use std::thread;
use std::time::Duration;

use serde_json::Value;
use skald_github_stand_in::User;
use skald_proto::{PROTOCOL_VERSION, VERSION_HEADER};

use super::common::google::{CALLBACK as GOOGLE_CALLBACK, Google, Setup};
use super::common::sign_in::{CALLBACK as GITHUB_CALLBACK, Jar, cookie_set, param};
use super::common::spec::{operation_violations, spec};
use super::common::{Response, get_url, post_form};
use super::{
    GITHUB_LINK, GOOGLE_LINK, LINK_AGE, METHODS, METHODS_PAGE, Method, github_and_google,
    methods_in,
};

/// A server with both providers' settings and the extra arguments `args`, whose public address is
/// where the test sends its requests.
fn local(args: &[&str]) -> Google {
    Google::launch(Setup {
        local: true,
        github: true,
        args,
        ..Setup::default()
    })
}

/// Checks that `response` to `<method> <path>` conforms to the spec.
#[track_caller]
fn conforms(spec: &Value, method: &str, path: &str, response: &Response) {
    let violations = operation_violations(spec, method, path, response);
    assert_eq!(violations, Vec::<String>::new());
}

/// `POST <path>` of no fields straight to the server with the request headers `headers`.
fn post(google: &Google, path: &str, headers: &[(&str, &str)]) -> Response {
    post_form(&format!("{}{path}", google.server.base()), headers, &[])
}

/// The web session's cookies after a sign-in that ends at `callback` with the cookies of `jar`.
fn signed_in(google: &Google, (jar, callback): (Jar, String)) -> Jar {
    google.finish(&jar, &callback)
}

/// `GET /account/sign-in-methods` with the cookies of `jar`, stating the protocol version.
fn list(google: &Google, jar: &Jar) -> Response {
    let version = PROTOCOL_VERSION.to_string();
    let headers = [
        (VERSION_HEADER, version.as_str()),
        ("cookie", &jar.header()),
    ];
    google.server.get_with(METHODS, &headers)
}

/// Starts a GitHub link from the session of `jar` and brings the GitHub user `user` back through
/// GitHub; gives the start's answer and the callback's.
fn github_link(google: &Google, spec: &Value, jar: &Jar, user: &User) -> (Response, Response) {
    let origin = google.public.as_str();
    let start = post(
        google,
        GITHUB_LINK,
        &[("cookie", &jar.header()), ("origin", origin)],
    );
    conforms(spec, "post", GITHUB_LINK, &start);
    assert!((300..400).contains(&start.status), "{:?}", start.headers);
    let github = google.github();
    let authorize = start.header("location").expect("a redirect to GitHub");
    assert!(
        authorize.starts_with(&format!("{}/login/oauth/authorize?", github.url())),
        "{authorize}"
    );
    let mut cookies = jar.clone();
    cookies.take(&start);
    let at_github = cookie_set(&get_url(&github.sign_in_url(user.id), &[]), "stand_in_user");
    let back = get_url(authorize, &[("cookie", &at_github)]);
    let callback = back.header("location").expect("GitHub goes back");
    assert!(callback.starts_with(&format!("{}{GITHUB_CALLBACK}?", google.public)));
    let done = settle(google, spec, callback, &mut cookies);
    (start, done)
}

/// The address of the page that `response` refreshes itself to with
/// `<meta http-equiv="refresh" content="0;url=...">`, if it does.
fn refresh(response: &Response) -> Option<String> {
    if !response.content_type.starts_with("text/html") {
        return None;
    }
    let page = String::from_utf8_lossy(&response.body).into_owned();
    let lower = page.to_ascii_lowercase();
    let meta = lower.find("http-equiv=\"refresh\"")?;
    let content = meta + lower[meta..].find("content=\"")? + "content=\"".len();
    let end = content + lower[content..].find('"')?;
    let url = lower[content..end].find("url=")? + content + "url=".len();
    Some(page[url..end].trim().replace("&amp;", "&"))
}

/// Opens `url`, a page of the server's API, with the cookies of `jar`, then each page of the API
/// that the answer redirects or refreshes itself to, as a browser does, keeping the cookies each
/// answer sets. Checks each answer against the spec; gives the last answer of the API.
fn settle(google: &Google, spec: &Value, url: &str, jar: &mut Jar) -> Response {
    let base = google.server.base();
    let mut path = url.replacen(&google.public, "", 1);
    for _ in 0..5 {
        let response = get_url(&format!("{base}{path}"), &[("cookie", &jar.header())]);
        jar.take(&response);
        let route = |path: &str| path.split(['?', '#']).next().unwrap_or_default().to_owned();
        conforms(spec, "get", &route(&path), &response);
        let next = match response.header("location") {
            Some(to) if (300..400).contains(&response.status) => Some(to.to_owned()),
            _ => refresh(&response),
        };
        let next = next.map(|next| next.replacen(&google.public, "", 1));
        // Only to a path of the API: a page such as `/` belongs to the web UI.
        match next {
            Some(next) if next.starts_with('/') && spec["paths"].get(route(&next)).is_some() => {
                path = next;
            }
            _ => return response,
        }
    }
    panic!("{path} keeps refreshing");
}

#[test]
fn link_and_list_responses_conform_to_spec() {
    let spec = spec();
    let google = local(&[]);
    let ada = google.person("110248495921238988400", "ada.linkspec@example.com");
    let bob = google.github_user(5_310_448_401, "bob-linkspec-standin", "bob.ls@example.org");
    let carol = google.github_user(5_310_448_402, "carol-linkspec-standin", "carol@example.org");
    let ada_jar = signed_in(&google, google.to_callback(&ada));
    signed_in(&google, google.github_to_callback(&carol));

    // The page and the list, with a session and without.
    let page = google
        .server
        .get_with(METHODS_PAGE, &[("cookie", &ada_jar.header())]);
    assert_eq!(page.status, 200);
    conforms(&spec, "get", METHODS_PAGE, &page);
    conforms(&spec, "get", METHODS_PAGE, &google.server.get(METHODS_PAGE));
    let listed = list(&google, &ada_jar);
    assert_eq!(listed.status, 200);
    conforms(&spec, "get", METHODS, &listed);
    let none = list(&google, &Jar::default());
    assert_eq!(none.status, 401);
    conforms(&spec, "get", METHODS, &none);

    // A link of a new GitHub user, and one of another account's.
    let (_, linked) = github_link(&google, &spec, &ada_jar, &bob);
    assert_eq!(linked.status, 200);
    let (_, refused) = github_link(&google, &spec, &ada_jar, &carol);
    assert!((400..500).contains(&refused.status), "{}", refused.status);
    let listed = list(&google, &ada_jar);
    conforms(&spec, "get", METHODS, &listed);
    let body = serde_json::from_slice(&listed.body).expect("JSON");
    assert_eq!(methods_in(&body), github_and_google(&bob));

    // A Google link, of the account's own Google method.
    let origin = google.public.as_str();
    let start = post(
        &google,
        GOOGLE_LINK,
        &[("cookie", &ada_jar.header()), ("origin", origin)],
    );
    conforms(&spec, "post", GOOGLE_LINK, &start);
    assert!((300..400).contains(&start.status), "{:?}", start.headers);
    let authorize = start
        .header("location")
        .expect("a redirect to the provider");
    assert!(
        authorize.starts_with(&format!("{}?", google.provider.authorize_url())),
        "{authorize}"
    );
    assert_eq!(
        param(authorize, "prompt").as_deref(),
        Some("select_account")
    );
    let mut cookies = ada_jar.clone();
    cookies.take(&start);
    let mut at_provider = Jar::default();
    at_provider.take(&get_url(&google.provider.sign_in_url(&ada.subject), &[]));
    let back = get_url(authorize, &[("cookie", &at_provider.header())]);
    let callback = back.header("location").expect("the provider goes back");
    assert!(callback.starts_with(&format!("{}{GOOGLE_CALLBACK}?", google.public)));
    settle(&google, &spec, callback, &mut cookies);

    // Starts that go nowhere: no session, another site's `Origin`, no `Origin`.
    let session = ada_jar.header();
    let refusals: [&[(&str, &str)]; 3] = [
        &[("origin", origin)],
        &[("cookie", &session), ("origin", "http://evil.test")],
        &[("cookie", &session)],
    ];
    for headers in refusals {
        for path in [GITHUB_LINK, GOOGLE_LINK] {
            let refused = post(&google, path, headers);
            conforms(&spec, "post", path, &refused);
            let to = refused.header("location").unwrap_or_default();
            assert!(
                !to.contains("/authorize"),
                "{path} {headers:?} goes nowhere: {to}"
            );
        }
    }
}

#[test]
fn old_session_link_response_conforms_to_spec() {
    let spec = spec();
    let google = local(&[LINK_AGE, "2"]);
    let ada = google.person("110248495921238988410", "ada.oldspec@example.com");
    let jar = signed_in(&google, google.to_callback(&ada));
    thread::sleep(Duration::from_secs(4));

    let origin = google.public.as_str();
    for path in [GITHUB_LINK, GOOGLE_LINK] {
        let refused = post(
            &google,
            path,
            &[("cookie", &jar.header()), ("origin", origin)],
        );
        conforms(&spec, "post", path, &refused);
        let to = refused.header("location").unwrap_or_default();
        assert!(!to.contains("/authorize"), "{path} goes nowhere: {to}");
    }
    let listed = list(&google, &jar);
    assert_eq!(listed.status, 200, "an old session still gets its list");
    let body = serde_json::from_slice(&listed.body).expect("JSON");
    assert_eq!(methods_in(&body), vec![Method::Google]);
}

#[test]
fn github_sign_in_start_names_the_pkce_challenge() {
    let spec = spec();
    let start = &spec["paths"]["/auth/github"]["get"];
    assert_eq!(start["operationId"], "startGitHubSignIn");
    let text = start.to_string();
    for name in ["code_challenge", "S256"] {
        assert!(
            text.contains(name),
            "startGitHubSignIn names {name}: {text}"
        );
    }
}
