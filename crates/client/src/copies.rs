//! Copies of a member device's channel messages to service agents outside the channel's group:
//! `docs/protocol/delivery.md`, "Send". A message with an `a2a` body also goes under `hpke` to each
//! `service` device whose grant the server gives the space's members, when the device has an entry
//! in the latest revision the sending device accepted but is not in the channel's `services` there,
//! has a valid certificate, and its grant lists the channel and passes the checks of "Service
//! grant"; and when the content has no `to`, or `to` names that entry.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_crypto::check_grant;
use skald_proto::{
    Address, Cbor as _, ChannelId, DeviceId, DeviceKind, Grant, Roster, ServerName, SpaceId,
};

use crate::devices::text;
use crate::follow::{Own, SpaceDevices};
use crate::recv::{member_cert, service_entry};
use crate::spaces::Follower;
use crate::state;

/// `ServiceGrants` in the API spec.
#[derive(Deserialize)]
struct Grants {
    grants: Vec<String>,
}

/// A service agent outside a channel's group that gets copies of its messages.
#[derive(Debug, Clone, PartialEq)]
pub struct Reader {
    pub device: DeviceId,
    /// The HPKE key of its certificate.
    pub hpke: [u8; 32],
    /// Its `owner/agent`.
    pub agent: String,
}

/// The service agents that get a copy of a message to `to`, or to every agent without it, in
/// `channel` of `roster`, the space's last accepted revision with `devices`, its accounts' devices,
/// on `server`, by the grants the server gives the space's members. Warns, and gives none, if it
/// gets no grants.
pub fn readers(
    follower: &mut Follower,
    (roster, devices): (&Roster, &SpaceDevices),
    (channel, to): (ChannelId, Option<&Address>),
    server: &ServerName,
) -> Vec<Reader> {
    let space = roster.body().space;
    match grants(follower, space) {
        Ok(grants) => chosen(
            &follower.own,
            (roster, devices),
            (channel, to),
            server,
            &grants,
        ),
        Err(err) => {
            eprintln!(
                "skald: warning: the server gives no service grants of the space {}, so no \
                 service agent outside the channel's group gets a copy of the message: {err}",
                roster.body().name.as_str()
            );
            Vec::new()
        }
    }
}

/// The grants the server gives the space `space`'s members, those that do not decode left out.
fn grants(follower: &mut Follower, space: SpaceId) -> Result<Vec<Grant>, String> {
    let path = format!("/spaces/{space}/service-grants");
    let body = (follower.connection)
        .get_as(&path, &follower.token)
        .map_err(text)?;
    let given: Grants = serde_json::from_slice(&body)
        .map_err(|_| "unexpected answer from the server: service grants without their list")?;
    Ok(given
        .grants
        .iter()
        .filter_map(|grant| URL_SAFE_NO_PAD.decode(grant).ok())
        .filter_map(|bytes| Grant::from_cbor(&bytes).ok())
        .collect())
}

/// Of the devices `grants` name, those [`readers`] gives, each once, in the order of their first
/// grant that passes, checked as the device of `own` checks them.
fn chosen(
    own: &Own,
    (roster, devices): (&Roster, &SpaceDevices),
    (channel, to): (ChannelId, Option<&Address>),
    server: &ServerName,
    grants: &[Grant],
) -> Vec<Reader> {
    let body = roster.body();
    let Some(listed) = body.channels.iter().find(|listed| listed.id == channel) else {
        return Vec::new();
    };
    let now = state::unix_now();
    let mut readers: Vec<Reader> = Vec::new();
    for grant in grants {
        let device = grant.body.service;
        if readers.iter().any(|reader| reader.device == device) || listed.services.contains(&device)
        {
            continue;
        }
        let Some(entry) = service_entry(roster, device) else {
            continue;
        };
        let Some(cert) = member_cert(own, (roster, devices), device)
            .filter(|cert| cert.body().kind == DeviceKind::Service)
        else {
            continue;
        };
        let address = Address::new(
            body.name.clone(),
            entry.owner.clone(),
            entry.agent.clone(),
            server.clone(),
        );
        if to.is_some_and(|to| *to != address) {
            continue;
        }
        let passes = check_grant(
            grant,
            (device, channel),
            roster,
            &devices.certs,
            &devices.revocations,
            (own.account, &own.master),
            now,
        );
        if passes.is_ok() {
            readers.push(Reader {
                device,
                hpke: cert.body().hpke,
                agent: format!("{}/{}", entry.owner.as_str(), entry.agent.as_str()),
            });
        }
    }
    readers
}

/// The warning that the device starts sending copies of `channel`'s messages to `reader`.
pub fn first_copy(reader: &Reader, channel: ChannelId) -> String {
    format!(
        "this device starts sending copies of the messages of the channel {channel} to the \
         service agent {} (device {}), which is not in the channel's group: an admin's grant \
         lets it read them",
        reader.agent, reader.device
    )
}

#[cfg(test)]
mod tests {
    use skald_crypto::SigningKey;
    use skald_proto::{GRANT_LABEL, GrantBody, NonEmpty, ServiceAgent, Unknown};

    use super::*;
    use crate::send::tests::{Account, channel_of};
    use crate::spaces::tests::first;

    /// The service agent `ci/<agent>` of `device`.
    fn entry(device: DeviceId, agent: &str) -> ServiceAgent {
        ServiceAgent {
            device,
            owner: "ci".parse().unwrap(),
            agent: agent.parse().unwrap(),
            unknown: Unknown::default(),
        }
    }

    /// The grant `key`'s device `signer` signs for `service` in `channel`, an hour from now, or a
    /// second later if `later`.
    fn grant(
        (signer, key): (DeviceId, &SigningKey),
        service: DeviceId,
        channel: ChannelId,
        later: bool,
    ) -> Grant {
        let body = GrantBody {
            service,
            channels: NonEmpty::new(channel),
            signer,
            expires: state::unix_now() + 3600 + u64::from(later),
            unknown: Unknown::default(),
        };
        let signature = key.sign(GRANT_LABEL, &[&body.to_cbor()]);
        Grant { body, signature }
    }

    /// Ada, an admin, has a client device and five devices that her grants name, each but one
    /// certified by it. Of them, a copy goes only to `ci/deploy-bot`, once for its two grants: not
    /// to a member agent of the channel's `services`, nor to a device without an entry, nor to one
    /// whose grant lists another channel, nor to a `client` device with an entry. With `to`, only to
    /// the agent it names.
    #[test]
    fn copies_go_to_service_agents_outside_the_group_with_a_grant_for_the_channel() {
        let ada = Account::new();
        let (this, _, key) = ada.client();
        let id = this.body().device;
        let service = || ada.device(DeviceKind::Service, &key, Some(id));
        let ((deploy, deploy_hpke, _), (member, ..), (unlisted, ..), (other, ..)) =
            (service(), service(), service(), service());
        let (client, ..) = ada.device(DeviceKind::Client, &key, Some(id));
        let certs = vec![this, deploy, member, unlisted, other, client];
        let [_, deploy, member, unlisted, other, client] =
            [0, 1, 2, 3, 4, 5].map(|at| certs[at].body().device);
        let channel = ChannelId::generate();
        let mut body = first(ada.id).body().clone();
        let mut listed = channel_of(channel, &[ada.id]);
        listed.services.push(member);
        body.channels = vec![listed];
        body.service_agents = vec![
            entry(deploy, "deploy-bot"),
            entry(member, "member-bot"),
            entry(other, "other-bot"),
            entry(client, "client-bot"),
        ];
        let roster = skald_crypto::sign_roster(&body, &SigningKey::generate()).unwrap();
        let devices = SpaceDevices {
            ids: certs.iter().map(|cert| cert.body().device).collect(),
            certs,
            revocations: Vec::new(),
            records: Vec::new(),
            deleted: Default::default(),
        };
        let own = Own {
            account: ada.id,
            master: ada.master.public_key(),
        };
        let signer = (id, &key);
        let grants = [
            grant(signer, member, channel, false),
            grant(signer, unlisted, channel, false),
            grant(signer, deploy, channel, false),
            grant(signer, other, ChannelId::generate(), false),
            grant(signer, client, channel, false),
            grant(signer, deploy, channel, true),
        ];
        let server: ServerName = "skald.example".parse().unwrap();
        let chosen_for = |to: Option<&str>| {
            let to: Option<Address> = to.map(|to| to.parse().unwrap());
            let to = to.as_ref();
            chosen(&own, (&roster, &devices), (channel, to), &server, &grants)
        };
        let deploy = Reader {
            device: deploy,
            hpke: deploy_hpke,
            agent: "ci/deploy-bot".to_owned(),
        };

        assert_eq!(chosen_for(None), std::slice::from_ref(&deploy));
        assert_eq!(
            chosen_for(Some("acme/ci/deploy-bot@skald.example")),
            [deploy]
        );
        for to in [
            "acme/ci/other-bot@skald.example",
            "acme/ada/claude@skald.example",
        ] {
            assert_eq!(chosen_for(Some(to)), [], "{to}");
        }
        let elsewhere = chosen(
            &own,
            (&roster, &devices),
            (ChannelId::generate(), None),
            &server,
            &grants,
        );
        assert_eq!(elsewhere, []);
    }
}
