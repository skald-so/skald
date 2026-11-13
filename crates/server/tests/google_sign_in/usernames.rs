//! The username of an account a Google sign-in creates: made from the email's local part, as any
//! username is made from a text (`usernames.rs`), numbered when taken.

use super::Google;
use super::common::accounts::{NAME_MAX, assert_valid};
use super::common::browser::Browser;

#[test]
fn username_comes_from_the_email_local_part() {
    let browser = Browser::launch();
    let google = Google::start();
    for (subject, email, username) in [
        (
            "110248495921238986500",
            "Ada.Lovelace@example.com",
            "ada-lovelace",
        ),
        (
            "110248495921238986501",
            "grace_hopper+skald@example.org",
            "grace-hopper-skald",
        ),
        ("110248495921238986502", "linus@kernel.example", "linus"),
    ] {
        let person = google.person(subject, email);
        assert_eq!(
            google.signed_up(&browser, &person).username,
            username,
            "{email}"
        );
    }
}

#[test]
fn taken_or_reserved_username_gets_the_first_free_number() {
    let browser = Browser::launch();
    let google = Google::start();
    for (subject, email, username) in [
        ("110248495921238986510", "ada@one.example", "ada"),
        ("110248495921238986511", "ada@two.example", "ada-2"),
        ("110248495921238986512", "ADA@three.example", "ada-3"),
        ("110248495921238986513", "admin@example.com", "admin-2"),
        ("110248495921238986514", "root@example.com", "root-2"),
    ] {
        let person = google.person(subject, email);
        assert_eq!(
            google.signed_up(&browser, &person).username,
            username,
            "{email}"
        );
    }
}

#[test]
fn local_part_that_leaves_no_valid_username_still_gives_one() {
    let browser = Browser::launch();
    let google = Google::start();
    let long = "a".repeat(70);
    let mut usernames = Vec::new();
    for (at, local) in ["___", "-.-", "用户", "....", "-", long.as_str()]
        .into_iter()
        .enumerate()
    {
        let person = google.person(
            &format!("11024849592123898652{at}"),
            &format!("{local}@example.com"),
        );
        let username = google.signed_up(&browser, &person).username;
        assert_valid(&username);
        usernames.push(username);
    }
    assert_eq!(
        usernames.last().map(String::as_str),
        Some("a".repeat(NAME_MAX).as_str()),
        "a long local part is cut to a name's length"
    );
    let mut distinct = usernames.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), usernames.len(), "{usernames:?}");
}
