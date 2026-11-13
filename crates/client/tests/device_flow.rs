//! `skald sign-in` against stubs of the device flow's endpoints: the answers a `skald-server` gives
//! only in time or in a fault, a stored session on another server, the default state directory, a
//! device ID the server has taken and a name given to a named device. Then `skald devices` without
//! a session, with a token for another server and with the server's error answers.

#[allow(dead_code)]
mod common;
#[allow(dead_code)]
#[path = "sign_in/machine.rs"]
mod machine;

use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use common::{CLIENT, Stub, response};
use machine::{DEVICES, FIRST_DEVICE, KEY_STORAGE, Machine, NAME, SERVER, SIGN_IN, STATE_DIR, YES};

const TOKEN: &str = "tok-stub-0123456789abcdefghijkl";
const LOGIN: &str = "ada-stub-login";

/// Device authorization's answer, with codes that live `expires_in` seconds.
fn grant(expires_in: u64) -> String {
    let body = format!(
        r#"{{"device_code":"dc-stub-0123456789abcdefghijkl","user_code":"BCDF-GHJK","verification_uri":"http://stub.test/device","verification_uri_complete":"http://stub.test/device?user_code=BCDF-GHJK","expires_in":{expires_in},"interval":1}}"#
    );
    response("200 OK", "application/json", &body)
}

fn token() -> String {
    let body = format!(r#"{{"access_token":"{TOKEN}","token_type":"Bearer"}}"#);
    response("200 OK", "application/json", &body)
}

fn oauth_error(error: &str) -> String {
    response(
        "400 Bad Request",
        "application/json",
        &format!(r#"{{"error":"{error}"}}"#),
    )
}

fn session() -> String {
    let body =
        format!(r#"{{"account":"0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b","github_login":"{LOGIN}"}}"#);
    response("200 OK", "application/json", &body)
}

fn unauthorized() -> String {
    response(
        "401 Unauthorized",
        "application/problem+json",
        r#"{"type":"about:blank","title":"Unauthorized","status":401,"detail":"No session."}"#,
    )
}

/// The answer to a device's registration: taken, without a certificate.
fn registered() -> String {
    response(
        "201 Created",
        "application/json",
        r#"{"device":"0190a1b2-c3d4-8e5f-8a9b-0c1d2e3f4a5b","cert":null}"#,
    )
}

/// A stub that signs every machine in: each code is approved at its first poll, and each device's
/// registration is taken.
fn approving(path: &str) -> String {
    match path {
        "/auth/device" => grant(60),
        "/auth/token" => token(),
        "/session" => session(),
        "/devices" => registered(),
        _ => response("404 Not Found", "text/plain", ""),
    }
}

/// A stub on which every code is denied.
fn denying(path: &str) -> String {
    match path {
        "/auth/device" => grant(60),
        "/auth/token" => oauth_error("access_denied"),
        _ => response("404 Not Found", "text/plain", ""),
    }
}

/// The answer to a device's registration when the server has its ID with other keys or `name`, or
/// on another account.
fn device_taken() -> String {
    response(
        "409 Conflict",
        "application/problem+json",
        r#"{"type":"https://skald.so/problems/device-taken","title":"Device ID taken","status":409,"detail":"Taken."}"#,
    )
}

/// The requests to `path` among `stub`'s.
fn requests_to(stub: &Stub, path: &str) -> Vec<String> {
    stub.requests()
        .into_iter()
        .filter(|head| head.split(' ').nth(1) == Some(path))
        .collect()
}

#[test]
fn device_flow_requests_state_the_protocol_version() {
    let stub = Stub::start(approving);
    let machine = Machine::new();

    let finished = machine.sign_in(&stub.url).finish();

    assert!(finished.success, "{finished:?}");
    assert!(finished.shown().contains(LOGIN), "{finished:?}");
    let version = format!(
        "\nskald-protocol-version: {}",
        skald_proto::PROTOCOL_VERSION
    );
    for path in ["/auth/device", "/auth/token", "/session"] {
        let requests = requests_to(&stub, path);
        assert_eq!(requests.len(), 1, "{path}: {requests:?}");
        assert!(requests[0].contains(&version), "{}", requests[0]);
    }
}

#[test]
fn session_the_server_refuses_is_dropped_and_a_new_code_asked_for() {
    static SESSIONS: AtomicUsize = AtomicUsize::new(0);
    static POLLS: AtomicUsize = AtomicUsize::new(0);
    // The first sign-in gets a session; then the server refuses it and denies the new code.
    let stub = Stub::start(|path| match path {
        "/auth/token" if POLLS.fetch_add(1, Ordering::SeqCst) > 0 => oauth_error("access_denied"),
        "/session" if SESSIONS.fetch_add(1, Ordering::SeqCst) > 0 => unauthorized(),
        path => approving(path),
    });
    let machine = Machine::new();
    let first = machine.sign_in(&stub.url).finish();
    assert!(first.success, "{first:?}");

    let again = machine.sign_in(&stub.url).finish();

    assert!(!again.success, "{again:?}");
    assert!(again.shown().contains("denied"), "{again:?}");
    assert_eq!(requests_to(&stub, "/auth/device").len(), 2);
    assert_eq!(
        machine.files_holding(TOKEN),
        Vec::<std::path::PathBuf>::new()
    );
}

#[test]
fn session_on_another_server_is_never_sent_to_this_one() {
    let first = Stub::start(approving);
    let other = Stub::start(denying);
    let machine = Machine::new();
    assert!(machine.sign_in(&first.url).finish().success);

    let finished = machine.sign_in(&other.url).finish();

    assert!(!finished.success, "{finished:?}");
    let sent = other.requests();
    assert!(
        sent.iter().all(|head| !head.contains("\nauthorization:")),
        "{sent:?}"
    );
    assert_eq!(requests_to(&other, "/auth/device").len(), 1);
    // The denied code leaves the session on the first server as it was.
    let again = machine.sign_in(&first.url).finish();
    assert!(again.success, "{again:?}");
    assert_eq!(requests_to(&first, "/auth/device").len(), 1);
    assert_eq!(machine.files_holding(TOKEN).len(), 1);
}

#[test]
fn code_still_pending_at_its_lifetime_stops_the_sign_in() {
    let stub = Stub::start(|path| match path {
        "/auth/device" => grant(2),
        _ => oauth_error("authorization_pending"),
    });
    let machine = Machine::new();
    let started = Instant::now();

    let finished = machine.sign_in(&stub.url).finish();

    assert!(!finished.success, "{finished:?}");
    assert!(finished.shown().contains("expired"), "{finished:?}");
    assert!(started.elapsed() < Duration::from_secs(10), "{finished:?}");
    assert_eq!(
        machine.files_holding(TOKEN),
        Vec::<std::path::PathBuf>::new()
    );
}

#[test]
fn other_errors_of_the_token_endpoint_stop_the_sign_in() {
    let stub = Stub::start(|path| match path {
        "/auth/device" => grant(60),
        _ => oauth_error("invalid_grant"),
    });

    fails_saying(&stub, "invalid_grant");

    assert_eq!(requests_to(&stub, "/auth/token").len(), 1);
}

/// Signs a new machine in on `stub`; fails unless the run fails and its stderr holds `part`.
fn fails_saying(stub: &Stub, part: &str) {
    let finished = Machine::new().sign_in(&stub.url).finish();
    assert!(!finished.success, "{finished:?}");
    assert!(finished.stderr.contains(part), "{finished:?}");
}

/// Runs `skald sign-in` against `server` with the file key storage, the environment `env` and
/// nothing else; gives whether it succeeded and its stderr.
fn sign_in_with(server: &str, env: &[(&str, &std::path::Path)]) -> (bool, String) {
    let output = Command::new(CLIENT)
        .args([SIGN_IN, SERVER, server])
        .env_clear()
        .env(KEY_STORAGE, "file")
        .envs(env.iter().copied())
        .output()
        .expect("run skald");
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    (output.status.success(), stderr)
}

#[test]
fn state_directory_defaults_to_the_platforms() {
    let stub = Stub::start(approving);
    let machine = Machine::new();
    let home = machine.root().join("home");

    let (success, stderr) = sign_in_with(&stub.url, &[("HOME", &home)]);

    assert!(success, "{stderr}");
    let dir = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/skald")
    } else {
        home.join(".local/state/skald")
    };
    let secrets = fs::read_to_string(dir.join("secrets.json")).expect("the secrets file");
    assert!(secrets.contains(TOKEN), "{secrets}");
    assert_eq!(machine.files_holding(TOKEN), vec![dir.join("secrets.json")]);

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let state = machine.root().join("xdg-state");
        let (success, stderr) =
            sign_in_with(&stub.url, &[("HOME", &home), ("XDG_STATE_HOME", &state)]);
        assert!(success, "{stderr}");
        assert!(state.join("skald/secrets.json").is_file());
    }

    let (success, stderr) = sign_in_with(&stub.url, &[]);
    assert!(!success);
    assert!(stderr.contains(STATE_DIR), "{stderr}");
}

/// The requests among `stub`'s that carry a session token.
fn authorized(stub: &Stub) -> usize {
    stub.requests()
        .iter()
        .filter(|head| head.contains("\nauthorization:"))
        .count()
}

#[test]
fn token_goes_only_to_the_server_it_came_from_even_when_the_state_names_another() {
    let first = Stub::start(approving);
    let second = Stub::start(approving);
    let machine = Machine::new();
    assert!(machine.sign_in(&first.url).finish().success);
    assert!(machine.sign_in(&second.url).finish().success);
    // As if the state, written after the secrets, still named the first server.
    let state = rusqlite::Connection::open(machine.state_dir().join("state.sqlite3")).unwrap();
    state
        .execute("UPDATE session SET server = ?1", [&first.url])
        .unwrap();
    drop(state);
    let before = authorized(&first);

    let finished = machine.sign_in(&first.url).finish();

    assert!(finished.success, "{finished:?}");
    assert_eq!(
        authorized(&first),
        before + 1,
        "only the new session's token"
    );
    assert_eq!(requests_to(&first, "/auth/device").len(), 2);
}

#[test]
fn device_id_the_server_has_taken_stops_the_sign_in() {
    let stub = Stub::start(|path| match path {
        "/devices" => device_taken(),
        path => approving(path),
    });

    fails_saying(&stub, "other keys or name");
}

#[test]
fn name_of_a_named_device_does_not_change_and_it_registers_once() {
    let stub = Stub::start(approving);
    let machine = Machine::new();
    let sign_in = |name: &str| {
        let args = [SIGN_IN, SERVER, &stub.url, NAME, name];
        machine.start(&args).finish()
    };
    let first = sign_in("Ada's laptop");
    assert!(first.success, "{first:?}");
    assert_eq!(requests_to(&stub, "/devices").len(), 1, "registered");

    let renamed = sign_in("Ada's desktop");

    assert!(!renamed.success, "{renamed:?}");
    assert!(renamed.stderr.contains("named"), "{renamed:?}");
    assert_eq!(
        requests_to(&stub, "/session").len(),
        1,
        "the run that renames asks the server"
    );
    let again = sign_in("Ada's laptop");
    assert!(again.success, "{again:?}");
    assert_eq!(
        requests_to(&stub, "/devices").len(),
        1,
        "registered once only"
    );
}

/// A stub that signs every machine in as the account's first device: the account has no master key
/// and takes the device's; `/devices` answers as `devices` gives.
fn certifying(path: &str, devices: fn() -> String) -> String {
    match path {
        "/account/master" => response("200 OK", "application/json", r#"{"master":null}"#),
        "/devices" => devices(),
        path => approving(path),
    }
}

/// Signs `machine` in on `stub` as the account's first device, which holds its certificate then.
fn sign_in_first(machine: &Machine, stub: &Stub) {
    let args = [SIGN_IN, SERVER, &stub.url, FIRST_DEVICE, YES];
    let finished = machine.start(&args).finish();
    assert!(finished.success, "{finished:?}");
    assert!(
        finished.stdout.contains("holds its certificate"),
        "{finished:?}"
    );
}

/// Runs `skald devices` on `machine` with `args` before it; fails unless the run fails and its
/// stderr holds each of `parts`.
fn devices_fail_saying(machine: &Machine, args: &[&str], parts: &[&str]) {
    let args: Vec<&str> = args.iter().copied().chain([DEVICES]).collect();
    let finished = machine.start(&args).finish();
    assert!(!finished.success, "{finished:?}");
    for part in parts {
        assert!(finished.stderr.contains(part), "{part:?}: {finished:?}");
    }
}

const NOT_SIGNED_IN: [&str; 2] = ["not signed in", "skald sign-in"];

#[test]
fn devices_without_a_session_ask_to_sign_in() {
    let machine = Machine::new();
    let missing = machine.root().join("missing");
    devices_fail_saying(
        &machine,
        &["--state-dir", missing.to_str().expect("a UTF-8 path")],
        &NOT_SIGNED_IN,
    );
    assert!(!missing.exists(), "the run made the state directory");

    devices_fail_saying(&machine, &[], &NOT_SIGNED_IN);

    // A denied code leaves the state without a session.
    let denied = Stub::start(denying);
    assert!(!machine.sign_in(&denied.url).finish().success);
    devices_fail_saying(&machine, &[], &NOT_SIGNED_IN);
}

#[test]
fn devices_never_send_the_token_to_a_server_it_was_not_stored_for() {
    let first = Stub::start(|path| certifying(path, registered));
    let other = Stub::start(|path| certifying(path, registered));
    let machine = Machine::new();
    sign_in_first(&machine, &first);
    let state = rusqlite::Connection::open(machine.state_dir().join("state.sqlite3")).unwrap();
    state
        .execute("UPDATE session SET server = ?1", [&other.url])
        .unwrap();
    drop(state);

    devices_fail_saying(&machine, &[], &NOT_SIGNED_IN);

    assert_eq!(other.requests(), Vec::<String>::new());
}

#[test]
fn devices_show_the_servers_error_answers() {
    static LISTS: AtomicUsize = AtomicUsize::new(0);
    // The device's registration is taken; then the device list fails.
    let failing = Stub::start(|path| {
        certifying(path, || {
            if LISTS.fetch_add(1, Ordering::SeqCst) == 0 {
                return registered();
            }
            response(
                "500 Internal Server Error",
                "application/problem+json",
                r#"{"type":"about:blank","title":"Internal Server Error","status":500,"detail":"The database failed."}"#,
            )
        })
    });
    let machine = Machine::new();
    sign_in_first(&machine, &failing);

    devices_fail_saying(&machine, &[], &["500", "The database failed."]);

    // An answer that is no device list, such as a registration's.
    let unlisting = Stub::start(|path| certifying(path, registered));
    let machine = Machine::new();
    sign_in_first(&machine, &unlisting);

    devices_fail_saying(&machine, &[], &["a device list without its devices"]);
}
