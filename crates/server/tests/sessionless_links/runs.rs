//! A run between a new device without a session and a session of the account that enters its
//! pairing goes through byte for byte, also with each side on another instance; nobody else gets or
//! relays its messages.

use crate::common::devices::{Device, assert_taken};
use crate::kit::{Fresh, pair, reply_and_request, send_pairing};
use crate::relay::{abort, assert_refused_link, grant, messages_of, position};

#[test]
fn a_run_of_a_device_without_a_session_goes_through_byte_for_byte() {
    let f = Fresh::start(3_175_300_011);
    let issuer = &f.alice.session;
    let run = f.open(&Device::new("Tablet"));
    assert_eq!(
        pair(issuer, &run.pairing),
        run.hello,
        "the issuing device takes the hello by the pairing"
    );

    let (reply, request) = reply_and_request(issuer, &run);

    let grant = grant(&run.session);
    assert_taken(&issuer.send_message(&grant));
    let got = run.messages_with(&grant);
    assert!(
        position(&got, &reply) < position(&got, &request)
            && position(&got, &request) < position(&got, &grant),
        "in the order taken: {got:?}"
    );
    assert_eq!(
        issuer.messages(&run.session),
        got,
        "both sides get the same messages"
    );
}

#[test]
fn a_run_of_a_device_without_a_session_goes_through_with_each_side_on_another_instance() {
    let f = Fresh::start(3_175_300_021);
    let other = f.flow.stand.instance(&[], &[]);
    let issuer = f.alice.session.on(&other);
    let run = f.open(&Device::new("Tablet"));
    assert_eq!(
        pair(&issuer, &run.pairing),
        run.hello,
        "paired on the other instance"
    );

    reply_and_request(&issuer, &run);

    // The new device moves to the other instance, the issuing device back to the first.
    let run = run.on(&other);
    let issuer = &f.alice.session;
    let grant = grant(&run.session);
    assert_taken(&issuer.send_message(&grant));
    run.messages_with(&grant);

    // A `link-abort` the new device sends on one instance ends the run on the other.
    assert_taken(&run.send(&abort(&run.session)));
    crate::kit::until_refused(
        issuer,
        &crate::relay::grant(&run.session),
        "a grant after the abort, on the first instance",
    );
}

#[test]
fn nobody_else_gets_or_relays_the_runs_messages() {
    let f = Fresh::start(3_175_300_031);
    let issuer = &f.alice.session;
    let bob = &f.bob.session;
    let run = f.open(&Device::new("Tablet"));
    assert_eq!(
        issuer.hello(),
        None,
        "no account's latest hello before it pairs"
    );
    assert_eq!(bob.hello(), None, "nor bob's");
    pair(issuer, &run.pairing);
    crate::relay::assert_refused_link(
        &send_pairing(bob, &run.pairing),
        "bob enters the pairing alice's session entered",
    );

    let (reply, request) = reply_and_request(issuer, &run);

    // bob's session gets none of the run's messages and relays none into it.
    let got = bob.messages_response(&run.session);
    if got.status == 200 {
        assert_eq!(messages_of(&got), Vec::<Vec<u8>>::new(), "bob gets none");
    } else {
        assert_refused_link(&got, "bob's request for the run's messages");
    }
    let bobs = grant(&run.session);
    assert_refused_link(&bob.send_message(&bobs), "bob's grant");
    assert_refused_link(&bob.send_message(&abort(&run.session)), "bob's abort");

    // Requests without the run's token, or with another run's, get none either.
    let nobody = f.nobody();
    for (side, what) in [
        (nobody.clone(), "no token"),
        (
            crate::api::Session::of_token(&f.flow, "not-a-run-token"),
            "an unknown token",
        ),
        (
            crate::api::Session::of_token(&f.flow, f.bob.session.token()),
            "bob's session token",
        ),
    ] {
        let got = side.call("get", crate::kit::RUN_MESSAGES, None);
        assert_eq!(
            got.status,
            401,
            "{what}: {}",
            crate::common::device_flow::text(&got)
        );
        let sent = side.call(
            "post",
            crate::kit::RUN_MESSAGES,
            Some(&serde_json::json!({ "message": crate::common::devices::b64(&bobs) })),
        );
        assert_eq!(
            sent.status,
            401,
            "{what}: {}",
            crate::common::device_flow::text(&sent)
        );
    }
    let another = f.open(&Device::new("Phone"));
    assert_eq!(
        another.messages(),
        Vec::<Vec<u8>>::new(),
        "another run's token"
    );
    assert_refused_link(
        &another.send(&abort(&run.session)),
        "another run's token aborts this run",
    );

    let ours = grant(&run.session);
    assert_taken(&issuer.send_message(&ours));
    let got = run.messages_with(&ours);
    assert_eq!(
        got,
        vec![reply, request, ours],
        "only the run's own messages"
    );
}
