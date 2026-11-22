//! A web address in the server's answer that is no origin, such as one with a control character or
//! a path, is refused before the device signs or stores an invite, and shows no link.

use skald_proto::InviteId;

use crate::{Admin, invite_at_terminal, shows_a_link};

/// The relay answers Ada's request for an invite ID itself, with a new ID and each web address that
/// is no origin. `skald invite acme` at a terminal fails each time, shows no link and sends no
/// invite to store.
#[test]
fn web_address_that_is_no_origin_is_refused_before_anything_is_stored() {
    let admin = Admin::new((6_418_305_001, "ada-gets-a-bad-web-address"));
    let relay = &admin.cloud().relay;
    let issue = format!("POST /spaces/{}/invites", admin.space);
    let store = format!("PUT /spaces/{}/invites/", admin.space);
    for web in [
        "http://skald.test\u{1b}]8;;http://elsewhere.test\u{7}",
        "http://skald.test/app",
        "javascript:alert(1)",
    ] {
        let answer = serde_json::json!({ "invite": InviteId::generate().to_string(), "web": web });
        relay.answer_with(
            &issue,
            "201 Created",
            ("application/json", answer.to_string().as_bytes()),
        );
        let since = relay.mark();

        let refused = invite_at_terminal(&admin.ada.machine, &[]);

        relay.stop_refusing();
        assert!(!refused.success, "{web:?}: {}", refused.text);
        assert!(!shows_a_link(&refused.text), "{web:?}: {}", refused.text);
        let stored = admin.cloud().count_sent(since, &store);
        assert_eq!(stored, 0, "{web:?}: an invite was sent to be stored");
    }
}
