//! What the tests share: the history command and what it shows, the messages the CLI sends and the
//! played devices say, the entries a compromised device writes, and the entries a compromised
//! server adds.

use std::thread;
use std::time::Duration;

use aes_gcm::aead::{Aead as _, Payload};
use aes_gcm::{Aes128Gcm, KeyInit as _, Nonce};
use serde_json::Value;
use skald_crypto::{HistoryKey, SigningKey};
use skald_proto::{
    Cbor as _, ChannelId, ChannelMember, DeviceId, Frame, HISTORY_LABEL, HistoryBody, HistoryEntry,
    HistoryItem, HistoryPlaintext, MessageId, Role, SpaceId, Unknown,
};
use skald_server::Settings;

use crate::chain::{key_of, uploads_link};
use crate::cloud::Cloud;
use crate::fixtures::{person_address, send};
use crate::kit::{b64, latest};
use crate::machine::{Finished, Machine};
use crate::peer::Peer;
use crate::played::Played;
use crate::talk::{Line, Talk, encrypted, stored};
use crate::world::{ACME, Person, next_with, now, run, store, warnings};

/// The command that shows a channel's history.
pub const HISTORY: &str = "history";

/// What the history command says on the line that counts the entries it holds no key for.
pub const NO_KEY: &str = "no key";

/// Runs `skald history <channel>` on `machine`, with `text` after it if any.
pub fn history(machine: &Machine, channel: ChannelId, text: Option<&str>) -> Finished {
    let channel = channel.to_string();
    let mut args = vec![HISTORY, channel.as_str()];
    args.extend(text);
    run(machine, &args)
}

/// Runs `skald history <channel>` on `machine`; fails unless it succeeds.
#[track_caller]
pub fn shown_history(machine: &Machine, channel: ChannelId) -> Finished {
    let shown = history(machine, channel, None);
    assert!(shown.success, "{shown:?}");
    shown
}

/// A message as the history shows it.
#[derive(Debug, Clone)]
pub struct Said {
    pub text: String,
    /// `from`, in text form.
    pub from: String,
    /// The sending device's certificate `name`.
    pub device: &'static str,
    /// The first and the last second its entry's `timestamp` may be.
    pub sent: (u64, u64),
}

/// Ada, whose machine runs the CLI, and Bob, whose device the tests play, in a channel of `acme`,
/// and Bob's address there.
pub struct Duo {
    pub talk: Talk,
    pub at: (SpaceId, ChannelId),
    pub from_bob: String,
}

impl Duo {
    /// Ada, GitHub user `ids[0]` with `logins[0]`, and Bob the next, Bob a member; the server's
    /// settings changed by `change`.
    pub fn with(ids: &[u64; 2], logins: &[&str; 2], change: impl FnOnce(&mut Settings)) -> Self {
        let talk = Talk::with(ACME, ids, logins, false, (Role::Member, change));
        Duo {
            at: talk.at(),
            from_bob: talk.address(&talk.played[0].person),
            talk,
        }
    }

    /// As [`Duo::with`], with the server's settings as they are.
    pub fn new(ids: &[u64; 2], logins: &[&str; 2]) -> Self {
        Self::with(ids, logins, |_| {})
    }

    /// Bob's line `text`, his `counter`th in the channel.
    pub fn bob_s(&self, counter: u64, text: &'static str) -> Line<'static> {
        Line::new(&self.from_bob, counter, text)
    }

    /// Bob says `line` in the channel, as [`says`] does.
    #[track_caller]
    pub fn bob_says(&mut self, line: &Line) -> (Said, MessageId, Vec<u8>) {
        let talk = &mut self.talk;
        says(&talk.world.cloud, &mut talk.played[0], self.at, line)
    }

    /// Runs `skald history` of the channel on Ada's machine; fails unless it succeeds.
    #[track_caller]
    pub fn ada_s_history(&self) -> Finished {
        shown_history(&self.talk.ada.machine, self.at.1)
    }
}

/// Waits until the clock has moved on to the next second, so that a message sent after has a
/// later time than every message sent before.
pub fn next_second() {
    let second = now();
    while now() == second {
        thread::sleep(Duration::from_millis(20));
    }
}

/// `person`'s machine sends `text` to `channel` of the space `space` with `skald send`, a second
/// after the message before; fails unless it succeeds.
#[track_caller]
pub fn sends(person: &Person, (space, channel): (&str, ChannelId), text: &str) -> Said {
    next_second();
    let before = now();
    let sent = send(&person.machine, channel, text);
    assert!(sent.success, "{sent:?}");
    Said {
        text: text.to_owned(),
        from: person_address(space, person),
        device: person.host,
        sent: (before, now()),
    }
}

/// The body of the entry of the message `msg_id` with `line`, from the device `sender`, naming
/// `channel`, stated as sent now.
pub fn body_of(
    sender: DeviceId,
    channel: ChannelId,
    (msg_id, line): (MessageId, &Line),
) -> HistoryBody {
    HistoryBody {
        sender,
        channel,
        msg_id,
        timestamp: now(),
        content: line.content(msg_id),
        unknown: Unknown::default(),
    }
}

/// `played` says `line` in `at`'s channel of the space `space` as its own client would, a second
/// after the message before: it takes in its mailbox, sends the message in its current epoch, then
/// uploads the message's entry under that epoch's key, signed by its device; fails unless the
/// server stores both. Gives the message, its `msg_id` and the entry's bytes.
#[track_caller]
pub fn says(
    cloud: &Cloud,
    played: &mut Played,
    at: (SpaceId, ChannelId),
    line: &Line,
) -> (Said, MessageId, Vec<u8>) {
    next_second();
    played.take_in(cloud);
    let envelope = encrypted(played, at, at.1, line);
    stored(cloud, played, &envelope);
    let msg_id = envelope.envelope().header.msg_id;
    let body = body_of(played.device(), at.1, (msg_id, line));
    let epoch = played.mls.epoch(at.1).expect("the device's epoch");
    let entry = key_of(played, at.1, epoch)
        .seal_entry(epoch, &body, &played.person.key)
        .expect("an entry")
        .to_cbor();
    uploads_entry(cloud, &played.person, at, (msg_id, &entry));
    let said = Said {
        text: line.text.to_owned(),
        from: line.from.clone(),
        device: played.person.host,
        sent: (body.timestamp, body.timestamp),
    };
    (said, msg_id, entry)
}

/// `uploader`'s device uploads `entry`, the entry of the message `msg_id`, of `at`'s channel on its
/// connection; fails unless the server stores it.
#[track_caller]
pub fn uploads_entry(
    cloud: &Cloud,
    uploader: &Person,
    at: (SpaceId, ChannelId),
    (msg_id, entry): (MessageId, &[u8]),
) {
    let item = HistoryItem::Entry {
        msg_id,
        entry: entry.to_vec(),
    };
    let answer = Peer::connect(cloud, uploader, &uploader.key).upload(1, at, item);
    assert_eq!(
        answer,
        Frame::HistoryStored { upload: 1 },
        "the entry not stored"
    );
}

/// The entry of `at`'s channel of `epoch` under `key` that holds `body` signed by `signer`, sealed
/// whatever channel the body names, as a compromised device could.
pub fn sealed_as_is(
    key: &HistoryKey,
    (channel, epoch): (ChannelId, u64),
    body: &HistoryBody,
    signer: &SigningKey,
) -> HistoryEntry {
    let bytes = body.to_cbor();
    let signature = signer.sign(HISTORY_LABEL, &[&bytes]);
    let plaintext = HistoryPlaintext::from_parts(bytes, signature)
        .expect("a history plaintext")
        .to_cbor();
    let nonce = [0x5a; 12];
    let cipher = Aes128Gcm::new((&*key.to_bytes()).into());
    let aad = HistoryEntry::associated_data(channel, epoch);
    let ciphertext = cipher
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: &plaintext,
                aad: &aad,
            },
        )
        .expect("a sealed entry");
    HistoryEntry {
        channel,
        epoch,
        nonce,
        ciphertext,
        unknown: Unknown::default(),
    }
}

/// The request for `at`'s channel's history entries, as its line starts.
pub fn entries_request((space, channel): (SpaceId, ChannelId)) -> String {
    format!("GET /spaces/{space}/channels/{channel}/history/entries")
}

/// From now on, the relay gives the machines the entries of `at`'s channel the server gives with
/// `added`, encoded entries, after them, as a compromised server would.
pub fn add_entries(cloud: &Cloud, at: (SpaceId, ChannelId), added: &[Vec<u8>]) {
    let added: Vec<Value> = added
        .iter()
        .map(|entry| Value::String(b64(entry)))
        .collect();
    rewrite_entries(cloud, at, move |list| list.extend(added.iter().cloned()));
}

/// From now on, changes the entries of `at` the relay gives the machines.
pub fn rewrite_entries(
    cloud: &Cloud,
    at: (SpaceId, ChannelId),
    rewrite: impl Fn(&mut Vec<Value>) + Send + Sync + 'static,
) {
    cloud.relay.rewrite(&entries_request(at), move |answer| {
        let mut given: Value = serde_json::from_str(answer).expect("the server's entries");
        let list = given["entries"].as_array_mut().expect("a list of entries");
        rewrite(list);
        given.to_string()
    });
}

/// The revision after the latest of `at`'s space, signed by `signer`'s device and stored as a
/// fixture, that adds `person`, a member of the space, to `at`'s channel with `history`.
pub fn admits(
    cloud: &Cloud,
    signer: &Person,
    at: (SpaceId, ChannelId),
    (person, history): (&Person, bool),
) {
    let previous = latest(cloud, &signer.token, at.0);
    let next = next_with(&previous, (signer.device, &signer.key), |body| {
        let channel = (body.channels.iter_mut())
            .find(|channel| channel.id == at.1)
            .expect("the channel in the space");
        channel.members.push(ChannelMember {
            account: person.account,
            history,
        });
    });
    store(cloud, &signer.token, &next);
}

/// `played` commits an empty Commit in `at`'s channel as [`crate::chain::commits_empty`] does,
/// then uploads the link of the new epoch to the one before; gives the new epoch.
#[track_caller]
pub fn rotates(cloud: &Cloud, played: &mut Played, at: (SpaceId, ChannelId)) -> u64 {
    let epoch = crate::chain::commits_empty(cloud, played, at);
    uploads_link(cloud, played, at, (epoch, epoch - 1));
    epoch
}

/// `seconds` since the Unix epoch in UTC as RFC 3339 to the second, such as
/// `2026-11-17T08:43:03Z`.
pub fn rfc3339(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).expect("a time within range");
    let of_day = seconds % 86_400;
    // The civil date of a day count, by Howard Hinnant's algorithm.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let of_era = shifted - era * 146_097;
    let year_of_era = (of_era - of_era / 1_460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * of_year + 2) / 153;
    let day = of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3_600,
        of_day % 3_600 / 60,
        of_day % 60
    )
}

/// Fails unless the output of `shown`, a run of the history command, shows the messages `expected`
/// of `channel`, in order, as [`assert_shows`] says, and none of `absent`.
#[track_caller]
pub fn assert_history(shown: &Finished, channel: ChannelId, expected: &[Said], absent: &[&str]) {
    let mut last: Option<(usize, &Said)> = None;
    for said in expected {
        let at = assert_shows(shown, channel, said);
        if let Some((before, earlier)) = last {
            assert!(
                before < at,
                "{:?} not shown after {:?}: {shown:?}",
                said.text,
                earlier.text
            );
        }
        last = Some((at, said));
    }
    for text in absent {
        assert!(!shown.shown().contains(text), "{text:?} shown: {shown:?}");
    }
}

/// Fails unless the output of `shown` holds `said`'s text once, on a line of the standard output
/// that holds `channel`'s ID, `said`'s `from` and device name, and its time; gives the line's
/// number.
#[track_caller]
pub fn assert_shows(shown: &Finished, channel: ChannelId, said: &Said) -> usize {
    let text = said.text.as_str();
    assert_eq!(
        shown.shown().matches(text).count(),
        1,
        "{text:?} not shown once: {shown:?}"
    );
    let (at, line) = (shown.stdout.lines().enumerate())
        .find(|(_, line)| line.contains(text))
        .unwrap_or_else(|| panic!("{text:?} not on a line of its own: {shown:?}"));
    for part in [channel.to_string().as_str(), &said.from, said.device] {
        assert!(line.contains(part), "{text:?} shown without {part}: {line}");
    }
    let times: Vec<String> = (said.sent.0..=said.sent.1).map(rfc3339).collect();
    assert!(
        times.iter().any(|time| line.contains(time.as_str())),
        "{text:?} shown without its time, one of {times:?}: {line}"
    );
    at
}

/// The number of entries the output of `shown` says the history command holds no key for: the
/// first number on a line saying [`NO_KEY`]; `None` without such a line.
pub fn without_key(shown: &Finished) -> Option<u64> {
    let all = shown.shown();
    let line = all
        .lines()
        .find(|line| line.to_lowercase().contains(NO_KEY))?;
    line.split_whitespace()
        .map(|word| word.trim_matches(|char: char| !char.is_ascii_alphanumeric()))
        .find_map(|word| word.parse().ok())
}

/// The warning lines of `shown`, in lowercase, that hold `device`'s ID.
pub fn warned_of(shown: &Finished, device: impl std::fmt::Display) -> Vec<String> {
    let device = device.to_string();
    warnings(shown)
        .into_iter()
        .filter(|line| line.contains(&device))
        .collect()
}
