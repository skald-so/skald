//! `skald invite`: on a `client` device of an admin of a space, creates an invite
//! (`docs/protocol/membership.md`, "Invite") and shows its link. The device makes the secret, gets
//! an invite ID from the server, signs the invite and stores it, sending the same bytes again after
//! a lost answer. The secret stays in memory: a run that ends before the link is shown leaves an
//! invite no one can accept until it ends. The CLI shows the link only at a terminal, as the
//! person (`main`).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_crypto::{InviteLink, InviteSecret, sign_invite};
use skald_proto::{Cbor as _, ChannelId, Invite, InviteBody, InviteId, Role, SpaceId, Unknown};

use crate::connection::{self, Connection, Method, printable};
use crate::devices::{date, text};
use crate::spaces::{Follower, Synced, admin_device, default_of, no_admin, now};

/// Why a run whose output is no terminal shows no link.
pub const NOT_AT_TERMINAL: &str = "skald invite shows an invite link only at a terminal, never \
                                   through a pipe or into a file; nothing was sent";

/// The expiry of an invite unless the person sets another, and the latest, in days.
pub const DEFAULT_DAYS: u64 = 7;
pub const MOST_DAYS: u64 = 30;

/// A day, in seconds.
const DAY: u64 = 86_400;

/// How much earlier than 30 days ahead an invite of 30 days expires, in seconds: the server takes
/// one expiring at most 30 days after it stores it, by its own clock, which may lag the device's.
const MARGIN: u64 = 30;

/// How many times a run sends a request after it got no answer.
const SENDS: usize = 3;

/// What the person asks of an invite.
pub struct Asked<'a> {
    pub admin: bool,
    /// The channels, by ID, whose earlier history the newcomer reads.
    pub with_history: &'a [String],
    /// The channels, by ID, whose earlier history the newcomer does not read.
    pub without_history: &'a [String],
    pub days: u64,
}

/// `NewInvite` in the spec.
#[derive(Deserialize)]
struct Issued {
    invite: String,
    web: Option<String>,
}

/// Creates an invite to `synced`, a space the device follows, as `asked` says, and shows its link.
pub fn create(synced: Synced, asked: &Asked) -> Result<(), String> {
    let Synced {
        mut follower,
        space,
        roster,
        ..
    } = synced;
    let name = space.name.as_str();
    if !admin_device(&roster, follower.own.account, follower.this) {
        return Err(no_admin(&roster));
    }
    let default = default_of(&roster).ok_or_else(|| {
        format!(
            "the space {name} has no default channel with every member yet, which every invite \
             names; run the command again"
        )
    })?;
    let mut channels = vec![(default, true)];
    let named = (asked.with_history.iter().map(|id| (id, true)))
        .chain(asked.without_history.iter().map(|id| (id, false)));
    for (given, history) in named {
        let id: ChannelId = given
            .parse()
            .map_err(|_| format!("\"{}\" is not a channel ID", printable(given)))?;
        if id == default {
            return Err(format!(
                "the channel {id} is the default channel of the space {name}, which every invite \
                 names first"
            ));
        }
        if !roster
            .body()
            .channels
            .iter()
            .any(|channel| channel.id == id)
        {
            return Err(format!("the space {name} has no channel {id}"));
        }
        if channels.iter().any(|(named, _)| *named == id) {
            return Err(format!("the channel {id} is named twice"));
        }
        channels.push((id, history));
    }
    let keys = follower.keys()?;
    let role = if asked.admin {
        Role::Admin
    } else {
        Role::Member
    };
    let (invite, web) = issue(&mut follower, space.id)?;
    let web = web.ok_or("the server names no web address, which an invite link starts with")?;
    let secret = InviteSecret::generate();
    let link = InviteLink {
        web,
        invite,
        secret,
        revision: roster.hash(),
    };
    // The link must read back to what made it: so the web address is an origin with nothing a
    // terminal acts on.
    if !InviteLink::parse(&link.shown()).is_ok_and(|read| read.web == link.web) {
        return Err(format!(
            "the server names \"{}\" as its web address, which is no origin such as \
             https://skald.example; nothing was stored",
            printable(&link.web)
        ));
    }
    let body = InviteBody {
        invite,
        space: space.id,
        key: link.secret.invite_key().public_key(),
        role,
        channels,
        signer: follower.this,
        expires: expiry(now(), asked.days),
        unknown: Unknown::default(),
    };
    let signed =
        sign_invite(&body, keys.signing()).map_err(|err| format!("the invite fails: {err}"))?;
    store(&mut follower, space.id, &signed)?;
    let role = match role {
        Role::Admin => "an admin",
        _ => "a member",
    };
    println!(
        "Invite to the space {name} for {role}, expiring {} (UTC):",
        date(body.expires)
    );
    println!("{}", link.shown().as_str());
    println!("The link admits one person: send it only to them, over a channel you trust.");
    Ok(())
}

/// The `expires` of an invite made at `now` for `days`: that many days ahead, but [`MARGIN`] short
/// of 30 days at most.
fn expiry(now: u64, days: u64) -> u64 {
    now + (days * DAY).min(MOST_DAYS * DAY - MARGIN)
}

/// `POST /spaces/<space>/invites`: an invite ID the server issues for `space`, and the server's web
/// address, if it names one.
fn issue(follower: &mut Follower, space: SpaceId) -> Result<(InviteId, Option<String>), String> {
    let path = format!("/spaces/{space}/invites");
    let answer = sent(
        &mut follower.connection,
        &follower.token,
        |connection, token| connection.post_as(&path, token),
    )?;
    let unexpected = || "unexpected answer from the server: an invite without its ID".to_owned();
    let issued: Issued = serde_json::from_slice(&answer).map_err(|_| unexpected())?;
    Ok((issued.invite.parse().map_err(|_| unexpected())?, issued.web))
}

/// `PUT /spaces/<space>/invites/<invite>`: stores `invite`.
fn store(follower: &mut Follower, space: SpaceId, invite: &Invite) -> Result<(), String> {
    let path = format!("/spaces/{space}/invites/{}", invite.body().invite);
    let request = serde_json::json!({ "invite": URL_SAFE_NO_PAD.encode(invite.to_cbor()) });
    let request = request.to_string();
    sent(
        &mut follower.connection,
        &follower.token,
        |connection, token| connection.send_json_as(Method::Put, &path, token, request.as_bytes()),
    )
    .map(drop)
}

/// The answer to what `send` sends on `connection` as the session `token`, sent again on a new
/// connection while the server does not answer.
fn sent(
    connection: &mut Connection,
    token: &str,
    send: impl Fn(&mut Connection, &str) -> Result<Vec<u8>, connection::Error>,
) -> Result<Vec<u8>, String> {
    resent(connection, token, send).map_err(|err| match err {
        err if err.reached() => text(err),
        err => format!(
            "{}; an invite the server took without answering admits no one, as its link was \
             never shown, and ends when it expires",
            text(err)
        ),
    })
}

/// As [`sent`], giving the error of the last send: the server's answer, or none after the last of
/// [`SENDS`] sends.
pub fn resent(
    connection: &mut Connection,
    token: &str,
    send: impl Fn(&mut Connection, &str) -> Result<Vec<u8>, connection::Error>,
) -> Result<Vec<u8>, connection::Error> {
    let mut sends = 1;
    loop {
        match send(connection, token) {
            Err(err) if !err.reached() && sends < SENDS => {
                sends += 1;
                *connection = connection.again()?;
            }
            answer => return answer,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use skald_proto::VersionRange;

    use super::*;

    /// An invite expires the days asked ahead, and one of 30 days a margin short of them.
    #[test]
    fn expiry_is_the_days_ahead_and_30_days_a_margin_short() {
        let now = 1_800_000_000;
        assert_eq!(expiry(now, 1), now + DAY);
        assert_eq!(expiry(now, 7), now + 7 * DAY);
        assert_eq!(expiry(now, 29), now + 29 * DAY);
        assert_eq!(expiry(now, 30), now + 30 * DAY - MARGIN);
    }

    /// The sends of `answers` in turn by [`resent`] on a connection to `http://127.0.0.1:9`: what
    /// it gives, and how many it sent.
    fn resends(
        answers: Vec<Result<Vec<u8>, connection::Error>>,
    ) -> (Result<Vec<u8>, String>, usize) {
        let mut connection = Connection::new(
            "http://127.0.0.1:9",
            VersionRange::default(),
            Duration::from_secs(1),
        )
        .expect("a connection");
        let answers = std::cell::RefCell::new(answers.into_iter());
        let count = std::cell::Cell::new(0);
        let given = sent(&mut connection, "token", |_, token| {
            assert_eq!(token, "token");
            count.set(count.get() + 1);
            answers
                .borrow_mut()
                .next()
                .expect("no more sends than answers")
        });
        (given, count.get())
    }

    /// A send the server does not answer is sent again, [`SENDS`] times in all; then the error
    /// says an invite the server took admits no one.
    #[test]
    fn unanswered_sends_are_sent_again_up_to_sends() {
        let unanswered = || Err(connection::Error::Unreachable("no answer".to_owned()));
        let (given, count) = resends((0..SENDS + 1).map(|_| unanswered()).collect());
        assert_eq!(count, SENDS);
        assert!(given.unwrap_err().contains("admits no one"));

        let mut answers: Vec<_> = (1..SENDS).map(|_| unanswered()).collect();
        answers.push(Ok(b"taken".to_vec()));
        assert_eq!(resends(answers), (Ok(b"taken".to_vec()), SENDS));
    }

    /// The server's answer, a refusal too, is given at once, as it says.
    #[test]
    fn an_answer_is_given_at_once() {
        let refused = connection::Error::Refused {
            status: 409,
            kind: "invite-taken".to_owned(),
            detail: String::new(),
            task: None,
        };
        let (given, count) = resends(vec![Err(refused), Ok(Vec::new())]);
        assert_eq!(count, 1);
        assert!(!given.unwrap_err().contains("admits no one"));
    }
}
