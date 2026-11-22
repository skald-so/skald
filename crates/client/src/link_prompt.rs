//! The prompt while a device of the account waits for linking (`docs/protocol/membership.md`,
//! "Device certificate" and "Link code"). Each command of a linked device that reaches the server,
//! and each tool call of its MCP server, asks the server for the account's devices waiting for
//! linking (`getWaitingDevices` in the OpenAPI spec) and says once of each, on one line, that the
//! server reports, unverified, a device of that `name` and kind waiting, and that `skald link`
//! links it; with `--json`, as an object. The name is the server's text, escaped as other text from
//! the server is. Only a prompt: a certificate goes only through the code its person types
//! ([`crate::link_code`]). A `skald recv --wait` learns of a device that starts waiting from the
//! server's `link-notice` ([`crate::wait`]).

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;
use skald_proto::VersionRange;

use crate::connection::printable;
use crate::devices::{signed_in, to_session};
use crate::secrets;
use crate::state::{self, Link, State};

/// Where the server gives the account's devices waiting for linking.
const WAITING: &str = "/account/links/waiting";

/// `WaitingDevices` in the API spec.
#[derive(Deserialize)]
struct Waiting {
    devices: Vec<WaitingDevice>,
}

/// `WaitingDevice` in the API spec.
#[derive(Deserialize)]
struct WaitingDevice {
    device: String,
    kind: String,
    name: String,
}

/// The devices a command said were waiting, by their IDs, so that it says each once.
pub type Said = Mutex<Vec<String>>;

/// Says, as [`line`] does or as JSON if `json`, each device of the account of the machine whose
/// state is in `dir`, its secrets in `storage`, that the server reports waiting for linking and
/// `said` does not hold yet, and adds it there; only on a device with a certificate. Gives whether
/// it said one. A failure says nothing: the prompt is only a prompt.
pub fn show(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    json: bool,
    said: &Said,
) -> bool {
    let Some(waiting) = waiting(dir, storage, speaks, timeout) else {
        return false;
    };
    let mut said = said.lock().expect("the devices said");
    let mut shown = false;
    for device in waiting {
        if said.contains(&device.device) {
            continue;
        }
        if json {
            println!("{}", object(&device));
        } else {
            eprintln!("{}", line(&device));
        }
        said.push(device.device);
        shown = true;
    }
    shown
}

/// The account's devices other than this one that the server reports waiting for linking; `None`
/// on a device without a certificate, or if the server does not say.
fn waiting(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
) -> Option<Vec<WaitingDevice>> {
    if !dir.join(state::FILE).is_file() {
        return None;
    }
    let this = State::open(dir).ok()?.device().ok()??;
    if !matches!(this.link, Link::Certified { .. }) {
        return None;
    }
    let (_, server, token) = signed_in(dir, storage).ok()?;
    let mut connection = to_session(&server, speaks, timeout).ok()?;
    let body = connection.get_as(WAITING, &token).ok()?;
    let listed: Waiting = serde_json::from_slice(&body).ok()?;
    let this = this.id.to_string();
    Some(
        (listed.devices.into_iter())
            .filter(|device| device.device != this)
            .collect(),
    )
}

/// The line that says `device` waits.
fn line(device: &WaitingDevice) -> String {
    format!(
        "skald: the server reports, unverified, a {} device \"{}\" waiting: run skald link",
        printable(&device.kind),
        printable(&device.name)
    )
}

/// The JSON object that says `device` waits.
fn object(device: &WaitingDevice) -> serde_json::Value {
    json!({
        "waiting_device": {
            "kind": device.kind,
            "name": device.name,
            "verified": false,
            "link_with": "skald link",
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str) -> WaitingDevice {
        WaitingDevice {
            device: "0".to_owned(),
            kind: "client".to_owned(),
            name: name.to_owned(),
        }
    }

    #[test]
    fn the_line_names_the_device_as_the_server_reports_it() {
        let line = line(&device("ada-laptop"));
        for part in [
            "the server reports",
            "unverified",
            "client",
            "\"ada-laptop\"",
            "skald link",
        ] {
            assert!(line.contains(part), "no {part:?} in {line:?}");
        }
    }

    #[test]
    fn the_line_escapes_the_name_on_one_line() {
        let line = line(&device("a\u{1b}[31mb\nskald: forged\u{7}"));
        assert!(
            line.contains(r#""a\u{1b}[31mb\nskald: forged\u{7}""#),
            "{line}"
        );
        assert!(!line.contains(['\u{1b}', '\n', '\u{7}']), "{line:?}");
    }

    #[test]
    fn the_object_holds_the_name_and_kind_as_given() {
        let object = object(&device("ada\nlaptop"));
        assert_eq!(object["waiting_device"]["name"], "ada\nlaptop");
        assert_eq!(object["waiting_device"]["kind"], "client");
        assert_eq!(object["waiting_device"]["verified"], false);
    }
}
