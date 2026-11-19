//! A new device linked through the pages on both sides, as through the CLI: certified by the linked
//! device, following its spaces and its personal space; neither command's output holds the code,
//! the confirmation or the page's secret, no argument of the program that opens the browser holds
//! the secret, and the opened files, readable only by their user, are gone with the pages once the
//! commands end.

use std::fs;
use std::path::Path;

use skald_crypto::check_valid_device;
use skald_proto::DeviceKind;

use crate::anchors::held;
use crate::at_terminal::as_agent;
use crate::linking::{LINK, OWN_DEVICE, RELAYED, forms, holds, typed};
use crate::machine::{DEVICES, Machine, NOT_LINKED};
use crate::pages::{
    NewOnPage, Opener, Opening, assert_gone, assert_holds_none, confirm_on_page, link_on_page,
    request,
};
use crate::world::{GLOBEX, World, create, plain_lines, run, spaces};

/// The host name of Ada's new machine, its certificate's `name`.
const LAPTOP: &str = "ada-laptop";

/// Fails unless `opening`, of a command on `machine` that still runs, names a file in the machine's
/// state directory that only its user may read, and no argument of the stand-in holds the page's
/// URL or its secret.
#[track_caller]
fn assert_handed_privately(what: &str, machine: &Machine, opening: &Opening) {
    let canonical = |path: &Path| fs::canonicalize(path).expect("a path that exists");
    assert!(
        canonical(&opening.file).starts_with(canonical(&machine.state_dir())),
        "{what}'s file {:?} is outside the state directory",
        opening.file
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(&opening.file)
            .expect("the file")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "{what}'s file is open to others: {mode:o}");
    }
    for arg in &opening.args {
        assert!(
            !arg.contains(&opening.query) && !arg.contains(&opening.url),
            "{what}: an argument holds the secret: {arg}"
        );
    }
}

/// Fails unless the page of `opening` answers a request with its secret and not one without it,
/// each answer not to be stored.
#[track_caller]
fn assert_takes_only_its_secret(what: &str, opening: &Opening) {
    let host = &opening.authority;
    let with = request(opening.address, "GET", &opening.target(), host, None);
    let with = with.unwrap_or_else(|| panic!("{what}'s page does not answer"));
    assert!(
        with.taken() && with.not_stored(),
        "{what}, with its secret: {with:?}"
    );
    let without = request(opening.address, "GET", &opening.path, host, None);
    if let Some(without) = without {
        assert!(
            !without.taken() && without.not_stored(),
            "{what}, without its secret: {without:?}"
        );
    }
}

/// Ada's account follows `globex`, which she created, and her personal space. Without its page, an
/// agent's `skald link` on her first machine refuses and sends nothing. Her new machine signs in as
/// an agent with `--browser`; on its page she answers that it is not the first device and reads its
/// code. As an agent, `skald link --browser` on her first machine opens its page, where she reads
/// the reminder, enters the code and confirms the new device's name and kind. Both commands exit
/// with success. The new device's certificate is issued by her first device and is valid against
/// her master key; the new device lists itself as linked, follows her spaces and starts from her
/// first device's anchors and personal space. Neither output holds the code, the confirmation or a
/// page's secret; the relay and the server's database hold no form of the code. Each file the
/// stand-in got was in its machine's state directory and its user's alone, no argument held the
/// secret, the pages' own requests got answers not to be stored, and once the commands end, the
/// files are gone and the pages answer nothing.
#[test]
fn a_new_device_links_through_the_pages_on_both_sides() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_179_101_001, "ada-links-on-pages", "ada-workstation");
    create(cloud, &ada, GLOBEX);
    let on_ada = spaces(&ada.machine);

    let before = cloud.relay.mark();
    let refused = run(&ada.machine, &as_agent(&[LINK]));
    assert!(!refused.success, "{refused:?}");
    let path = RELAYED.to_lowercase();
    assert!(
        !(cloud.relay.sent_since(before).iter()).any(|sent| sent.head.contains(&path)),
        "skald link without its page sent a request"
    );

    let mut new = NewOnPage::start(&world, &ada, LAPTOP, &[]);
    let signing_in = new.paged.opening.clone();
    assert_handed_privately("skald sign-in", &new.machine, &signing_in);
    assert_takes_only_its_secret("skald sign-in", &signing_in);
    let code = new.code();
    new.open_run(cloud, &ada);

    let opener = Opener::on(&ada.machine);
    let (linking, paged) = link_on_page(&world, &ada, &opener, OWN_DEVICE);
    let issuing = paged.opening.clone();
    assert_handed_privately("skald link", &ada.machine, &issuing);
    let linked = confirm_on_page(linking, &paged, &typed(&code), LAPTOP);
    assert!(linked.success, "skald link: {linked:?}");
    let signed_in = new.finish();
    assert!(signed_in.success, "the new device: {signed_in:?}");

    // What the commands printed and handed on.
    let codes = [code];
    let secrets = [signing_in.query.as_str(), issuing.query.as_str()];
    assert_holds_none("skald link", &linked.shown(), &codes, &secrets);
    assert_holds_none("skald sign-in", &signed_in.shown(), &codes, &secrets);
    paged.assert_answers_not_stored();
    new.paged.assert_answers_not_stored();
    assert_gone("skald link", &issuing);
    assert_gone("skald sign-in", &signing_in);
    let sent = cloud.relay.sent_bytes();
    let stored = cloud.rows().join("\n").into_bytes();
    for form in forms(&codes[0]) {
        assert!(!holds(&sent, &form), "a request holds the code as {form:?}");
        assert!(
            !holds(&stored, &form),
            "the server holds the code as {form:?}"
        );
    }

    // The certificate, the device list and the spaces.
    let certs: Vec<_> = (cloud.devices(&ada.token).into_iter())
        .filter_map(|device| device.cert)
        .collect();
    let valid = check_valid_device(&ada.master, &certs, &[], new.device);
    let body = valid
        .expect("a certificate valid against the master key")
        .body();
    assert_eq!(
        (body.issuer, body.account, body.kind, body.name.as_str()),
        (Some(ada.device), ada.account, DeviceKind::Client, LAPTOP),
        "the new device's certificate, issued by the first device"
    );
    let listed = run(&new.machine, &[DEVICES]);
    assert!(
        listed.success && !listed.shown().to_lowercase().contains(NOT_LINKED),
        "{listed:?}"
    );
    let on_new = spaces(&new.machine);
    assert_eq!(plain_lines(&on_new), plain_lines(&on_ada), "{on_new:?}");
    let objects = held(cloud, &ada.token);
    let by = |device| {
        (objects.iter().find(|object| object.device == device))
            .unwrap_or_else(|| panic!("no anchors object of {device}"))
    };
    let (first, laptop) = (by(ada.device), by(new.device));
    assert_eq!(
        laptop.anchors, first.anchors,
        "the revisions it starts from"
    );
    assert!(first.personal.is_some(), "no personal space");
    assert_eq!(laptop.personal, first.personal, "the personal space");
}
