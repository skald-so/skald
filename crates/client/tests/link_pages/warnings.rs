//! As an agent, neither command's output names a space, its warnings included: they show on the
//! page and wait for the person's run, as other runs of an agent keep them.

use std::thread;
use std::time::{Duration, Instant};

use crate::pages::NewOnPage;
use crate::world::{GLOBEX, WARNING, World, create, run, space_id, spaces, warnings};

/// The host name of Ada's new machine, its certificate's `name`.
const LAPTOP: &str = "ada-laptop";

/// Ada's account follows `globex` and her personal space. Her new machine signs in as an agent with
/// `--browser` and is linked on the page of an agent's `skald link --browser` on her first machine,
/// which adds it to her channels and so warns of the new device, naming her space. The page of
/// `skald link` shows that warning; neither command's output holds a warning or names `globex` or
/// its ID; the person's next `skald recv` on the first machine shows the warning.
#[test]
fn an_agent_s_output_names_no_space_and_its_warnings_wait_for_the_person() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_179_601_001, "ada-warned-on-pages", "ada-workstation");
    create(cloud, &ada, GLOBEX);
    let globex = space_id(cloud, &ada.token, GLOBEX)
        .to_string()
        .to_lowercase();
    spaces(&ada.machine);

    let mut new = NewOnPage::start(&world, &ada, LAPTOP, &[]);
    let (paged, linked) = new.link_from(&world, &ada);
    let signed_in = new.finish();
    assert!(signed_in.success, "the new device: {signed_in:?}");

    let id = new.device.to_string().to_lowercase();
    let of_new = |line: &str| line.contains(WARNING) && line.contains(&id);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let on_page = paged.page.text().to_lowercase();
        if on_page.lines().any(of_new) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the page of skald link shows no warning of the new device: {on_page}"
        );
        thread::sleep(Duration::from_millis(100));
    }
    for (what, finished) in [("skald link", &linked), ("skald sign-in", &signed_in)] {
        let shown = finished.shown().to_lowercase();
        assert!(!shown.contains(WARNING), "{what} shows a warning: {shown}");
        for named in [GLOBEX, &globex] {
            assert!(!shown.contains(named), "{what} names {named}: {shown}");
        }
    }
    let person = run(&ada.machine, &["recv"]);
    assert!(
        warnings(&person).iter().any(|line| of_new(line)),
        "the person's run shows no warning of the new device: {person:?}"
    );
}
