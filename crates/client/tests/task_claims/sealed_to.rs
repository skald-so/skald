//! The scheme of a result ("Results"): under `hpke` to the task's `sender` when it is no member
//! device of the claiming device's current epoch, such as a linked browser of the account.

use skald_crypto::open_message;
use skald_proto::{Cbor as _, Outcome, Scheme};

use crate::claiming::{
    COMPLETE, Claims, Craft, DONE, Expected, Got, assert_result, result_of, sealed_by,
};
use crate::fixtures::hex;
use crate::senders::{OWN, labelled_of};
use crate::wraps::valid_browser;

/// What the test writes and answers; no text is anywhere else.
const FROM_BROWSER: &str = "scheme test: ada's browser asks codex for the deploy log";
const LOG_DONE: &str = "scheme test: codex attached the deploy log";

/// The browser's certificate `name`.
const BROWSER: &str = "ada-browser";

/// A `browser` device of Ada's account, certified by her first device, writes a task from Ada's
/// person address to the queue `default`, sealed to the channel's current epoch; the test stores it
/// in the server's database as the browser's upload. Codex claims it, labelled with the browser and
/// `own`, and completes it. The server gives it `done`; the second machine sent one result under
/// `hpke` to the browser alone, which opens with the browser's key and verifies with the second
/// machine's certificate: from codex to Ada's person address, `hops` 1, `done`, with codex's text.
/// Bob's device reads no result in the channel.
#[test]
fn result_of_a_browser_s_task_goes_under_hpke_to_the_browser() {
    let mut claims = Claims::new(
        [4_267_081_001, 4_267_081_002],
        ["ada-asks-from-her-browser", "bob-reads-no-browser-result"],
    );
    let browser = valid_browser(claims.cloud(), claims.ada(), BROWSER);
    let person = claims.person();
    let craft = Craft::new(&person, claims.queue, FROM_BROWSER);
    let record = sealed_by(
        claims.bob(),
        claims.at,
        (&browser.signing, browser.id),
        &craft,
    );
    claims.cloud().execute(&format!(
        "INSERT INTO tasks (id, queue_id, uploader_id, queued_task) \
         VALUES ('{}'::uuid, '{}'::uuid, '{}'::uuid, '\\x{}'::bytea)",
        record.task,
        claims.queue,
        browser.id,
        hex(&record.to_cbor())
    ));
    let task = record.task;

    let claimed = claims.codex_claimed(&[]);

    let line = labelled_of(&claimed.shown(), FROM_BROWSER, FROM_BROWSER);
    assert_eq!(
        (
            line.from.as_str(),
            line.device.as_str(),
            line.level.as_str()
        ),
        (person.as_str(), BROWSER, OWN),
        "{line:?}"
    );
    let held = claims.read(task).held().clone();
    let sent_before = claims.talk().front.sent().len();

    claims.codex_ran(COMPLETE, &[&task.to_string(), LOG_DONE]);

    assert_eq!(claims.read(task).state, DONE);
    let laptop = claims.laptop();
    let laptop_cert = (claims.cloud().cert(&laptop.token, laptop.device))
        .expect("the second machine's certificate");
    let to_browser: Vec<Got> = claims.talk().front.sent()[sent_before..]
        .iter()
        .map(|sent| sent.envelope.envelope().clone())
        .filter(|envelope| {
            envelope.header.scheme == Scheme::Hpke
                && envelope.header.recipients.as_deref() == Some(&[browser.id][..])
        })
        .filter_map(|envelope| {
            let opened = open_message(
                &envelope.header,
                &envelope.payload,
                browser.id,
                &browser.hpke,
            )
            .unwrap_or_else(|err| panic!("an envelope to the browser that does not open: {err:?}"));
            let sender = opened.sender();
            let content = opened
                .verify(&laptop_cert)
                .unwrap_or_else(|err| panic!("a signature that fails: {err:?}"));
            let result = result_of(&content)?;
            Some(Got {
                msg_id: envelope.header.msg_id,
                sender,
                content,
                result,
            })
        })
        .collect();
    let [one] = to_browser.as_slice() else {
        panic!("not one result to the browser: {to_browser:?}");
    };
    let codex = claims.codex();
    let expected = Expected {
        sender: laptop.device,
        from: &codex,
        to: &person,
        hops: 1,
        task,
        held: &held,
        outcome: Outcome::Done,
        text: LOG_DONE,
    };
    assert_result(one, &expected);
    let in_channel = claims.results(task);
    assert!(in_channel.is_empty(), "a result under mls: {in_channel:?}");
}
