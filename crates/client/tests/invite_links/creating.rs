//! An admin's device creates an invite at a terminal: the server holds it, signed by the device,
//! and the link it shows reads back to the server's web address, the invite's ID, a secret that
//! derives the invite's `key` and the hash of the revision the device last accepted.

use skald_crypto::{InviteLink, check_invite};
use skald_proto::{Invite, Role};

use crate::world::{ADMIN, MEMBER, now};
use crate::{
    AS_ADMIN, Admin, CHANNEL, DAY, DAYS, SLACK, WITHOUT_HISTORY, assert_word, dates_around,
    invite_at_terminal, server_invite, shown_link,
};

/// What a run of `skald invite` showed, the link it showed, the invite the server holds under the
/// link's ID, and the time before and after the run.
struct Created {
    text: String,
    link: InviteLink,
    invite: Invite,
    span: (u64, u64),
}

/// Runs `skald invite acme <extra>` on Ada's machine at a terminal; fails unless it succeeds,
/// shows one link under the server's web address, and the server holds an invite under its ID.
#[track_caller]
fn created(admin: &Admin, extra: &[&str]) -> Created {
    let before = now();
    let shown = invite_at_terminal(&admin.ada.machine, extra);
    let after = now();
    assert!(shown.success, "{}", shown.text);
    let (_, link) = shown_link(&shown.text, &admin.cloud().public);
    let invite = server_invite(admin.cloud(), &admin.ada.token, link.invite)
        .unwrap_or_else(|| panic!("the server holds no invite {}: {}", link.invite, shown.text));
    Created {
        text: shown.text,
        link,
        invite,
        span: (before, after),
    }
}

/// Fails unless `created`'s invite is the one its link names, of `acme`, signed by Ada's device
/// with its key, its `key` derived from the link's secret, and the link names the hash of the last
/// revision of `acme`, which Ada's device signed and so last accepted; fails unless its expiry is
/// `days` ahead of the run and the run shows its date.
#[track_caller]
fn assert_invite_of_the_link(admin: &Admin, created: &Created, days: u64) {
    let (invite, link) = (created.invite.body(), &created.link);
    assert_eq!(invite.invite, link.invite, "{invite:?}");
    assert_eq!(invite.space, admin.space, "{invite:?}");
    assert_eq!(invite.signer, admin.ada.device, "{invite:?}");
    check_invite(&created.invite, &admin.ada.key.public_key())
        .unwrap_or_else(|err| panic!("the invite's signature: {err}"));
    assert_eq!(
        invite.key,
        link.secret.invite_key().public_key(),
        "the link's secret does not derive the invite's key"
    );
    assert_eq!(
        link.revision,
        admin.latest().hash(),
        "the link names another revision than the last one the device accepted"
    );
    let (before, after) = created.span;
    let ahead = days * DAY;
    assert!(
        (before + ahead - SLACK..=after + ahead + SLACK).contains(&invite.expires),
        "expires at {}, not {days} days after the run at {before}..{after}",
        invite.expires
    );
    assert!(
        dates_around(invite.expires)
            .iter()
            .any(|date| created.text.contains(date.as_str())),
        "it does not show the expiry's date, {:?}: {}",
        dates_around(invite.expires),
        created.text
    );
}

/// Fails unless the run says the link admits one person and goes only to them over a channel the
/// person trusts.
#[track_caller]
fn assert_says_how_to_send_it(text: &str) {
    let said = text.to_lowercase();
    for part in ["one person", "trust"] {
        assert!(said.contains(part), "it does not say {part:?}: {text}");
    }
}

/// `skald invite acme` with no option, on Ada's machine at a terminal: the server holds an invite
/// for a member to `acme`'s default channel alone, with `history` true, expiring 7 days ahead,
/// signed by Ada's device; the link reads back to the server's web address, the invite's ID, a
/// secret that derives its `key` and the hash of `acme`'s last revision. The run shows the role,
/// the expiry's date, and that the link admits one person and goes over a channel they trust.
#[test]
fn admin_device_creates_an_invite_to_the_default_channel_and_shows_its_link() {
    let admin = Admin::new((6_418_301_001, "ada-invites-a-member"));

    let created = created(&admin, &[]);

    assert_eq!(
        created.link.web,
        admin.cloud().public,
        "the link's web address"
    );
    let invite = created.invite.body();
    assert_eq!(invite.role, Role::Member, "{invite:?}");
    assert_eq!(invite.channels, [(admin.default, true)], "{invite:?}");
    assert_invite_of_the_link(&admin, &created, 7);
    assert_word(&created.text, MEMBER);
    assert_says_how_to_send_it(&created.text);
}

/// `skald invite acme --admin --channel <first> --channel-without-history <second> --days 30`: the
/// server holds an invite for an admin, naming the default channel first with `history` true, then
/// the first channel with `history` true and the second with `history` false, expiring 30 days
/// ahead; the run shows the role `admin`.
#[test]
fn admin_device_invites_an_admin_to_the_channels_it_names_for_30_days() {
    let admin = Admin::new((6_418_301_101, "ada-invites-an-admin"));
    let [first, second] = admin.channels.map(|channel| channel.to_string());

    let created = created(
        &admin,
        &[
            AS_ADMIN,
            CHANNEL,
            &first,
            WITHOUT_HISTORY,
            &second,
            DAYS,
            "30",
        ],
    );

    let invite = created.invite.body();
    assert_eq!(invite.role, Role::Admin, "{invite:?}");
    assert_eq!(
        invite.channels.first(),
        Some(&(admin.default, true)),
        "not the default channel first: {invite:?}"
    );
    let mut named = invite.channels[1..].to_vec();
    named.sort();
    let mut asked = vec![(admin.channels[0], true), (admin.channels[1], false)];
    asked.sort();
    assert_eq!(
        named, asked,
        "the channels after the default one: {invite:?}"
    );
    assert_invite_of_the_link(&admin, &created, 30);
    assert_word(&created.text, ADMIN);
    assert_says_how_to_send_it(&created.text);
}

/// `skald invite acme --days 1`: the invite expires a day ahead.
#[test]
fn days_set_the_expiry() {
    let admin = Admin::new((6_418_301_201, "ada-invites-for-a-day"));

    let created = created(&admin, &[DAYS, "1"]);

    assert_eq!(created.invite.body().channels, [(admin.default, true)]);
    assert_invite_of_the_link(&admin, &created, 1);
}
