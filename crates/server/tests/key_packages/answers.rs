//! The answers of the KeyPackages API, as the server's tests and the client's read them.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use skald_proto::DeviceId;

/// What a `GET /devices/{device}/key-packages` answer `body` says: how many unused KeyPackages the
/// device has that expire in more than 7 days, and whether its last-resort one is to be replaced.
#[track_caller]
pub fn status_of(body: &Value) -> (u64, bool) {
    let unused = body["unused"].as_u64();
    let replace = body["replace_last_resort"].as_bool();
    match (unused, replace) {
        (Some(unused), Some(replace)) => (unused, replace),
        _ => panic!("not a count of KeyPackages: {body}"),
    }
}

/// The KeyPackages of a `POST /key-packages` answer `body`, in its order, each with its device.
#[track_caller]
pub fn handed_of(body: &Value) -> Vec<(DeviceId, Vec<u8>)> {
    let Some(listed) = body["key_packages"].as_array() else {
        panic!("no key_packages in {body}");
    };
    listed
        .iter()
        .map(|entry| {
            let device = entry["device"].as_str().and_then(|id| id.parse().ok());
            let bytes = entry["key_package"]
                .as_str()
                .and_then(|text| URL_SAFE_NO_PAD.decode(text).ok());
            match (device, bytes) {
                (Some(device), Some(bytes)) => (device, bytes),
                _ => panic!("not a handed-out KeyPackage: {entry}"),
            }
        })
        .collect()
}
