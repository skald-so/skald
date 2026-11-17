//! Removing a method: the other one still signs in; the last one is never removed, also when two
//! removals come at once.

use std::sync::Barrier;
use std::thread;

use crate::common::delete_url;
use crate::common::spec::{operation_violations, spec};
use crate::common::{Response, device_flow::text};
use crate::{
    BOTH, ONE, Provider, Stand, Via, assert_refused, assert_removed, from_page, listed, path_of,
    remove, sorted,
};

/// Ada, with Google and GitHub, removes Google from a web session she signed in with GitHub: the
/// list keeps GitHub alone, with its login, and that session still works. A GitHub sign-in reaches
/// her account. No method's ID holds the subject its provider gives her.
#[test]
fn removed_method_is_gone_and_the_other_still_signs_in() {
    let stand = Stand::launch(&[]);
    let api = stand.api();
    let ada = stand.with_both(1, "ada-removes");
    let by_github = stand.signed_in(Via::GitHub(&ada.user));
    assert_eq!(by_github.who.account, ada.session.who.account);
    for method in &ada.methods {
        for subject in [ada.person.subject.clone(), ada.user.id.to_string()] {
            assert!(!method.id.contains(&subject), "{method:?} holds {subject}");
        }
    }

    let cookie = &by_github.cookie;
    assert_removed(&remove(&api, cookie, &ada.id(Provider::Google)));

    let left = listed(&api, cookie);
    assert_eq!(sorted(&left), [Provider::GitHub], "{left:#?}");
    assert_eq!(left[0].login.as_deref(), Some(ada.user.login.as_str()));
    assert_eq!(left[0].id, ada.id(Provider::GitHub), "the same method");
    assert!(api.web_works(cookie), "the GitHub session works");
    let again = stand.signed_in(Via::GitHub(&ada.user));
    assert_eq!(again.who.account, ada.session.who.account);
}

/// The account's last method is refused: of Ada's two methods, the first goes, the second stays,
/// and Google still signs in to her account.
#[test]
fn last_method_is_refused() {
    let stand = Stand::launch(&[]);
    let api = stand.api();
    let ada = stand.with_both(11, "ada-last");
    let cookie = &ada.session.cookie;

    assert_removed(&remove(&api, cookie, &ada.id(Provider::GitHub)));
    for what in ["the last method", "the last method again"] {
        assert_refused(&remove(&api, cookie, &ada.id(Provider::Google)), what);
    }

    let left = listed(&api, cookie);
    assert_eq!(sorted(&left), [Provider::Google], "{left:#?}");
    assert!(api.web_works(cookie));
    let again = stand.signed_in(Via::Google(&ada.person));
    assert_eq!(again.who.account, ada.session.who.account);
}

/// An account with two methods and two web sessions, one signed in with each, a round per account:
/// at once, the GitHub session removes Google and the Google session removes GitHub. One removal
/// is refused; the other method stays and signs in to the account.
#[test]
fn of_two_removals_at_once_one_is_refused() {
    let stand = Stand::launch(&[]);
    let api = stand.api();
    let spec = spec();
    for round in 0..3u64 {
        let ada = stand.with_both(20 + round, &format!("ada-race{round}"));
        let by_github = stand.signed_in(Via::GitHub(&ada.user));
        let removals = [
            (by_github.cookie.as_str(), ada.id(Provider::Google)),
            (ada.session.cookie.as_str(), ada.id(Provider::GitHub)),
        ];

        let start = Barrier::new(removals.len());
        let answers: Vec<Response> = thread::scope(|scope| {
            let running: Vec<_> = removals
                .iter()
                .map(|(cookie, id)| {
                    let (start, api) = (&start, &api);
                    scope.spawn(move || {
                        let url = format!("{}{}", api.base, path_of(id));
                        let version = skald_proto::PROTOCOL_VERSION.to_string();
                        let mut headers = from_page(api, cookie).to_vec();
                        headers.push((skald_proto::VERSION_HEADER, &version));
                        start.wait();
                        delete_url(&url, &headers)
                    })
                })
                .collect();
            running
                .into_iter()
                .map(|removal| removal.join().expect("a removal"))
                .collect()
        });

        for answer in &answers {
            let violations = operation_violations(&spec, "delete", ONE, answer);
            assert_eq!(violations, Vec::<String>::new(), "{}", text(answer));
        }
        let statuses: Vec<u16> = answers.iter().map(|answer| answer.status).collect();
        let through: Vec<usize> = (0..answers.len())
            .filter(|at| statuses[*at] == 204)
            .collect();
        let [through] = through.as_slice() else {
            panic!("round {round}: one removal of two: {statuses:?}");
        };
        let refused = statuses[1 - through];
        assert!((400..500).contains(&refused), "round {round}: {statuses:?}");

        // The method that stays is that of the session whose removal went through.
        let reached = if *through == 0 {
            stand.signed_in(Via::GitHub(&ada.user))
        } else {
            stand.signed_in(Via::Google(&ada.person))
        };
        assert_eq!(
            reached.who.account, ada.session.who.account,
            "round {round}"
        );
        let left = listed(&api, &reached.cookie);
        assert_eq!(left.len(), 1, "round {round}: {left:#?}");
        assert_ne!(sorted(&left), BOTH);
    }
}
