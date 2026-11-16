//! A session ended through one instance is refused at once on the others, and its connections to
//! them close.

use std::time::Duration;

use tungstenite::protocol::frame::coding::CloseCode;

use crate::common::Closed;
use crate::common::device_flow::{DeviceFlow, INTERVAL, text};
use crate::sessions::{
    Api, CHROME_ON_MACOS, Connection, FIREFOX_ON_WINDOWS, Kind, Laptop, Listed, web_sign_in,
};

/// How soon another instance closes the connections of an ended session.
const CLOSE: Duration = Duration::from_secs(1);

/// With two instances on one database: Ada's laptop is connected to each as its device when her
/// web session ends its CLI session through the first. Within a second, each closes its connection
/// with the reason `session-ended`, and the second refuses the session; a web session ended
/// through the first is refused by the second too, and the other sessions work on both.
#[test]
fn ended_cli_sessions_connection_to_another_instance_closes_within_a_second() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1"]);
    let first = Api::of(&flow);
    let other = flow.stand.instance(&[], &[]);
    let second = first.on(&other);
    let ada = flow.stand.user(3_222_400_001, "ada-instances-standin");
    let laptop = Laptop::sign_in(&flow, &ada, "ada-laptop");
    let firefox = web_sign_in(&flow, &ada, FIREFOX_ON_WINDOWS);
    let cookie = web_sign_in(&flow, &ada, CHROME_ON_MACOS);
    let mut on_second = Connection::open(&other, first.public, &laptop.token, &laptop.device);
    let mut on_first = Connection::open(
        &flow.stand.server,
        first.public,
        &laptop.token,
        &laptop.device,
    );
    let listed = first.list(&cookie);
    let id_of = |pick: &dyn Fn(&Listed) -> bool| {
        let found: Vec<&Listed> = listed.iter().filter(|session| pick(session)).collect();
        assert_eq!(found.len(), 1, "{listed:#?}");
        found[0].id.clone()
    };
    let device = id_of(&|session| session.kind == Kind::Device);
    let web = id_of(&|session| {
        session.kind == Kind::Web
            && session
                .browser
                .as_deref()
                .is_some_and(|name| name.to_lowercase().contains("firefox"))
    });

    let ended = first.end(&cookie, &device);
    assert_eq!(ended.status, 204, "{}", text(&ended));

    let session_ended = Some(Closed::Frame(CloseCode::Policy, "session-ended".to_owned()));
    assert_eq!(
        on_second.closed_within(CLOSE),
        session_ended,
        "on the second"
    );
    assert_eq!(on_first.closed_within(CLOSE), session_ended, "on the first");
    assert!(!second.device_works(&laptop.token));

    let ended = first.end(&cookie, &web);
    assert_eq!(ended.status, 204, "{}", text(&ended));
    assert!(!second.web_works(&firefox));
    assert!(second.web_works(&cookie));
    assert_eq!(second.list(&cookie).len(), listed.len() - 2);
}
