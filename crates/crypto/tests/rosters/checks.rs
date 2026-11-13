//! Accepting revision n+1 after an accepted revision n: checks 1 to 4 of `membership.md`,
//! "Roster", with the certificates the server returns. The signer's device passes the proof check
//! and checks 1 and 2 of "Device checks" against the master key pinned for the client's own
//! account, or the `master` revision n gives for another account. Checks 3 and 4 also hold for a
//! revision on its own.

use crate::fixtures::*;

/// [`next`] signed by `seed`, its `signer` that device.
fn signed_by(seed: u8) -> Rev {
    Rev {
        signer: seed,
        ..next()
    }
}

#[test]
fn a_valid_next_revision_by_an_admins_client_device_is_accepted() {
    accepts(&next().roster()).unwrap();
}

#[test]
fn a_valid_next_revision_by_an_admins_browser_is_accepted() {
    accepts(&signed_by(ALICE_BROWSER).roster()).unwrap();
}

#[test]
fn a_valid_next_revision_by_a_device_of_another_admin_account_is_accepted() {
    accepts(&signed_by(BOB_LAPTOP).roster()).unwrap();
}

#[test]
fn a_valid_revision_passes_the_content_checks_on_its_own() {
    passes_content_checks(&current().bytes()).unwrap();
    passes_content_checks(&next().bytes()).unwrap();
}

#[test]
fn revisions_follow_one_another() {
    let third = next();
    let fourth = successor(&third);
    accepts_after(&third, &fourth.roster()).unwrap();
    assert!(accepts(&fourth.roster()).is_err(), "revision 4 after 2");
}

// Check 1: the signer.

#[test]
fn a_signer_of_a_member_account_that_is_no_admin_is_rejected() {
    assert!(accepts(&signed_by(CAROL_LAPTOP).roster()).is_err());
}

#[test]
fn a_member_that_makes_itself_admin_is_rejected() {
    let mut rev = signed_by(CAROL_LAPTOP);
    rev.members[2].role = ADMIN;
    assert!(accepts(&rev.roster()).is_err());
}

#[test]
fn a_signer_of_an_account_outside_the_space_is_rejected() {
    assert!(accepts(&signed_by(DAVE_LAPTOP).roster()).is_err());

    // Also when the revision it signs adds its account as an admin.
    let mut rev = signed_by(DAVE_LAPTOP);
    rev.members.push(member(DAVE, "dave", ADMIN, DAVE_MASTER));
    assert!(accepts(&rev.roster()).is_err());
}

#[test]
fn an_admin_of_revision_n_signs_a_revision_that_makes_it_a_member() {
    let mut rev = signed_by(BOB_LAPTOP);
    rev.members[1].role = MEMBER;
    accepts(&rev.roster()).unwrap();
}

#[test]
fn a_service_device_on_an_admins_account_is_no_signer() {
    assert!(accepts(&signed_by(ALICE_SERVICE).roster()).is_err());
    assert!(accepts(&signed_by(BOB_CI).roster()).is_err());
}

#[test]
fn a_signer_whose_certificate_only_the_server_signed_is_rejected() {
    assert!(accepts(&signed_by(ALICE_BY_SERVER).roster()).is_err());
}

#[test]
fn a_signer_chained_to_another_master_key_is_rejected() {
    assert!(accepts(&signed_by(ALICE_ROGUE).roster()).is_err());
    assert!(accepts(&signed_by(BOB_ROGUE).roster()).is_err());
    // Chained to the master key of another member of the space.
    assert!(accepts(&signed_by(BOB_BY_CAROL).roster()).is_err());
}

#[test]
fn a_signer_chained_to_an_admins_master_key_but_certified_on_another_account_is_rejected() {
    assert!(accepts(&signed_by(BOB_BY_ALICE).roster()).is_err());
}

#[test]
fn a_device_that_revision_n_excludes_is_no_signer() {
    let mut previous = current();
    previous.members[0].excluded = vec![device(ALICE_LAPTOP)];
    let mut rev = successor(&previous);
    rev.signer = ALICE_LAPTOP;
    assert!(accepts_after(&previous, &rev.roster()).is_err());

    // Another device of the same admin account still signs.
    rev.signer = ALICE_BROWSER;
    accepts_after(&previous, &rev.roster()).unwrap();
}

#[test]
fn a_signer_without_a_certificate_is_rejected() {
    assert!(accepts(&signed_by(UNCERTIFIED).roster()).is_err());
}

#[test]
fn a_signer_of_the_clients_own_account_is_checked_against_the_pinned_master_key() {
    // Revision n gives alice's account another master key than the one the client pinned.
    let mut previous = current();
    previous.members[0].master = public(ROGUE_MASTER);
    let mut rev = successor(&previous);
    rev.signer = ALICE_ROGUE;
    assert!(accepts_after(&previous, &rev.roster()).is_err());

    rev.signer = ALICE_LAPTOP;
    accepts_after(&previous, &rev.roster()).unwrap();
}

#[test]
fn a_signer_of_another_account_is_checked_against_the_master_key_of_revision_n() {
    // Revision n+1 gives bob's account another master key; its signer is chained to that one.
    // No service agent of bob's account, whose certificate is chained to his first master key.
    let mut rev = signed_by(BOB_ROGUE);
    rev.members[1].master = public(ROGUE_MASTER);
    rev.service_agents.clear();
    for channel in &mut rev.channels {
        channel.services.clear();
    }
    assert!(accepts(&rev.roster()).is_err());

    // The same change signed by a device chained to the master key of revision n.
    rev.signer = BOB_LAPTOP;
    accepts(&rev.roster()).unwrap();
}

#[test]
fn a_broken_signature_is_rejected() {
    let mut encoded = next().bytes();
    let last = encoded.len() - 1;
    encoded[last] ^= 0x01;
    assert!(accepts_bytes(&encoded).is_err());
}

#[test]
fn a_signature_over_another_label_is_rejected() {
    let rev = next();
    let body = rev.body();
    for label in [
        "skald cert",
        "skald invite",
        "skald anchors",
        "skald roster ",
    ] {
        let signature = signature(ALICE_LAPTOP, label, &body);
        let encoded = crate::cbor::Value::Array(vec![
            crate::cbor::bytes(&body),
            crate::cbor::bytes(&signature),
        ])
        .encode();
        assert!(accepts_bytes(&encoded).is_err(), "{label}");
    }
}

#[test]
fn a_signature_by_another_admins_device_than_the_signer_is_rejected() {
    // `signer` names alice's laptop; bob's laptop signed it.
    assert!(accepts(&next().signed_with(BOB_LAPTOP)).is_err());
}

// Check 2: the revision follows revision n.

#[test]
fn another_space_is_rejected() {
    let mut rev = next();
    rev.space = OTHER_SPACE;
    assert!(accepts(&rev.roster()).is_err());
}

#[test]
fn another_name_is_rejected() {
    let mut rev = next();
    rev.name = "acme2".to_owned();
    assert!(accepts(&rev.roster()).is_err());
}

#[test]
fn another_revision_number_is_rejected() {
    for revision in [1, 2, 4, 5] {
        let mut rev = next();
        rev.revision = revision;
        assert!(accepts(&rev.roster()).is_err(), "revision {revision}");
    }
}

#[test]
fn no_revision_follows_the_highest_revision_number() {
    let mut previous = current();
    previous.revision = u64::MAX;
    let rev = Rev {
        prev: Some(previous.roster().hash()),
        ..previous.clone()
    };
    assert!(accepts_after(&previous, &rev.roster()).is_err());
}

#[test]
fn another_prev_is_rejected() {
    let mut rev = next();
    rev.prev = Some([0x77; 32]);
    assert!(accepts(&rev.roster()).is_err(), "the prev of revision n");

    let mut flipped = next().prev.unwrap();
    flipped[0] ^= 0x01;
    rev.prev = Some(flipped);
    assert!(accepts(&rev.roster()).is_err(), "one bit off");

    rev.prev = None;
    assert!(accepts_bytes(&rev.bytes()).is_err(), "no prev");
}

// Checks 3 and 4: the content. Each revision fails both accepting it after revision n and the
// checks of a revision on its own.

/// [`next`] changed by `change`.
fn changed(change: impl FnOnce(&mut Rev)) -> Rev {
    let mut rev = next();
    change(&mut rev);
    rev
}

fn assert_rejected(what: &str, rev: &Rev) {
    let encoded = rev.bytes();
    assert!(accepts_bytes(&encoded).is_err(), "accepted: {what}");
    assert!(
        passes_content_checks(&encoded).is_err(),
        "passes the content checks: {what}"
    );
}

#[test]
fn a_revision_without_an_admin_is_rejected() {
    let rev = changed(|rev| {
        rev.members[0].role = MEMBER;
        rev.members[1].role = MEMBER;
    });
    assert_rejected("no admin", &rev);
}

#[test]
fn a_repeated_account_is_rejected() {
    let rev = changed(|rev| {
        rev.members
            .push(member(CAROL, "carol2", MEMBER, CAROL_MASTER))
    });
    assert_rejected("carol's account twice", &rev);
}

#[test]
fn a_repeated_device_is_rejected() {
    let rev = changed(|rev| {
        rev.service_agents
            .push(service_agent(BOB_CI, "ops", "test-bot"));
    });
    assert_rejected("a service agent's device twice", &rev);
}

#[test]
fn a_repeated_channel_is_rejected() {
    let rev = changed(|rev| {
        let mut again = rev.channels[0].clone();
        again.members = vec![(BOB, false)];
        rev.channels.push(again);
    });
    assert_rejected("a channel twice", &rev);
}

#[test]
fn a_repeated_invite_is_rejected() {
    let rev = changed(|rev| rev.members[2].invite = Some(BOB_INVITE));
    assert_rejected("bob's invite for carol too", &rev);
}

#[test]
fn an_invalid_handle_is_rejected() {
    for handle in [
        "Carol", "carol_1", "-carol", "carol-", "", "car ol", "carol.x",
    ] {
        let rev = changed(|rev| rev.members[2].handle = handle.to_owned());
        assert_rejected(&format!("handle {handle:?}"), &rev);
    }
    let rev = changed(|rev| rev.members[2].handle = "a".repeat(64));
    assert_rejected("a handle of 64 characters", &rev);
}

#[test]
fn an_invalid_owner_is_rejected() {
    for owner in ["CI", "c_i", "-ci", "", "c/i"] {
        let rev = changed(|rev| rev.service_agents[0].owner = owner.to_owned());
        assert_rejected(&format!("owner {owner:?}"), &rev);
    }
}

#[test]
fn a_handle_used_twice_is_rejected() {
    let rev = changed(|rev| rev.members[2].handle = "bob".to_owned());
    assert_rejected("two members with the handle bob", &rev);
}

#[test]
fn an_owner_that_is_a_members_handle_is_rejected() {
    let rev = changed(|rev| rev.service_agents[0].owner = "carol".to_owned());
    assert_rejected("a service agent owned as carol", &rev);
}

#[test]
fn two_service_agents_with_one_owner_are_accepted() {
    let rev = changed(|rev| {
        rev.service_agents
            .push(service_agent(ALICE_SERVICE, "ci", "test-bot"));
    });
    accepts(&rev.roster()).unwrap();
    passes_content_checks(&rev.bytes()).unwrap();
}

#[test]
fn a_repeated_owner_and_agent_is_rejected() {
    let rev = changed(|rev| {
        rev.service_agents
            .push(service_agent(ALICE_SERVICE, "ci", "deploy-bot"));
    });
    assert_rejected("ci/deploy-bot twice", &rev);
}

#[test]
fn an_invalid_agent_or_space_name_is_rejected() {
    for agent in ["Deploy", "deploy_bot", "-bot", ""] {
        let rev = changed(|rev| rev.service_agents[0].agent = agent.to_owned());
        assert_rejected(&format!("agent {agent:?}"), &rev);
    }
    let rev = changed(|rev| rev.name = "Acme".to_owned());
    assert_rejected("space name \"Acme\"", &rev);
}

#[test]
fn a_repeated_account_or_service_in_a_channel_is_rejected() {
    let rev = changed(|rev| rev.channels[0].members.push((ALICE, false)));
    assert_rejected("alice twice in a channel", &rev);
    let rev = changed(|rev| rev.channels[0].services.push(device(BOB_CI)));
    assert_rejected("bob's ci twice in a channel", &rev);
}

#[test]
fn one_account_and_one_service_in_two_channels_are_accepted() {
    let rev = changed(|rev| {
        rev.channels[1].members.push((ALICE, false));
        rev.channels[1].services.push(device(BOB_CI));
    });
    accepts(&rev.roster()).unwrap();
    passes_content_checks(&rev.bytes()).unwrap();
}

#[test]
fn a_repeated_excluded_device_is_rejected() {
    let rev = changed(|rev| rev.members[2].excluded = vec![device(CAROL_LAPTOP); 2]);
    assert_rejected("carol's laptop excluded twice", &rev);

    let rev = changed(|rev| rev.members[2].excluded = vec![device(CAROL_LAPTOP)]);
    accepts(&rev.roster()).unwrap();
}

#[test]
fn a_service_agent_without_a_valid_service_certificate_on_a_members_account_is_rejected() {
    for (what, seed) in [
        ("no certificate", UNCERTIFIED),
        ("a client device", BOB_LAPTOP),
        ("a service device outside the space", DAVE_CI),
        (
            "a service device chained to another master key",
            BOB_ROGUE_CI,
        ),
    ] {
        let rev = changed(|rev| rev.service_agents[0].device = device(seed));
        assert_rejected(what, &rev);
    }
}

#[test]
fn a_service_agent_chained_to_a_members_master_key_but_certified_on_another_account_is_rejected() {
    let rev = changed(|rev| rev.service_agents[0].device = device(DAVE_BY_BOB_CI));
    assert_rejected(
        "a service device on dave's account chained to bob's master key",
        &rev,
    );
}

#[test]
fn a_service_agent_on_an_admins_account_is_accepted() {
    let rev = changed(|rev| {
        rev.service_agents
            .push(service_agent(ALICE_SERVICE, "ops", "deploy-bot"));
    });
    accepts(&rev.roster()).unwrap();
    passes_content_checks(&rev.bytes()).unwrap();
}

#[test]
fn a_channel_member_that_is_no_member_is_rejected() {
    let rev = changed(|rev| rev.channels[0].members.push((DAVE, true)));
    assert_rejected("dave in a channel", &rev);
}

#[test]
fn a_policy_whose_history_is_under_a_day_is_rejected() {
    for history in [0, 3_600, DAY - 1] {
        let rev = changed(|rev| rev.history = history);
        assert_rejected(&format!("history {history}"), &rev);
    }
}

#[test]
fn a_policy_whose_history_is_a_day_is_accepted() {
    let rev = changed(|rev| rev.history = DAY);
    accepts(&rev.roster()).unwrap();
    passes_content_checks(&rev.bytes()).unwrap();
}

#[test]
fn a_mode_of_a_later_version_and_unknown_fields_are_accepted() {
    let rev = changed(|rev| {
        rev.ceilings = [7, FULL];
        rev.extra = vec![(11, crate::cbor::text("later"))];
        rev.policy_extra = vec![(3, crate::cbor::Value::Uint(1))];
        rev.members[1].extra = vec![(9, crate::cbor::Value::Bool(false))];
    });
    accepts(&rev.roster()).unwrap();
    passes_content_checks(&rev.bytes()).unwrap();
}

#[test]
fn the_error_names_the_check_that_fails() {
    use skald_crypto::{RosterError, accept_next};

    let reason = |rev: &Rev| {
        accept_next(
            &current().roster(),
            &rev.roster(),
            &certs(),
            ALICE,
            &public(ALICE_MASTER),
        )
        .unwrap_err()
    };
    let carol = device(CAROL_LAPTOP);
    let cases = [
        (
            reason(&signed_by(CAROL_LAPTOP)),
            RosterError::Signer(carol),
            format!("the signer {carol} is no valid client or browser device of an admin"),
        ),
        (
            reason(&changed(|rev| rev.space = OTHER_SPACE)),
            RosterError::Space,
            "another space than the previous revision's".to_owned(),
        ),
        (
            reason(&changed(|rev| rev.revision = 4)),
            RosterError::Revision {
                expected: 3,
                actual: 4,
            },
            "revision 4, not 3".to_owned(),
        ),
        (
            reason(&changed(|rev| rev.prev = Some([0x77; 32]))),
            RosterError::Prev,
            "prev is not the hash of the previous revision".to_owned(),
        ),
        (
            reason(&changed(|rev| {
                rev.members[0].role = MEMBER;
                rev.members[1].role = MEMBER;
            })),
            RosterError::NoAdmin,
            "no member is an admin".to_owned(),
        ),
        (
            reason(&changed(|rev| rev.members[2].handle = "bob".to_owned())),
            RosterError::Repeated("members' handles"),
            "members' handles repeat an item".to_owned(),
        ),
        (
            reason(&changed(|rev| {
                rev.service_agents[0].owner = "carol".to_owned();
            })),
            RosterError::OwnerIsHandle("carol".parse().unwrap()),
            "the service agent owner carol is a member's handle".to_owned(),
        ),
        (
            reason(&changed(|rev| {
                rev.service_agents[0].device = device(DAVE_CI)
            })),
            RosterError::ServiceAgent(device(DAVE_CI)),
            format!(
                "the service agent {} has no valid service certificate on a member's account",
                device(DAVE_CI)
            ),
        ),
        (
            reason(&changed(|rev| rev.channels[0].members.push((DAVE, true)))),
            RosterError::ChannelMember {
                channel: CHANNEL,
                account: DAVE,
            },
            format!("{DAVE} in channel {CHANNEL} is not a member"),
        ),
    ];
    for (error, expected, message) in cases {
        assert_eq!(error, expected);
        assert_eq!(error.to_string(), message);
    }

    let mut renamed = next();
    renamed.name = "acme2".to_owned();
    let error = reason(&renamed);
    assert_eq!(
        (error.clone(), error.to_string()),
        (
            RosterError::Name,
            "another name than the previous revision's".to_owned()
        )
    );

    let mut last = current();
    last.revision = u64::MAX;
    let after_last = Rev {
        prev: Some(last.roster().hash()),
        ..last.clone()
    };
    let error = accept_next(
        &last.roster(),
        &after_last.roster(),
        &certs(),
        ALICE,
        &public(ALICE_MASTER),
    )
    .unwrap_err();
    assert_eq!(error, RosterError::LastRevision);
    assert_eq!(
        error.to_string(),
        "the previous revision has the highest number; none follows it"
    );

    let error = accept_next(
        &current().roster(),
        &next().signed_with(BOB_LAPTOP),
        &certs(),
        ALICE,
        &public(ALICE_MASTER),
    )
    .unwrap_err();
    assert_eq!(error, RosterError::Signature);
    assert_eq!(
        error.to_string(),
        "the signature does not verify with the signer's key"
    );
}
