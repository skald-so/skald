use skald_oidc_stand_in::{Key, Person, Provider};

use super::common::browser::{
    Browser, CODE_FIELD, SIGN_IN_CORPORATE, SIGN_IN_GITHUB, SIGN_IN_GOOGLE,
};
use super::common::google::HOST;
use super::common::spa::{Spa, wait_for_cabinet, wait_until};
use super::{Site, TestDb};

#[test]
fn the_first_corporate_cabinet_guides_machine_sign_in() {
    let browser = Browser::launch();
    let key = Key::rsa();
    let person = Person::new("new-worker", "new-worker@example.test");

    for github in [true, false] {
        let db = TestDb::create();
        let provider = Provider::start(std::slice::from_ref(&key));
        let spa = Spa::start(&browser, HOST);
        let site = Site::with_web(&db, &provider, None, github, Some(&spa.url));
        spa.serve(&browser, site.server.base());
        provider.put_person(person.clone());
        let page = browser.page();
        page.goto(&provider.sign_in_url(&person.subject));
        page.goto(&spa.at("/"));
        let corporate = "role=link[name=/continue with corporate/i]";
        wait_until(&page, "corporate sign-in choice", |page| {
            page.count(corporate) == 1
        });
        let visit = page.click_settled(corporate);
        assert_eq!(visit.url, spa.at("/"), "{visit:?}");
        wait_for_cabinet(&page, "new-worker");
        wait_until(&page, "empty space and machines", |page| {
            let text = page.text();
            text.contains("No space yet.") && text.contains("No machines yet.")
        });
        let text = page.text();
        page.goto(&spa.at("/access"));
        wait_until(&page, "Corporate in the Access session row", |page| {
            page.count("role=row >> text=Corporate") == 1
        });
        let visit = page.goto_settled(&spa.at("/device"));
        assert_eq!(visit.status, Some(200), "{visit:?}");
        assert_eq!(page.count(CODE_FIELD), 1, "{}", page.text());
        assert!(page.text().contains("new-worker"), "{}", page.text());

        let signed_out = browser.page();
        let visit = signed_out.goto_settled(&spa.at("/device"));
        assert_eq!(visit.status, Some(200), "{visit:?}");
        assert_eq!(
            signed_out.count(SIGN_IN_CORPORATE),
            1,
            "{}",
            signed_out.text()
        );
        assert_eq!(signed_out.count(SIGN_IN_GITHUB), usize::from(github));
        assert_eq!(signed_out.count(SIGN_IN_GOOGLE), 0);

        for follow_up in ["to make your personal space", "to list it here"] {
            assert!(text.contains(follow_up), "{text}");
        }
        assert!(
            !text.contains("You need a linked GitHub sign-in method."),
            "{text}"
        );
    }
}
