//! `skald history`: the device reads a channel's history (`docs/protocol/membership.md`, "Key
//! chain", "History entry" and "Access"); as an agent, only a channel of its folder's space.
//!
//! - It accepts the space's latest revisions, then gets the channel's links and entries from the
//!   server. Given a text, it searches on the device: no request carries the text.
//! - It opens each entry with the key of its epoch: one the device holds, or one it reaches back
//!   from those through every link it can open. A key from a link stays in memory.
//! - It shows an entry only if it passes the checks of "History entry": the sender's certificate
//!   passes checks 1 and 2 of "Device checks", revoked or not, with the master key of an account
//!   that a revision the device accepted lists among the channel's members, or, for a `service`
//!   device, one the channel's `services` there lists; its content passes checks 2 and 4 of
//!   "Envelope". It drops any other with a warning naming the sending device, once it knows it, and
//!   the check.
//! - It shows each message once per sending device and `msg_id`, oldest first by the time its
//!   sender states, on a line of the standard output: that time in UTC, the channel, `from`, the
//!   sending device's name and ID and the text, control characters escaped. It then says how many
//!   entries it holds no key for.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write as _};
use std::path::Path;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_crypto::{HistoryKey, HistoryMessage, MlsDevice, check_device};
use skald_proto::{
    Address, Cbor as _, Cert, ChannelId, DeviceId, DeviceKind, HistoryEntry, HistoryLink,
    MessageId, Roster, ServerName, SpaceId, VersionRange,
};

use crate::agents::server_name;
use crate::connection::printable;
use crate::devices::text;
use crate::follow::{self, Own, SpaceDevices, master_of};
use crate::groups;
use crate::recv::{a2a_text, of_account};
use crate::secrets;
use crate::spaces::{self, Follower, Scope, Synced};
use crate::state;

/// Shows the history of the channel whose ID is `channel`, by the state in `dir`, its secrets in
/// `storage`, as an agent if `agent`; with `search`, only the messages whose text contains it.
pub fn run(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    channel: &str,
    search: Option<&str>,
    agent: bool,
) -> Result<(), String> {
    let channel: ChannelId = channel
        .parse()
        .map_err(|_| format!("\"{}\" is not a channel ID", printable(channel)))?;
    let scope = Scope::of_run(dir, agent)?;
    let Synced {
        mut follower,
        space,
        roster,
        devices,
    } = spaces::of_channel(dir, storage, speaks, timeout, channel, scope)?;
    let revisions = accepted(&mut follower, &space, roster)?;
    let server = server_name(&mut follower.connection)?;
    let at = (space.id, channel);
    let links: Vec<HistoryLink> = (listed(&mut follower, at, LINKS)?.iter())
        .filter_map(|bytes| HistoryLink::from_cbor(bytes).ok())
        .filter(|link| link.channel == channel)
        .collect();
    let entries = listed(&mut follower, at, ENTRIES)?;
    let keys = follower.keys()?;
    let opened = groups::read(&follower, &keys, |device| {
        open(device, channel, &links, &entries)
    })?;
    let checks = Checks {
        own: &follower.own,
        revisions: &revisions,
        devices: &devices,
        server: server.as_ref(),
        channel,
    };
    let (mut read, without_key) = checks.read(opened);
    read.sort_by_key(|message| message.timestamp);
    let mut out = io::stdout().lock();
    for message in read
        .iter()
        .filter(|message| search.is_none_or(|search| message.text.contains(search)))
    {
        writeln!(out, "{}", message.line(channel))
            .and_then(|()| out.flush())
            .map_err(|err| format!("cannot write a message to the output: {err}"))?;
    }
    if without_key > 0 {
        eprintln!(
            "skald: history entries under no key this device holds, not shown: {without_key}"
        );
    }
    Ok(())
}

/// The kinds of history items the server lists, as their paths and lists name them.
const LINKS: &str = "links";
const ENTRIES: &str = "entries";

/// The revisions of `space` the device accepted that the server gives, `latest` its last accepted
/// one first.
fn accepted(
    follower: &mut Follower,
    space: &state::Space,
    latest: Roster,
) -> Result<Vec<Roster>, String> {
    let held = follow::read(&follower.state, space.id, &space.name)?;
    let given = follow::listed(&mut follower.connection, &follower.token, space.id)?;
    let mut revisions = vec![latest];
    for roster in (given.iter())
        .filter_map(|entry| URL_SAFE_NO_PAD.decode(entry).ok())
        .filter_map(|bytes| Roster::from_cbor(&bytes).ok())
    {
        if held.hashes.get(&roster.body().revision) == Some(&roster.hash()) {
            revisions.push(roster);
        }
    }
    Ok(revisions)
}

/// `HistoryLinks` and `HistoryEntries` in the API spec: the list named by its kind.
#[derive(Deserialize)]
struct Listed(BTreeMap<String, Vec<String>>);

/// The items of `kind`, [`LINKS`] or [`ENTRIES`], of `at`'s channel the server lists, each as its
/// bytes; one that is no base64url, as no bytes, which decode as no item.
fn listed(
    follower: &mut Follower,
    (space, channel): (SpaceId, ChannelId),
    kind: &str,
) -> Result<Vec<Vec<u8>>, String> {
    let path = format!("/spaces/{space}/channels/{channel}/history/{kind}");
    let body = follower
        .connection
        .get_as(&path, &follower.token)
        .map_err(text)?;
    let unexpected = || format!("unexpected answer from the server: a channel's history {kind}");
    let Listed(mut given) = serde_json::from_slice(&body).map_err(|_| unexpected())?;
    let items = given.remove(kind).ok_or_else(unexpected)?;
    Ok((items.iter())
        .map(|item| URL_SAFE_NO_PAD.decode(item).unwrap_or_default())
        .collect())
}

/// What opening an entry gave.
#[derive(Debug)]
enum Opened {
    Message(Box<HistoryMessage>),
    /// The device holds no key of its epoch.
    NoKey,
    /// It does not decode, is of another channel or does not decrypt with its epoch's key: why.
    Failed(String),
}

/// The keys of the epochs of `channel` before those `device` holds that `links`, the channel's,
/// reach back from them, following every link it can open.
fn reach(
    device: &MlsDevice,
    channel: ChannelId,
    links: &[HistoryLink],
) -> BTreeMap<u64, HistoryKey> {
    let mut links: Vec<&HistoryLink> = links.iter().collect();
    // A link's `prev` is below its epoch, so from the latest epoch down each link's key is known
    // by the time it comes up.
    links.sort_by_key(|link| std::cmp::Reverse(link.epoch));
    let mut reached = BTreeMap::new();
    for link in links {
        if device.history_key(channel, link.prev).is_some() {
            continue;
        }
        let key = (device.history_key(channel, link.epoch))
            .or_else(|| reached.get(&link.epoch))
            .and_then(|key| key.open_link(link).ok());
        if let Some(key) = key {
            reached.entry(link.prev).or_insert(key);
        }
    }
    reached
}

/// Each of `entries`, encoded entries of `channel`, opened with the keys `device` holds and those
/// `links`, the channel's, reach.
fn open(
    device: &MlsDevice,
    channel: ChannelId,
    links: &[HistoryLink],
    entries: &[Vec<u8>],
) -> Vec<Opened> {
    let reached = reach(device, channel, links);
    let key = |epoch| {
        device
            .history_key(channel, epoch)
            .or_else(|| reached.get(&epoch))
    };
    (entries.iter())
        .map(|bytes| {
            let entry = match HistoryEntry::from_cbor(bytes) {
                Ok(entry) => entry,
                Err(err) => return Opened::Failed(format!("it does not decode: {err}")),
            };
            if entry.channel != channel {
                return Opened::Failed("it is of another channel".to_owned());
            }
            let Some(key) = key(entry.epoch) else {
                return Opened::NoKey;
            };
            match key.decrypt_entry(&entry) {
                Ok(message) => Opened::Message(Box::new(message)),
                Err(err) => Opened::Failed(format!(
                    "it does not open with the key of its epoch {}: {err}",
                    entry.epoch
                )),
            }
        })
        .collect()
}

/// What the device checks entries of `channel` against: its account and the master key it pinned,
/// the revisions of the space it accepted, the latest first, the devices of the space's accounts
/// and the server's name.
struct Checks<'c> {
    own: &'c Own,
    revisions: &'c [Roster],
    devices: &'c SpaceDevices,
    server: Option<&'c ServerName>,
    channel: ChannelId,
}

/// A message of the history that passed the checks.
#[derive(Debug)]
struct Read {
    sender: DeviceId,
    msg_id: MessageId,
    /// When it was sent, as its sender states it, in seconds since the Unix epoch.
    timestamp: u64,
    from: Address,
    /// The sending device's certificate `name`.
    device: String,
    text: String,
}

impl Read {
    /// Its line, as the history shows it in `channel`.
    fn line(&self, channel: ChannelId) -> String {
        format!(
            "{} {channel} {} by the device {} ({}): {}",
            rfc3339(self.timestamp),
            self.from,
            printable(&self.device),
            self.sender,
            printable(&self.text)
        )
    }
}

impl Checks<'_> {
    /// The messages of `opened` that pass the checks, each once per sending device and `msg_id`,
    /// and how many entries the device holds no key for; warns of each entry it drops.
    fn read(&self, opened: Vec<Opened>) -> (Vec<Read>, u64) {
        let channel = self.channel;
        let mut seen = BTreeSet::new();
        let mut read = Vec::new();
        let mut without_key = 0;
        for item in opened {
            match item {
                Opened::NoKey => without_key += 1,
                Opened::Failed(why) => eprintln!(
                    "skald: warning: dropped a history entry of the channel {channel}: {why}; it \
                     is not shown"
                ),
                Opened::Message(message) => {
                    let sender = message.sender();
                    match self.check(*message) {
                        Ok(message) => {
                            if seen.insert((sender, message.msg_id)) {
                                read.push(message);
                            }
                        }
                        Err(why) => eprintln!(
                            "skald: warning: dropped a history entry of the channel {channel} \
                             from the device {sender}: {why}; it is not shown"
                        ),
                    }
                }
            }
        }
        (read, without_key)
    }

    /// The message of `message` once it passes the checks of "History entry"; why it fails
    /// otherwise.
    fn check(&self, message: HistoryMessage) -> Result<Read, String> {
        if message.channel() != self.channel {
            return Err("its body names another channel than the entry".to_owned());
        }
        let sender = message.sender();
        let (cert, admitting) = self.sender(sender).ok_or_else(|| {
            "no revision of the space this device accepted lists the sending device's account \
             among the channel's members, or the device among its services, with a valid \
             certificate"
                .to_owned()
        })?;
        let body = message.verify(cert).map_err(|_| {
            "its signature does not verify with the sending device's certificate".to_owned()
        })?;
        let from = &body.content.from;
        if !(admitting.iter()).any(|roster| self.sends_as(roster, cert, from)) {
            return Err(
                "its from is no address of the sending device's account in the space".to_owned(),
            );
        }
        let text = a2a_text(&body.content, body.msg_id)?;
        Ok(Read {
            sender,
            msg_id: body.msg_id,
            timestamp: body.timestamp,
            from: from.clone(),
            device: cert.body().name.clone(),
            text,
        })
    }

    /// The certificate of `sender`, revoked or not, and the revisions that admit it to the channel:
    /// each that lists its account among the channel's members, or, for a `service` device, the
    /// device among the channel's `services`, where it passes checks 1 and 2 with the master key
    /// the revision gives its account; none if none does.
    fn sender(&self, sender: DeviceId) -> Option<(&Cert, Vec<&Roster>)> {
        let mut found: Option<(&Cert, Vec<&Roster>)> = None;
        for roster in self.revisions {
            let body = roster.body();
            let Some(channel) = body.channels.iter().find(|it| it.id == self.channel) else {
                continue;
            };
            let service = channel.services.contains(&sender);
            let accounts: Vec<_> = if service {
                body.members.iter().map(|member| member.account).collect()
            } else {
                channel.members.iter().map(|entry| entry.account).collect()
            };
            let cert = accounts.into_iter().find_map(|account| {
                let master = master_of(roster, self.own, account)?;
                let cert = check_device(&master, &self.devices.certs, sender).ok()?;
                let of = cert.body();
                (of.account == account && (of.kind == DeviceKind::Service) == service)
                    .then_some(cert)
            });
            if let Some(cert) = cert {
                found
                    .get_or_insert_with(|| (cert, Vec::new()))
                    .1
                    .push(roster);
            }
        }
        found
    }

    /// Whether `from` is an address the device of `cert` sends as by `roster`: a `service`
    /// device's own, as the space's service agents give it; any other's, one of its account.
    fn sends_as(&self, roster: &Roster, cert: &Cert, from: &Address) -> bool {
        let body = roster.body();
        if cert.body().kind != DeviceKind::Service {
            return of_account(from, body, cert.body().account, self.server);
        }
        body.service_agents.iter().any(|agent| {
            agent.device == cert.body().device
                && from.space() == &body.name
                && from.owner() == &agent.owner
                && from.agent() == &agent.agent
                && Some(from.server()) == self.server
        })
    }
}

/// `seconds` since the Unix epoch in UTC as RFC 3339 to the second, such as
/// `2026-11-17T08:43:03Z`.
fn rfc3339(seconds: u64) -> String {
    let (days, of_day) = (seconds / 86_400, seconds % 86_400);
    // The civil date of a day count from 1970-01-01, by Howard Hinnant's algorithm, from
    // 0000-03-01.
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let of_era = shifted % 146_097;
    let year_of_era = (of_era - of_era / 1_460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * of_year + 2) / 153;
    let day = of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3_600,
        of_day % 3_600 / 60,
        of_day % 60
    )
}

#[cfg(test)]
mod tests {
    use skald_crypto::SigningKey;
    use skald_proto::{Body, Content, HistoryBody, ServiceAgent, Unknown};

    use super::*;
    use crate::send::tests::{Account, channel_of};
    use crate::spaces::tests::{first, member};

    /// Ada's device, this one, in `acme` with Bob. In the earlier revision, the channel has Ada and
    /// Bob, and Bob's `service` device, which sends as `acme/ci/deploy`; in the latest, Ada alone.
    struct World {
        own: Own,
        latest: Roster,
        earlier: Roster,
        devices: SpaceDevices,
        channel: ChannelId,
        server: ServerName,
        bob: (DeviceId, SigningKey),
        service: (DeviceId, SigningKey),
        key: HistoryKey,
    }

    impl World {
        fn new() -> Self {
            let (ada, bob) = (Account::new(), Account::new());
            let (bob_cert, _, bob_key) = bob.client();
            let bob_id = bob_cert.body().device;
            let (service_cert, _, service_key) =
                bob.device(DeviceKind::Service, &bob_key, Some(bob_id));
            let service_id = service_cert.body().device;
            let channel = ChannelId::generate();
            let mut body = first(ada.id).body().clone();
            body.members
                .push(member(bob.id, "bob", bob.master.public_key()));
            body.service_agents = vec![ServiceAgent {
                device: service_id,
                owner: "ci".parse().unwrap(),
                agent: "deploy".parse().unwrap(),
                unknown: Unknown::default(),
            }];
            let mut in_channel = channel_of(channel, &[ada.id, bob.id]);
            in_channel.services = vec![service_id];
            body.channels = vec![in_channel];
            let sign = |body: &skald_proto::RosterBody| {
                skald_crypto::sign_roster(body, &SigningKey::generate()).unwrap()
            };
            let earlier = sign(&body);
            body.revision = 2;
            body.channels = vec![channel_of(channel, &[ada.id])];
            let latest = sign(&body);
            let certs = vec![bob_cert, service_cert];
            World {
                own: Own {
                    account: ada.id,
                    master: ada.master.public_key(),
                },
                latest,
                earlier,
                devices: SpaceDevices {
                    ids: certs.iter().map(|cert| cert.body().device).collect(),
                    certs,
                    revocations: Vec::new(),
                },
                channel,
                server: "skald.example".parse().unwrap(),
                bob: (bob_id, bob_key),
                service: (service_id, service_key),
                key: HistoryKey::from_bytes(&[7; 16]),
            }
        }

        /// The checks with `revisions` as those the device accepted.
        fn checks<'w>(&'w self, revisions: &'w [Roster]) -> Checks<'w> {
            Checks {
                own: &self.own,
                revisions,
                devices: &self.devices,
                server: Some(&self.server),
                channel: self.channel,
            }
        }

        /// The body of the message `msg_id` from `sender` in `channel`, from `from` with the
        /// text `text`.
        fn body(
            &self,
            (sender, channel): (DeviceId, ChannelId),
            msg_id: MessageId,
            from: &str,
            text: &str,
        ) -> HistoryBody {
            let a2a = serde_json::json!({
                "messageId": msg_id.to_string(),
                "role": "ROLE_USER",
                "parts": [{ "text": text }],
            });
            HistoryBody {
                sender,
                channel,
                msg_id,
                timestamp: 1_800_000_000,
                content: Content {
                    from: format!("{from}@{}", self.server).parse().unwrap(),
                    to: None,
                    counter: std::num::NonZeroU64::new(1),
                    hops: 0,
                    reply_to: None,
                    body: Some(Body::A2a(a2a.to_string())),
                    delegation: None,
                    grant: None,
                    attachments: None,
                    unknown: Unknown::default(),
                },
                unknown: Unknown::default(),
            }
        }

        /// The message of an entry of epoch 1 that holds `body` signed with `key`.
        fn opened(&self, body: &HistoryBody, key: &SigningKey) -> HistoryMessage {
            let entry = self.key.seal_entry(1, body, key).unwrap();
            self.key.decrypt_entry(&entry).unwrap()
        }

        /// The message of an entry of epoch 1 of `channel` from `sender`, signed with its key,
        /// from `from` with the text `text`.
        fn entry(
            &self,
            (sender, key): (DeviceId, &SigningKey),
            channel: ChannelId,
            from: &str,
            text: &str,
        ) -> HistoryMessage {
            let body = self.body((sender, channel), MessageId::generate(), from, text);
            self.opened(&body, key)
        }
    }

    /// A member removed from the channel since, and a `service` device the channel had, pass by
    /// the earlier revision the device accepted; by the latest alone, neither does.
    #[test]
    fn senders_pass_by_any_revision_the_device_accepted() {
        let world = World::new();
        let both = [world.latest.clone(), world.earlier.clone()];
        let latest = std::slice::from_ref(&world.latest);
        let bob = (world.bob.0, &world.bob.1);
        let service = (world.service.0, &world.service.1);
        let channel = world.channel;

        let read = world
            .checks(&both)
            .check(world.entry(bob, channel, "acme/bob/user", "from bob"))
            .unwrap();
        assert_eq!(
            (read.sender, read.device.as_str(), read.text.as_str()),
            (world.bob.0, "laptop", "from bob")
        );
        assert_eq!(read.from.to_string(), "acme/bob/user@skald.example");
        let read = world
            .checks(&both)
            .check(world.entry(service, channel, "acme/ci/deploy", "deployed"))
            .unwrap();
        assert_eq!(read.text, "deployed");

        for (sender, from) in [(bob, "acme/bob/user"), (service, "acme/ci/deploy")] {
            let refused = world
                .checks(latest)
                .check(world.entry(sender, channel, from, "dropped"));
            assert!(
                refused
                    .unwrap_err()
                    .contains("lists the sending device's account"),
                "{from}"
            );
        }
    }

    /// A `service` device sends only as its own service agent, and any other only as an agent of
    /// its account; a body of another channel is dropped first.
    #[test]
    fn senders_send_only_as_their_own_addresses() {
        let world = World::new();
        let both = [world.latest.clone(), world.earlier.clone()];
        let checks = world.checks(&both);
        let bob = (world.bob.0, &world.bob.1);
        let service = (world.service.0, &world.service.1);
        let channel = world.channel;

        for (sender, from) in [
            (service, "acme/bob/user"),
            (service, "acme/ci/other"),
            (bob, "acme/ci/deploy"),
            (bob, "acme/ada/user"),
            (bob, "other/bob/user"),
        ] {
            let refused = checks.check(world.entry(sender, channel, from, "dropped"));
            assert!(refused.unwrap_err().contains("its from"), "{from}");
        }
        let elsewhere = world.entry(bob, ChannelId::generate(), "acme/bob/user", "dropped");
        assert_eq!(
            checks.check(elsewhere).unwrap_err(),
            "its body names another channel than the entry"
        );
    }

    /// A message shows once per sending device and `msg_id`, whatever its entries hold, and
    /// entries under no key are counted.
    #[test]
    fn a_message_reads_once_and_entries_under_no_key_are_counted() {
        let world = World::new();
        let both = [world.latest.clone(), world.earlier.clone()];
        let (bob, key) = (world.bob.0, &world.bob.1);
        let msg_id = MessageId::generate();
        let once = world.body((bob, world.channel), msg_id, "acme/bob/user", "once");
        let otherwise = world.body((bob, world.channel), msg_id, "acme/bob/user", "otherwise");
        let other = world.body(
            (bob, world.channel),
            MessageId::generate(),
            "acme/bob/user",
            "next",
        );

        let (read, without_key) = world.checks(&both).read(vec![
            Opened::Message(Box::new(world.opened(&once, key))),
            Opened::NoKey,
            Opened::Message(Box::new(world.opened(&once, key))),
            Opened::Message(Box::new(world.opened(&otherwise, key))),
            Opened::Failed("it does not decode".to_owned()),
            Opened::Message(Box::new(world.opened(&other, key))),
            Opened::NoKey,
        ]);

        let texts: Vec<&str> = read.iter().map(|message| message.text.as_str()).collect();
        assert_eq!(texts, ["once", "next"]);
        assert_eq!(without_key, 2);
    }

    /// Times show in UTC as RFC 3339 to the second, leap days included.
    #[test]
    fn times_show_as_rfc_3339_in_utc() {
        for (seconds, shown) in [
            (0, "1970-01-01T00:00:00Z"),
            (951_782_400, "2000-02-29T00:00:00Z"),
            (1_795_336_983, "2026-11-22T08:43:03Z"),
            (4_107_542_399, "2100-02-28T23:59:59Z"),
            (4_107_542_400, "2100-03-01T00:00:00Z"),
        ] {
            assert_eq!(rfc3339(seconds), shown, "{seconds}");
        }
    }
}
