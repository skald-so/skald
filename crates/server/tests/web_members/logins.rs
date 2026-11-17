//! The GitHub login of a member's account: only for an account with a GitHub method. On a server
//! with both providers' settings, alice signs up with GitHub and her machine signs in by the device
//! flow, and gail signs up with Google, so her account has no GitHub method.

use serde_json::{Value, json};
use skald_proto::{AccountId, Member, Name, PROTOCOL_VERSION, Role, RosterBody, SpaceId, Unknown};

use crate::api::space_of;
use crate::common::accounts::Who;
use crate::common::device_flow::{INTERVAL, Machine, text};
use crate::common::devices::{DEVICES, Device, Key, MASTER, assert_taken, b64};
use crate::common::google::{Google, Setup};
use crate::common::sign_in::Jar;
use crate::common::spec::{operation_violations, spec};
use crate::common::{Response, post_form, send};
use crate::listed::members;
use crate::rosters::{TIMESTAMP, next, signed};
use crate::sessions::{Api, COOKIE};

/// The device flow's verification page.
const VERIFICATION: &str = "/device";

/// A web session: the `Cookie` header of its session cookie, and its account.
struct Signed {
    cookie: String,
    who: Who,
}

impl Signed {
    fn account(&self) -> AccountId {
        self.who.account.parse().expect("an account ID")
    }

    fn handle(&self) -> Name {
        self.who
            .username
            .parse()
            .expect("a username that is a valid name")
    }
}

/// Ends the sign-in that `started` holds, its cookies and the callback URL, on `google`'s server.
fn web_session(google: &Google, (started, callback): (Jar, String)) -> Signed {
    let jar = google.finish(&started, &callback);
    let header = jar.header();
    let cookie = header
        .split("; ")
        .find(|pair| pair.starts_with(&format!("{COOKIE}=")))
        .unwrap_or_else(|| panic!("a session cookie: {header}"))
        .to_owned();
    Signed {
        who: Who::of(&google.session_of(&jar)),
        cookie,
    }
}

/// `<method> <path>` of the JSON `body` to `google`'s server as the session `token`, checked
/// against the spec's operation at `template`.
fn as_machine(
    google: &Google,
    token: &str,
    (method, path, template): (&str, &str, &str),
    body: &Value,
) -> Response {
    let version = PROTOCOL_VERSION.to_string();
    let bearer = format!("Bearer {token}");
    let headers = [
        (skald_proto::VERSION_HEADER, version.as_str()),
        ("authorization", bearer.as_str()),
        ("content-type", "application/json"),
    ];
    let url = format!("{}{path}", google.server.base());
    let bytes = serde_json::to_vec(body).expect("JSON");
    let response = send(method, &url, &headers, &bytes);
    let violations = operation_violations(&spec(), method, template, &response);
    assert_eq!(violations, Vec::<String>::new(), "{}", text(&response));
    response
}

/// Signs in a machine of `web`'s account by the device flow, approved by that web session, gives
/// the account its master key with `laptop`'s certificate, and creates the space `name`; gives the
/// machine's session token and the space.
fn machine_with_space(
    google: &Google,
    web: &Signed,
    (master, laptop): (&Key, &Device),
    name: &str,
) -> (String, SpaceId) {
    let base = google.server.base();
    let machine = Machine::at(base);
    let grant = machine.authorize();
    let page_headers = [("origin", google.public.as_str()), ("cookie", &web.cookie)];
    let decision = [("decision", "approve"), ("user_code", &grant.user_code)];
    let approved = post_form(&format!("{base}{VERIFICATION}"), &page_headers, &decision);
    assert_eq!(approved.status, 200, "approval: {}", text(&approved));
    let token = machine.token(&grant);

    let call = |route, body: &Value| as_machine(google, &token, route, body);
    assert_taken(&call(("post", DEVICES, DEVICES), &laptop.registration()));
    let cert = laptop.cert_as(web.account(), skald_proto::DeviceKind::Client, master);
    let given = json!({ "master": b64(&master.public()), "cert": b64(&cert) });
    assert_taken(&call(("put", MASTER, MASTER), &given));
    let created = call(("post", "/spaces", "/spaces"), &json!({ "name": name }));
    assert_taken(&created);
    (token, space_of(&created))
}

/// Alice, signed up with GitHub, creates `acme`; revision 1 lists her as its admin, and revision 2
/// adds gail, signed up with Google, as a member. Both web sessions get alice with the login of her
/// GitHub method and gail without a login.
#[test]
fn the_login_is_given_only_for_an_account_with_a_github_method() {
    let google = Google::launch(Setup {
        github: true,
        args: &[INTERVAL, "1"],
        ..Setup::default()
    });
    let login = "Alice-Logins-Standin";
    let user = google.github_user(3_225_600_001, login, "alice@example.org");
    let alice = web_session(&google, google.github_to_callback(&user));
    let person = google.person("3225600002", "gail@example.test");
    let gail = web_session(&google, google.to_callback(&person));
    assert_eq!(alice.who.github_login.as_deref(), Some(login));
    assert_eq!(
        gail.who.github_login, None,
        "a Google account: {:?}",
        gail.who
    );

    let master = Key::generate();
    let laptop = Device::new("Laptop");
    let (token, space) = machine_with_space(&google, &alice, (&master, &laptop), "acme");
    let store = |revision: &[u8]| {
        let path = format!("/spaces/{space}/revisions");
        let route = ("post", path.as_str(), "/spaces/{space}/revisions");
        let body = json!({ "revision": b64(revision) });
        assert_taken(&as_machine(&google, &token, route, &body));
    };
    let one = RosterBody::first(
        space,
        "acme".parse().expect("a name"),
        alice.account(),
        alice.handle(),
        master.public(),
        laptop.id,
        TIMESTAMP,
    );
    let one = signed(&one, &laptop.signing.key);
    store(&one);
    let mut two = next(&one, &laptop);
    two.members.push(Member {
        account: gail.account(),
        handle: gail.handle(),
        role: Role::Member,
        master: Key::generate().public(),
        invite: None,
        excluded: None,
        unknown: Unknown::default(),
    });
    store(&signed(&two, &laptop.signing.key));

    let api = Api::of_google(&google);
    for (web, who) in [(&alice, "alice"), (&gail, "gail")] {
        let shown = members(&api, space, &web.cookie);
        let logins: Vec<(String, Option<String>)> = shown
            .into_iter()
            .map(|(handle, member)| (handle, member.login))
            .collect();
        let mut expected = vec![
            (alice.who.username.clone(), Some(login.to_owned())),
            (gail.who.username.clone(), None),
        ];
        expected.sort();
        assert_eq!(logins, expected, "{who}'s web session");
    }
}
