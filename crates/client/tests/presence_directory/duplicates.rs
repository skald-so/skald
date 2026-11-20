//! Two running agents with one address within an owner: the list marks an address that two devices
//! of one account list, neither as `offline`, and a device that registers an agent whose address
//! another device of its account lists as running warns the agent's next result and its person's
//! next run, once.

use serde_json::Value;
use skald_proto::AgentState;

use crate::client::{CLIENT, session_in};
use crate::kit::received;
use crate::machine::Finished;
use crate::senders::LAPTOP;
use crate::values::{AVAILABLE, Directory, OFFLINE, listed_json, object_of, status};
use crate::world::{AGENT, now, warnings};

/// The agent's name both devices list.
const API: &str = "api";

/// The certificate `name` of Bob's second machine.
const BOB_LAPTOP: &str = "bob-laptop";

/// Whether `object`, an agent's object of `skald agents --json`, is marked.
#[track_caller]
fn marked(object: &Value) -> bool {
    object["duplicate"]
        .as_bool()
        .unwrap_or_else(|| panic!("no duplicate: {object}"))
}

/// Bob's first device lists his agents `api` available and `claude-d8u3fy` available; his second
/// machine, played, lists `api` busy. `skald agents --json` marks both objects of `api`, not
/// `claude-d8u3fy`. Then the second machine lists `api` offline: neither is marked.
#[test]
fn marks_an_address_two_devices_of_one_account_list_running_until_one_is_offline() {
    let mut directory = Directory::new(
        [4_214_009_001, 4_214_009_002],
        ["ada-sees-two-apis", "bob-runs-api-twice"],
    );
    let mut second = directory.second_machine(true, BOB_LAPTOP);
    let at = directory.at();
    let since = now() - 120;
    let (api, other) = (
        directory.bob_agent(API),
        directory.bob_agent("claude-d8u3fy"),
    );
    let first_lists = vec![
        status(&api, AgentState::Available, since, (None, None)),
        status(&other, AgentState::Available, since, (None, None)),
    ];
    directory.bob_uploads((at, 1), first_lists);
    let busy = status(&api, AgentState::Busy, since, (None, None));
    directory.uploads(&mut second, (at, 1), vec![busy]);
    let bob_device = directory.bob().device();

    let running = listed_json(directory.machine(), directory.folder());
    let offline = status(&api, AgentState::Offline, now(), (None, None));
    directory.uploads(&mut second, (at, 2), vec![offline]);
    let one_offline = listed_json(directory.machine(), directory.folder());

    assert!(marked(object_of(&running, &api, bob_device)));
    assert!(marked(object_of(&running, &api, second.device())));
    assert!(!marked(object_of(&running, &other, bob_device)));
    assert!(!marked(object_of(&one_offline, &api, bob_device)));
    assert!(!marked(object_of(&one_offline, &api, second.device())));
}

/// The lines of `shown` that hold `address`, as `<owner>/<agent>`, and `device`.
fn naming(shown: &str, address: &str, device: &str) -> Vec<String> {
    (shown.lines())
        .filter(|line| line.contains(address) && line.contains(device))
        .map(str::to_owned)
        .collect()
}

/// The warning lines of `run`, a run of the person's, that hold `address` and `device`.
fn warned(run: &Finished, address: &str, device: &str) -> Vec<String> {
    (warnings(run).into_iter())
        .filter(|line| line.contains(address) && line.contains(device))
        .collect()
}

/// Ada's second machine, played, lists her agent `api` in `state`. Then a session of `claude-code`
/// starts in `~/work/acme` as `--agent api`, which registers `api` on Ada's machine; it calls
/// `inbox` twice, and the person runs `skald recv` twice, then `skald agents --json`. Gives the
/// lines of each run that name `<ada>/api` and `ada-laptop`: of the results, any; of the person's
/// runs, warnings; and the object of the laptop's `api`.
fn registers_api_while_the_laptop_lists_it(
    ids: [u64; 2],
    logins: [&str; 2],
    state: AgentState,
) -> ([Vec<String>; 4], Value) {
    let mut directory = Directory::new(ids, logins);
    let mut laptop = directory.second_machine(false, LAPTOP);
    let at = directory.at();
    let listed = status(&directory.ada_agent(API), state, now() - 120, (None, None));
    directory.uploads(&mut laptop, (at, 1), vec![listed]);
    let address = format!("{}/{API}", directory.ada_handle().to_lowercase());
    let (machine, folder) = (directory.machine(), directory.folder());

    let mut session = session_in(machine, folder, &[AGENT, API], CLIENT);
    let results = [session.inbox(), session.inbox()];
    let runs = [received(machine), received(machine)];
    let objects = listed_json(machine, folder);

    let [first, next] = results.map(|shown| naming(&shown, &address, LAPTOP));
    let [first_run, next_run] = runs.map(|run| warned(&run, &address, LAPTOP));
    let object = object_of(&objects, &directory.ada_agent(API), laptop.device()).clone();
    ([first, next, first_run, next_run], object)
}

/// With Ada's second machine listing `api` available, the session's first `inbox` has one line
/// that names `<ada>/api` and `ada-laptop`, and the next none; the person's next `skald recv` warns
/// of it once, and the one after does not. The list shows the laptop's `api` available.
#[test]
fn registering_an_agent_another_device_runs_warns_its_result_and_person_once() {
    let ([first, next, first_run, next_run], laptops) = registers_api_while_the_laptop_lists_it(
        [4_214_010_001, 4_214_010_002],
        ["ada-starts-a-second-api", "bob-watches-ada-twice"],
        AgentState::Available,
    );

    assert_eq!(first.len(), 1, "the first result: {first:?}");
    assert_eq!(next, Vec::<String>::new(), "the next result");
    assert_eq!(first_run.len(), 1, "the person's next run: {first_run:?}");
    assert_eq!(next_run, Vec::<String>::new(), "the person's run after");
    assert_eq!(laptops["state"].as_str(), Some(AVAILABLE), "{laptops}");
}

/// With Ada's second machine listing `api` offline, neither result nor run names `<ada>/api` with
/// `ada-laptop`, and the list shows the laptop's `api` offline, not marked.
#[test]
fn registering_an_agent_another_device_lists_offline_warns_of_nothing() {
    let (found, laptops) = registers_api_while_the_laptop_lists_it(
        [4_214_011_001, 4_214_011_002],
        ["ada-starts-api-again", "bob-watches-ada-once"],
        AgentState::Offline,
    );

    for (lines, what) in found
        .iter()
        .zip(["first result", "next result", "run", "next run"])
    {
        assert_eq!(lines, &Vec::<String>::new(), "the {what}");
    }
    assert_eq!(laptops["state"].as_str(), Some(OFFLINE), "{laptops}");
    assert!(!marked(&laptops));
}
