//! The account records of the members of the spaces the device follows
//! (`docs/protocol/membership.md`, "Account" and "Device checks").
//!
//! - The device checks each record of a member account, those the server gives with a space's
//!   devices and the one it took before, with that account's `master`: the one it pinned for its
//!   own, the roster's for others. Of the valid ones it takes the one with the highest `revision`.
//! - It keeps the record it takes in its local state, and warns when the record changes the
//!   account's `recovery`.
//! - The recovery key of the record it takes is the one it wraps each clean epoch's key to
//!   ([`crate::wraps`]); an account without a valid record gets no wrap.

use skald_crypto::check_account;
use skald_proto::{AccountId, AccountRecord, Cbor as _, Member, Roster};

use crate::devices::text;
use crate::follow::{Own, SpaceDevices, master_of};
use crate::state::State;

/// The valid record of `account` with the highest `revision` among `records`, checked with
/// `master`; of several with that `revision`, the first.
fn best<'r>(
    account: AccountId,
    master: &[u8; 32],
    records: impl IntoIterator<Item = &'r AccountRecord>,
) -> Option<&'r AccountRecord> {
    let mut best: Option<&AccountRecord> = None;
    for record in records {
        let valid = record.body().account == account && check_account(record, master).is_ok();
        if valid && best.is_none_or(|best| record.body().revision > best.body().revision) {
            best = Some(record);
        }
    }
    best
}

/// The record `state` holds of `account`, if it decodes.
fn held(state: &State, account: AccountId) -> Result<Option<AccountRecord>, String> {
    let held = state.account_record(account).map_err(text)?;
    Ok(held.and_then(|bytes| AccountRecord::from_cbor(&bytes).ok()))
}

/// The record of `account`, a member of `roster`, the device takes by `roster` and `devices`, the
/// space's: of the one `state` holds and those `devices` gives, the valid one with the highest
/// `revision`, the held one first; none if none is valid.
pub fn of(
    state: &State,
    (roster, devices): (&Roster, &SpaceDevices),
    own: &Own,
    account: AccountId,
) -> Result<Option<AccountRecord>, String> {
    let Some(master) = master_of(roster, own, account) else {
        return Ok(None);
    };
    let held = held(state, account)?;
    Ok(best(account, &master, held.iter().chain(&devices.records)).cloned())
}

/// Takes the record of each member of `roster`, a space's last accepted revision, by `roster` and
/// `devices`, the space's, as [`of`] says, and keeps it in `state`; gives the warning of each that
/// changes its account's `recovery`.
pub fn settle(
    state: &State,
    synced: (&Roster, &SpaceDevices),
    own: &Own,
) -> Result<Vec<String>, String> {
    (synced.0.body().members.iter())
        .filter_map(|member| take(state, synced, own, member).transpose())
        .collect()
}

/// Takes the record of `member` of `roster` as [`settle`] says; gives the warning if it changes
/// the account's `recovery`.
fn take(
    state: &State,
    (roster, devices): (&Roster, &SpaceDevices),
    own: &Own,
    member: &Member,
) -> Result<Option<String>, String> {
    let account = member.account;
    let Some(taken) = of(state, (roster, devices), own, account)? else {
        return Ok(None);
    };
    let held = held(state, account)?;
    if held.as_ref().map(AccountRecord::to_cbor) == Some(taken.to_cbor()) {
        return Ok(None);
    }
    state
        .set_account_record(account, &taken.to_cbor())
        .map_err(text)?;
    let changed = held.is_some_and(|held| held.body().recovery != taken.body().recovery);
    Ok(changed.then(|| {
        format!(
            "the account record of the member {} ({account}) in the space {} names a new recovery \
             key, revision {}; this device wraps the channels' history keys to it from now on",
            member.handle.as_str(),
            roster.body().name.as_str(),
            taken.body().revision
        )
    }))
}

#[cfg(test)]
mod tests {
    use skald_crypto::{HpkeKey, SigningKey, sign_account};
    use skald_proto::{AccountBody, Unknown};

    use super::*;

    /// A record of `account` with `master`'s public key, `recovery` and `revision`, signed by
    /// `signer`.
    fn record(
        account: AccountId,
        master: &SigningKey,
        (recovery, revision): ([u8; 32], u64),
        signer: &SigningKey,
    ) -> AccountRecord {
        let body = AccountBody {
            account,
            master: master.public_key(),
            recovery,
            revision,
            unknown: Unknown::default(),
        };
        sign_account(&body, signer).unwrap()
    }

    /// The valid record with the highest revision wins, the first of equals; one of another
    /// account, one naming another master key and one signed by another key never do.
    #[test]
    fn the_valid_record_with_the_highest_revision_is_taken() {
        let account = AccountId::generate();
        let master = SigningKey::generate();
        let other = SigningKey::generate();
        let key = |seed| HpkeKey::from_recovery_key(&[seed; 32]).public_key();
        let first = record(account, &master, (key(1), 1), &master);
        let second = record(account, &master, (key(2), 2), &master);
        let tied = record(account, &master, (key(3), 2), &master);
        let forged = record(account, &master, (key(4), 3), &other);
        let elsewhere = record(account, &other, (key(5), 4), &other);
        let foreign = record(AccountId::generate(), &master, (key(6), 5), &master);
        let pinned = master.public_key();

        let taken = best(
            account,
            &pinned,
            [&first, &second, &tied, &forged, &elsewhere, &foreign],
        );

        assert_eq!(taken.map(AccountRecord::to_cbor), Some(second.to_cbor()));
        assert!(best(account, &pinned, [&forged, &elsewhere, &foreign]).is_none());
        let taken = best(account, &other.public_key(), [&first, &elsewhere]);
        assert_eq!(taken.map(AccountRecord::to_cbor), Some(elsewhere.to_cbor()));
    }
}
