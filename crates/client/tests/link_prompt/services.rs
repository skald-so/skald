//! The commands of service agents and `skald sign-out` reach the server too: each names the
//! waiting device.

use crate::fixtures::fronted;
use crate::machine::SIGN_OUT;
use crate::prompt::{LAPTOP, assert_prompted, waiting_device};
use crate::service_kit::{
    Acme, BOT, CHANNEL, CREATE_SERVICE_TOKEN, KEEP_EARLIER, REVOKE_SERVICE_TOKEN, SERVICE_TOKENS,
    create_agent, listed_ids,
};
use crate::world::{ACME, run};

/// Ada's laptop waits for linking. Her first machine's `skald create-service-agent`,
/// `skald create-service-token`, `skald service-tokens`, `skald revoke-service-token` and, last,
/// `skald sign-out` each succeed and say once that the server reports, unverified, the `client`
/// device `ada-laptop` waiting and that `skald link` links it.
#[test]
fn every_service_command_and_sign_out_name_the_waiting_device_once() {
    let (world, _front) = fronted();
    let ada = world.sign_up(
        4_178_701_001,
        "ada-runs-service-commands",
        "ada-workstation",
    );
    let acme = Acme::create(&world.cloud, &ada);
    let _new = waiting_device(&world, &ada, LAPTOP);
    let default = acme.default.to_string();

    let created = create_agent(&ada.machine, ACME, BOT, &[acme.default], &[]);
    let token = run(
        &ada.machine,
        &[
            CREATE_SERVICE_TOKEN,
            ACME,
            BOT,
            CHANNEL,
            &default,
            KEEP_EARLIER,
        ],
    );
    let listed = run(&ada.machine, &[SERVICE_TOKENS, ACME]);
    let ids = listed_ids(&world.cloud, &ada, acme.id);
    let first = ids.first().expect("a token of the service agent");
    let revoked = run(&ada.machine, &[REVOKE_SERVICE_TOKEN, ACME, first]);
    let signed_out = run(&ada.machine, &[SIGN_OUT]);

    for (what, finished) in [
        ("create-service-agent", created),
        ("create-service-token", token),
        ("service-tokens", listed),
        ("revoke-service-token", revoked),
        ("sign-out", signed_out),
    ] {
        assert!(finished.success, "{what}: {finished:?}");
        assert_prompted(&finished.shown(), LAPTOP);
    }
}
