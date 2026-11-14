//! What the tests of sending and receiving messages share: a world whose server is behind a front,
//! people's devices with the keys the tests hold, devices the tests write to the server's database,
//! a channel of Ada's in a space whose revisions the tests sign, and runs of `skald send`.

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use skald_crypto::{HpkeKey, SigningKey, issue};
use skald_proto::{
    AccountId, Cbor as _, Cert, CertBody, ChannelId, ChannelMember, DeviceId, DeviceKind,
    MessageId, NonEmpty, Role, Roster, SpaceId, Unknown,
};
use skald_server::Settings;

use super::browser::Browser;
use super::cloud::{Cloud, WAIT};
use super::front::{Front, Sent};
use super::keys;
use super::machine::{Finished, Machine, Run};
use super::world::{
    ACME, CREATE_CHANNEL, HOST, Person, World, create, next_by, next_with, now, run, store,
};

/// The command that sends a message.
pub const SEND: &str = "send";

/// The setting of the request timeout, and the one the tests give a run that retries.
pub const TIMEOUT_SETTING: &str = "SKALD_TIMEOUT";
pub const TIMEOUT: Duration = Duration::from_secs(4);

/// A world whose server's public address is that of a front before the relay, which forwards to
/// it, and the front.
pub fn fronted() -> (World, Front) {
    fronted_with(|_| {})
}

/// As [`fronted`], the server's settings changed by `change`.
pub fn fronted_with(change: impl FnOnce(&mut Settings)) -> (World, Front) {
    let front = Front::bind();
    let cloud = Cloud::fronted_with(HOST, front.port(), 120, 1, change);
    front.forward_to(cloud.relay.url.trim_start_matches("http://"));
    let world = World {
        cloud,
        browser: Browser::launch(),
    };
    (world, front)
}

/// `person`, its runs retrying for [`TIMEOUT`].
pub fn retrying_for_the_timeout(mut person: Person) -> Person {
    person.machine = person
        .machine
        .with_env(TIMEOUT_SETTING, TIMEOUT.as_secs().to_string());
    person
}

/// Runs `skald send <channel> <text>` on `machine`.
pub fn send(machine: &Machine, channel: ChannelId, text: &str) -> Finished {
    machine.start(&strs(&send_args(channel, text))).finish()
}

/// The arguments of `skald send <channel> <text>`.
pub fn send_args(channel: ChannelId, text: &str) -> Vec<String> {
    vec![SEND.to_owned(), channel.to_string(), text.to_owned()]
}

/// `args` as arguments.
pub fn strs(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

/// A device whose keys the test holds, to open what is sealed to it and to sign as it.
pub struct Keyed {
    pub id: DeviceId,
    pub hpke: HpkeKey,
    pub signing: SigningKey,
}

/// The certificate of `person`'s machine's device, as the server holds it.
pub fn cert_of(cloud: &Cloud, person: &Person) -> Cert {
    cloud
        .devices(&person.token)
        .into_iter()
        .find(|device| device.id == person.device)
        .and_then(|device| device.cert)
        .expect("the device's certificate")
}

/// `person`'s machine's device, its keys read from its secrets file.
pub fn keyed(cloud: &Cloud, person: &Person) -> Keyed {
    let public = cert_of(cloud, person).body().hpke;
    let holders = person.machine.files_holding(&person.token);
    assert_eq!(holders.len(), 1, "the secrets file: {holders:?}");
    let secrets = fs::read(&holders[0]).expect("read the secrets file");
    let secret = keys::private_key(&secrets, &public, keys::x25519)
        .expect("the secrets file holds the HPKE key");
    Keyed {
        id: person.device,
        hpke: HpkeKey::from_bytes(&secret),
        signing: SigningKey::from_bytes(&person.key.to_bytes()),
    }
}

/// Who issued the certificate of a device a test writes to the server's database.
pub enum Issuer<'a> {
    /// The device of this person's machine: a valid certificate.
    Device(&'a Person),
    /// A key of no device, in place of the account's master key, as only a server could sign it.
    Server,
    /// Nobody: the device has no certificate.
    Nobody,
}

/// Writes to the server's database a `client` device of `account` named `name`, its certificate
/// issued as `issuer` says.
pub fn add_device(cloud: &Cloud, account: AccountId, name: &str, issuer: &Issuer) -> Keyed {
    let signing = SigningKey::generate();
    let hpke = HpkeKey::generate();
    let id = DeviceId::from_signing_key(&signing.public_key());
    let body = |issuer| CertBody {
        account,
        device: id,
        kind: DeviceKind::Client,
        signing: signing.public_key(),
        hpke: hpke.public_key(),
        issuer,
        issued: now(),
        ssh: None,
        name: name.to_owned(),
        unknown: Unknown::default(),
    };
    let cert = match issuer {
        Issuer::Device(person) => Some(issue(&body(Some(person.device)), &person.key, &signing)),
        Issuer::Server => Some(issue(&body(None), &SigningKey::generate(), &signing)),
        Issuer::Nobody => None,
    }
    .map(|cert| cert.expect("a certificate").to_cbor());
    let bytes = |bytes: &[u8]| format!("'\\x{}'::bytea", hex(bytes));
    let columns = [
        format!("'{id}'::uuid"),
        format!("'{account}'::uuid"),
        "'client'".to_owned(),
        bytes(&signing.public_key()),
        bytes(&hpke.public_key()),
        bytes(name.as_bytes()),
        cert.as_deref().map_or_else(|| "NULL".to_owned(), bytes),
    ];
    cloud.execute(&format!(
        "INSERT INTO devices (id, account_id, kind, signing, hpke, name, cert) VALUES ({})",
        columns.join(", ")
    ));
    Keyed { id, hpke, signing }
}

/// `bytes` in lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A space of Ada's and the one channel Ada's machine created in it.
pub struct Channel {
    pub space: SpaceId,
    pub id: ChannelId,
}

/// Ada's machine creates `acme`, whose revision 2 adds `members`, and a channel in it; revision 4,
/// which Ada's device signs and the server stores while the machine has not accepted it yet, adds
/// `joining` to the channel and has each member's account of `excluded` bar its device.
pub fn acme(
    cloud: &Cloud,
    ada: &Person,
    members: &[&Person],
    joining: &[&Person],
    excluded: &[(AccountId, DeviceId)],
) -> Channel {
    let first = create(cloud, ada, ACME);
    channel_in(cloud, ada, &first, members, joining, excluded)
}

/// As [`acme`], in the space named `first.body().name` that Ada's machine created, whose revision 1
/// is `first`.
pub fn channel_in(
    cloud: &Cloud,
    ada: &Person,
    first: &Roster,
    members: &[&Person],
    joining: &[&Person],
    excluded: &[(AccountId, DeviceId)],
) -> Channel {
    let space = first.body().space;
    let second = next_by(first, ada, |list| {
        list.extend(members.iter().map(|person| person.member(Role::Member)));
    });
    store(cloud, &ada.token, &second);
    let name = first.body().name.to_string();
    let created = run(&ada.machine, &[CREATE_CHANNEL, &name]);
    assert!(created.success, "a channel: {created:?}");
    let (third, id) = last_channel(cloud, ada, space);
    if !joining.is_empty() || !excluded.is_empty() {
        let fourth = next_with(&third, (ada.device, &ada.key), |body| {
            let channel = body
                .channels
                .iter_mut()
                .find(|channel| channel.id == id)
                .expect("the channel");
            channel
                .members
                .extend(joining.iter().map(|person| ChannelMember {
                    account: person.account,
                    history: false,
                }));
            let mut members = body.members.clone().into_vec();
            for (account, device) in excluded {
                let member = members
                    .iter_mut()
                    .find(|member| member.account == *account)
                    .expect("a member");
                member.excluded = Some(NonEmpty::new(*device));
            }
            body.members = NonEmpty::from_vec(members).expect("a member");
        });
        store(cloud, &ada.token, &fourth);
    }
    Channel { space, id }
}

/// The last revision of `space` the server holds and the ID of its last channel.
pub fn last_channel(cloud: &Cloud, person: &Person, space: SpaceId) -> (Roster, ChannelId) {
    let (_, last) = cloud
        .revisions(&person.token, space)
        .pop()
        .expect("a revision");
    let id = last.body().channels.last().expect("a channel").id;
    (last, id)
}

/// The recipient device of `sent`, an envelope under `hpke` to one device.
pub fn recipient(sent: &Sent) -> DeviceId {
    let header = &sent.envelope.envelope().header;
    match header.recipients.as_deref() {
        Some([device]) => *device,
        other => panic!("not one recipient device: {other:?}"),
    }
}

pub fn msg_id(sent: &Sent) -> MessageId {
    sent.envelope.envelope().header.msg_id
}

/// The person address of `person` in `space`.
pub fn person_address(space: &str, person: &Person) -> String {
    format!("{space}/{}/user@{HOST}", person.handle)
}

/// What the server's database holds, in lowercase, byte strings in hex.
pub fn database(cloud: &Cloud) -> String {
    cloud.rows().concat().to_lowercase()
}

/// Waits for up to [`WAIT`] until `done` holds, then kills `run`; fails if `run` exits first.
pub fn kill_once(run: Run, what: &str, done: impl Fn() -> bool) {
    killed_once(run, what, done);
}

/// As [`kill_once`]; gives what `run` showed.
pub fn killed_once(mut run: Run, what: &str, done: impl Fn() -> bool) -> Finished {
    let deadline = Instant::now() + WAIT;
    while !done() {
        run.assert_running();
        assert!(Instant::now() < deadline, "no {what}: {}", run.shown());
        thread::sleep(Duration::from_millis(20));
    }
    run.kill()
}

/// In `world`, Ada, GitHub user `ids[0]` with `logins[0]`, whose machine retries for [`TIMEOUT`],
/// and Bob, `ids[1]` with `logins[1]`, sign up, and Ada's machine creates `acme` with a channel
/// they are both in, Bob added by a revision the machine has not accepted yet.
pub fn ada_and_bob(world: &World, ids: [u64; 2], logins: [&str; 2]) -> (Person, Person, Channel) {
    let ada = retrying_for_the_timeout(world.sign_up(ids[0], logins[0], "ada-workstation"));
    let bob = world.sign_up(ids[1], logins[1], "bob-workstation");
    let channel = acme(&world.cloud, &ada, &[&bob], &[&bob], &[]);
    (ada, bob, channel)
}
