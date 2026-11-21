//! As an agent in another space's folder, the warning of a space the device does not follow, as
//! its anchors disagree, waits for the person's run.

use crate::joined::{Account, anchors_off_one_chain, follows, next_run, warn_of};
use crate::kit::{assert_names_none, bind_client_a, received_as_agent};
use crate::world::{ACME, CLIENT_A};

/// The agent the second machine runs as.
const CLAUDE: &str = "claude";

/// Ada's first machine creates `clienta` before her second machine is linked, and the second
/// machine binds `~/work/clientA` to it. Then `acme`'s two anchors are off one chain
/// ([`anchors_off_one_chain`]). As an agent in `~/work/clientA`, the second machine's `skald recv`
/// names neither `acme` nor its ID; the person's next run does not follow `acme` and warns of it.
#[test]
fn as_an_agent_in_another_space_s_folder_the_warning_waits_for_the_person_s_run() {
    let account = Account::new(4_811_406_701, "ada-runs-her-agent", &[CLIENT_A]);
    let laptop = &account.laptop;
    let folder = bind_client_a(&laptop.machine);
    let acme = anchors_off_one_chain(&account);

    let agent = received_as_agent(&laptop.machine, &folder, CLAUDE);

    assert_names_none(&agent, &[ACME.to_owned(), acme.to_string()]);
    let (listed, said) = next_run(&laptop.machine);
    assert!(!follows(&listed, ACME), "acme is followed: {listed:?}");
    assert!(
        warn_of(&said, (ACME, acme)),
        "no warning of acme for the person: {said:?} {listed:?}"
    );
}
