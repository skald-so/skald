//! The account a Google sign-in reaches: bound to the provider's subject, never to the email.

use std::sync::Barrier;
use std::thread;

use super::common::browser::Browser;
use super::{Google, Setup, Who};

#[test]
fn first_sign_in_creates_an_account_and_the_next_reaches_it() {
    let browser = Browser::launch();
    let google = Google::start();
    let ada = google.person("110248495921238986420", "ada.first@example.com");

    let first = browser.page();
    google.assert_signed_out(&first);
    google.sign_in(&first, &ada);
    let account = google.account(&first);
    assert_eq!(account.github_login, None, "a Google account: {account:?}");
    assert!(
        !google.db.rows().is_empty(),
        "the database holds the account"
    );

    // Another browser of the same person reaches the same account.
    let next = browser.page();
    google.sign_in(&next, &ada);
    assert_eq!(google.account(&next), account);
    assert_eq!(google.account(&first), account);
}

#[test]
fn email_change_keeps_the_account() {
    let browser = Browser::launch();
    let google = Google::start();
    let grace = google.person("110248495921238986421", "grace.email@example.com");
    let account = google.signed_up(&browser, &grace);

    let moved = google.person(&grace.subject, "grace.moved@example.org");
    let next = browser.page();
    google.sign_in(&next, &moved);
    assert_eq!(
        google.account(&next),
        account,
        "the same subject reaches the same account, with the same username"
    );

    // Another person who takes the old email gets an account of their own.
    let newcomer = google.person("110248495921238986422", &grace.email);
    let other = google.signed_up(&browser, &newcomer);
    assert_ne!(other.account, account.account);
    assert_ne!(other.username, account.username);
    let again = browser.page();
    google.sign_in(&again, &moved);
    assert_eq!(google.account(&again), account);
}

#[test]
fn two_first_sign_ins_of_one_subject_at_once_create_one_account() {
    const ROUNDS: usize = 5;
    let google = Google::start();
    let mut accounts = Vec::new();

    for round in 0..ROUNDS {
        let person = google.person(
            &format!("11024849592123898643{round}"),
            &format!("ada.at.once.{round}@example.com"),
        );
        // Two browsers of the person, each held at the provider's redirect back to the server.
        let held = [google.to_callback(&person), google.to_callback(&person)];
        let ready = Barrier::new(held.len());
        let jars: Vec<_> = thread::scope(|scope| {
            let running: Vec<_> = held
                .iter()
                .map(|(jar, callback)| {
                    let ready = &ready;
                    let google = &google;
                    scope.spawn(move || {
                        ready.wait();
                        google.finish(jar, callback)
                    })
                })
                .collect();
            running
                .into_iter()
                .map(|done| done.join().expect("a sign-in"))
                .collect()
        });
        let reached: Vec<Who> = jars
            .iter()
            .map(|jar| Who::of(&google.session_of(jar)))
            .collect();
        assert_eq!(reached[0], reached[1], "round {round}");

        // The next sign-in reaches that one account too.
        let (jar, callback) = google.to_callback(&person);
        let next = Who::of(&google.session_of(&google.finish(&jar, &callback)));
        assert_eq!(next, reached[0], "round {round}");
        accounts.push(next.account);
    }
    accounts.sort();
    accounts.dedup();
    assert_eq!(accounts.len(), ROUNDS, "one account per person");
}

#[test]
fn two_google_users_get_two_accounts() {
    let browser = Browser::launch();
    let google = Google::start();
    let ada = google.person("110248495921238986440", "ada.two@example.com");
    let bob = google.person("110248495921238986441", "bob.two@example.com");

    let ada_page = browser.page();
    google.sign_in(&ada_page, &ada);
    let bob_page = browser.page();
    google.sign_in(&bob_page, &bob);

    let ada_account = google.account(&ada_page);
    let bob_account = google.account(&bob_page);
    assert_ne!(ada_account.account, bob_account.account);
    assert_ne!(ada_account.username, bob_account.username);
}

#[test]
fn google_and_github_users_with_one_email_get_two_accounts() {
    let browser = Browser::launch();
    let google = Google::launch(Setup {
        github: true,
        ..Setup::default()
    });

    // GitHub first, then Google, with one email.
    let octo = google.github_user(7_301_552_901, "ada-email-standin", "ada.same@example.com");
    let github_page = browser.page();
    google.github_sign_in(&github_page, &octo);
    let github_account = google.account(&github_page);
    assert_eq!(
        github_account.github_login.as_deref(),
        Some(octo.login.as_str())
    );
    let ada = google.person("110248495921238986450", "ada.same@example.com");
    let google_account = google.signed_up(&browser, &ada);
    assert_ne!(google_account.account, github_account.account);

    // Google first, then GitHub, with another email.
    let bob = google.person("110248495921238986451", "bob.same@example.com");
    let bob_google = google.signed_up(&browser, &bob);
    let octo_bob = google.github_user(7_301_552_902, "bob-email-standin", "bob.same@example.com");
    let page = browser.page();
    google.github_sign_in(&page, &octo_bob);
    let bob_github = google.account(&page);
    assert_eq!(
        bob_github.github_login.as_deref(),
        Some(octo_bob.login.as_str())
    );
    assert_ne!(bob_github.account, bob_google.account);

    // Each method keeps reaching its own account.
    assert_eq!(google.signed_up(&browser, &ada), google_account);
    let page = browser.page();
    google.github_sign_in(&page, &octo);
    assert_eq!(google.account(&page), github_account);
}
