//! A code's attempts and its end on the pages, as at a terminal (`membership.md`, "Link code"):
//! three wrong codes entered on the linked device's page end the new device's code, whose page says
//! it failed 3 times and that someone else may have tried and shows a fresh code only when its
//! person asks; the linked device's next page first asks whether the new device shows a new code. A
//! code ends once its lifetime is over, here sooner, its page saying it expired, and a fresh one
//! links.

use std::thread;
use std::time::Duration;

use crate::linking::{
    EXPIRED, HELLO, LIFETIME_SETTING, NEW_CODE, OWN_DEVICE, THREE_TIMES, codes_in, failures,
    other_code, says, typed, until_not_waiting, waiting,
};
use crate::pages::{
    NewOnPage, Opener, YES_BUTTON, confirm_on_page, link_failing_on_page, link_on_page,
};
use crate::world::World;

/// The host name of Ada's new machine.
const LAPTOP: &str = "ada-laptop";

/// How long a test watches for what must not happen.
const QUIET: Duration = Duration::from_secs(3);

/// Three wrong codes entered on the page of Ada's first machine each fail a run, and that
/// `skald link` exits failing; the new device's page then says its code failed 3 times and that
/// someone else may have tried, and it shows no fresh code and sends no hello. Asked on its page,
/// it shows a fresh code; the next page of `skald link` first asks whether the new device shows a
/// new code, and answered yes, links with it.
#[test]
fn three_wrong_codes_on_the_page_end_the_code_and_a_fresh_one_comes_only_when_asked() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_179_201_001, "ada-mistypes-on-pages", "ada-workstation");
    let opener = Opener::on(&ada.machine);
    let mut new = NewOnPage::start(&world, &ada, LAPTOP, &[]);
    let code = new.code();

    for failed in 1..=3 {
        new.open_run(cloud, &ada);
        link_failing_on_page(&world, &ada, &opener, &typed(&other_code(&code)));
        new.wait_for(&format!("{failed} failed runs"), |text| {
            failures(text) >= failed
        });
    }
    new.wait_for("that the code failed 3 times", |text| {
        says(text, &THREE_TIMES)
    });
    thread::sleep(QUIET);
    let mut shown = codes_in(&new.paged.page.text());
    shown.dedup();
    assert!(
        shown.iter().all(|shown| *shown == code),
        "a fresh code unasked: {shown:?}"
    );
    assert_eq!(cloud.count_sent(new.started, HELLO), 3, "a hello unasked");
    assert!(!waiting(cloud, &ada, new.device), "a run still open");

    let fresh = new.fresh_code();
    new.open_run(cloud, &ada);
    let (mut resumed, paged) = link_on_page(&world, &ada, &opener, NEW_CODE);
    paged.press(YES_BUTTON);
    paged.wait_for(&mut resumed, "the reminder", |text| {
        says(text, &[OWN_DEVICE])
    });
    let resumed = confirm_on_page(resumed, &paged, &typed(&fresh), LAPTOP);
    assert!(resumed.success, "{resumed:?}");
    let signed_in = new.finish();
    assert!(signed_in.success, "the new device: {signed_in:?}");
}

/// The new device's code lives 30 seconds here: its page then says the code expired and its run
/// ends, and `skald link` with that code on the linked device's page exits failing. Asked on its
/// page, the new device shows a fresh code, with which it links.
#[test]
fn a_code_on_the_page_expires_and_a_fresh_one_links() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_179_201_011, "ada-waits-on-a-page", "ada-workstation");
    let opener = Opener::on(&ada.machine);
    let mut new = NewOnPage::start(&world, &ada, LAPTOP, &[(LIFETIME_SETTING, "30")]);
    let code = new.code();

    new.wait_for("that the code expired", |text| says(text, &[EXPIRED]));
    until_not_waiting(cloud, &ada, new.device);
    link_failing_on_page(&world, &ada, &opener, &typed(&code));

    let fresh = new.fresh_code();
    // Its first run ended with the code; this is its second.
    new.runs = 1;
    new.open_run(cloud, &ada);
    let (linking, paged) = link_on_page(&world, &ada, &opener, OWN_DEVICE);
    let linked = confirm_on_page(linking, &paged, &typed(&fresh), LAPTOP);
    assert!(linked.success, "{linked:?}");
    let signed_in = new.finish();
    assert!(signed_in.success, "the new device: {signed_in:?}");
}
