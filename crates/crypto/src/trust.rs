//! A sender's trust level: `docs/protocol/membership.md`, "Trust levels".

use skald_proto::{AccountId, Cert, DeviceKind, RosterBody, TrustLevel};

/// The level a device of `recipient` gives a message from the device of `sender`, its valid
/// certificate, by its `account` and `kind` and `roster`, the body of the recipient space's
/// roster: `service` for a `service` device; `own` for a `client` or `browser` device of
/// `recipient`; `peer` for one of another account `roster` lists as a member; none otherwise.
pub fn trust_level(recipient: AccountId, sender: &Cert, roster: &RosterBody) -> Option<TrustLevel> {
    let body = sender.body();
    match body.kind {
        DeviceKind::Service => Some(TrustLevel::Service),
        DeviceKind::Client | DeviceKind::Browser if body.account == recipient => {
            Some(TrustLevel::Own)
        }
        DeviceKind::Client | DeviceKind::Browser => {
            roster.member(body.account).map(|_| TrustLevel::Peer)
        }
    }
}
