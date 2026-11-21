//! `skald sign-in` opens the verification page with its code filled in, through a file only its
//! user reads, and prints one fallback line with the link.

use std::fs;

use crate::browser::{APPROVE, CODE_FIELD, SIGN_IN_GITHUB, SUBMIT};
use crate::cloud::shown_grant;
use crate::machine::{SERVER, SIGN_IN};
use crate::pages::file_url;
use crate::world::World;
use crate::{assert_only_the_line, code_forms, complete_uri, opened_file, openings, with_opener};

/// Ada's new machine runs `skald sign-in`. The stand-in opener gets one argument, a file readable
/// only by its user, and no argument holds the code; the run prints one line, holding the link.
/// Signed in to the server in her browser, Ada opens that file: it takes her to the verification
/// page with this sign-in's code filled in, where she approves it without typing it, and the
/// machine signs in to her account. Once the command ends, the file is gone.
#[test]
fn sign_in_opens_the_verification_page_with_its_code_through_a_file_only_its_user_reads() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = cloud.user(4_312_640_001, "ada-opened-standin");
    let (machine, opener) = with_opener("ada-laptop");

    let mut run = machine.start(&[SIGN_IN, SERVER, &cloud.url]);
    let grant = shown_grant(cloud, &mut run, 0);
    let opened = openings(&opener, 1, || {
        run.assert_running();
        run.shown()
    });
    let file = opened_file(&opened[0]);
    for args in &opened {
        for arg in args {
            for form in code_forms(&grant.user_code) {
                assert!(!arg.contains(&form), "an argument holds the code: {arg:?}");
            }
        }
    }
    assert!(file.is_file(), "the opened file {file:?} is missing");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(&file)
            .expect("the opened file")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "{file:?} is readable by others: {mode:o}");
    }

    let page = world.browser.page();
    let visit = page.goto(&cloud.github.sign_in_url(ada.id));
    assert_eq!(visit.status, Some(200), "stand-in sign-in: {visit:?}");
    let visit = page.goto_settled(&grant.verification_uri);
    assert!(visit.error.is_none(), "the verification page: {visit:?}");
    if page.count(CODE_FIELD) == 0 {
        page.click_settled(&format!("{SIGN_IN_GITHUB} >> nth=0"));
    }
    assert_only_the_line(&run.shown(), &complete_uri(cloud));

    let visit = page.goto_settled(&file_url(&file));
    assert!(visit.error.is_none(), "the opened file: {visit:?}");
    assert_eq!(
        page.values(CODE_FIELD),
        vec![grant.user_code.clone()],
        "the page has not this sign-in's code filled in: {visit:?}: {}",
        page.text()
    );
    if page.count(APPROVE) == 0 {
        page.click(SUBMIT);
    }
    let approved = page.click(APPROVE);
    assert_eq!(
        approved.status.map(|status| status / 100),
        Some(2),
        "approve: {approved:?}"
    );
    let finished = run.finish();

    assert!(finished.success, "{finished:?}");
    assert_eq!(cloud.login_of(&cloud.last_token()), ada.login);
    assert!(!file.exists(), "the command left {file:?}");
}
