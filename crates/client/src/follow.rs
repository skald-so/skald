//! Following a space's roster (`docs/protocol/membership.md`, "Roster"): before a command uses a
//! space, the device fetches the space's revisions and the devices of its accounts, and accepts the
//! revisions after the last one it accepted in order, with checks 1 to 4. A revision that fails,
//! and every revision built on it, has no effect: the device keeps the last revision it accepted,
//! warns its user and keeps working with it. Another valid revision with a number it accepted means
//! the server forked the roster: the device keeps its own and warns. It warns when a revision it
//! accepts changes a member's `master`, and when a member's device is revoked, or valid again, since
//! it last checked it ("Device checks"). A revision's signer is checked with the revocations of its
//! account.
//!
//! A device follows only the spaces it has an anchor for: revision 1 of a space it created, its
//! personal space included, which counts as accepted before the server takes it, or the latest
//! revision its link gave it. Any other revision counts as accepted only once the server gives it,
//! those the device signed too. A device whose earliest accepted revision is after revision 1 also
//! takes, without a warning, each earlier one the server gives whose hash is the `prev` of one it
//! took, back to revision 1: the history checks senders against them ("History entry").

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_crypto::{RosterError, accept_next, check_device, check_valid_device};
use skald_proto::{AccountId, Cbor as _, Cert, DeviceId, Member, Revocation, Roster, SpaceId};

use crate::connection::Connection;
use crate::devices::{self, text};
use crate::revocations;
use crate::state::{self, State};

/// How many times a run accepts a space's revisions again when another run accepted some first, or
/// signs a revision again when another came first.
pub const ATTEMPTS: usize = 5;

/// `RosterRevisions` in the API spec.
#[derive(Deserialize)]
struct Revisions {
    revisions: Vec<String>,
}

/// Whom the device trusts a space's revisions by: its own account and the account's master key it
/// pinned.
pub struct Own {
    pub account: AccountId,
    pub master: [u8; 32],
}

/// The devices of a list, such as of a space's accounts, as the server gives them: their IDs, the
/// certificates that decode, and the revocations that decode with those the device holds.
#[derive(Clone)]
pub struct SpaceDevices {
    pub ids: Vec<DeviceId>,
    pub certs: Vec<Cert>,
    pub revocations: Vec<Revocation>,
}

/// The master key a device checks the certificates of `account` against in `roster`: the one `own`
/// pinned for its own account, otherwise the `master` the revision gives the account's member.
pub fn master_of(roster: &Roster, own: &Own, account: AccountId) -> Option<[u8; 32]> {
    if account == own.account {
        Some(own.master)
    } else {
        roster.body().member(account).map(|member| member.master)
    }
}

impl SpaceDevices {
    /// The valid certificates of `member`'s devices, `member` of `roster`, that its `excluded` does
    /// not list, checked with the revocations against the master key [`master_of`] gives.
    pub fn of(&self, roster: &Roster, own: &Own, member: &Member) -> Vec<&Cert> {
        let Some(master) = master_of(roster, own, member.account) else {
            return Vec::new();
        };
        let excluded = |device: &DeviceId| {
            (member.excluded.as_ref()).is_some_and(|excluded| excluded.contains(device))
        };
        let checked = devices::check(&master, self);
        (checked.linked.into_iter())
            .filter(|cert| cert.body().account == member.account && !excluded(&cert.body().device))
            .collect()
    }
}

/// Fetches the revisions of `space`, the space the state holds, and the devices of its accounts, as
/// the session `token`, accepts the new revisions, records them in `state` and prints the warnings:
/// gives the space's last accepted revision, its devices and the revisions the server gives.
pub fn sync(
    connection: &mut Connection,
    state: &State,
    token: &str,
    space: &state::Space,
    own: &Own,
) -> Result<(Roster, SpaceDevices, Vec<Roster>), String> {
    let (id, name) = (space.id, &space.name);
    let mut warnings = Vec::new();
    let mut given = Vec::new();
    for (place, entry) in listed(connection, token, id)?.iter().enumerate() {
        match decoded(entry) {
            Some(roster) => given.push(roster),
            None => warnings.push(format!(
                "revision {} the server gives of the space {name} does not decode, so it has no \
                 effect",
                place + 1
            )),
        }
    }
    let devices = space_devices(connection, token, state, id)?;

    let held = read(state, id, name)?;
    let (current, followed) = settle(state, (id, name), held, &given, &devices, own)?;
    revocations::keep(state, &devices, |account| master_of(&current, own, account))?;
    let changed = device_changes(state, &current, &devices, own)?;
    for warning in warnings.iter().chain(&followed).chain(&changed) {
        eprintln!("skald: warning: {warning}");
    }
    Ok((current, devices, given))
}

/// Records in `state` whether each device of a member of `roster`, a space's last accepted
/// revision, is revoked, its certificate checked as [`devices::check`] does among `devices`
/// against the master key [`master_of`] gives; gives the warning of each revoked, or valid again,
/// since the device last checked it.
pub fn device_changes(
    state: &State,
    roster: &Roster,
    devices: &SpaceDevices,
    own: &Own,
) -> Result<Vec<String>, String> {
    let body = roster.body();
    let mut warnings = Vec::new();
    for member in body.members.iter() {
        let Some(master) = master_of(roster, own, member.account) else {
            continue;
        };
        let checked = devices::check(&master, devices);
        let whose = format!(
            "the member {} in the space {}",
            member.handle.as_str(),
            body.name.as_str()
        );
        warnings.extend(devices::statuses(
            state,
            &checked,
            Some(member.account),
            &whose,
        )?);
    }
    Ok(warnings)
}

/// The revisions of the space `id` the server gives the session `token` that decode.
pub fn given(connection: &mut Connection, token: &str, id: SpaceId) -> Result<Vec<Roster>, String> {
    Ok(listed(connection, token, id)?
        .iter()
        .filter_map(|entry| decoded(entry))
        .collect())
}

/// The revision `entry`, in base64url as the server gives it, if it decodes.
fn decoded(entry: &str) -> Option<Roster> {
    URL_SAFE_NO_PAD
        .decode(entry)
        .ok()
        .and_then(|bytes| Roster::from_cbor(&bytes).ok())
}

/// The revisions of the space `id` the server gives the session `token`, each in base64url as it
/// gives it.
pub fn listed(
    connection: &mut Connection,
    token: &str,
    id: SpaceId,
) -> Result<Vec<String>, String> {
    let body = connection
        .get_as(&format!("/spaces/{id}/revisions"), token)
        .map_err(text)?;
    let list: Revisions = serde_json::from_slice(&body)
        .map_err(|_| "unexpected answer from the server: a revision list without its revisions")?;
    Ok(list.revisions)
}

/// The devices of the accounts of the space `id`, as the server gives them to the session `token`,
/// with the revocations `state` holds.
pub fn space_devices(
    connection: &mut Connection,
    token: &str,
    state: &State,
    id: SpaceId,
) -> Result<SpaceDevices, String> {
    devices::fetch(connection, &format!("/spaces/{id}/devices"), token, state)
}

/// What `state` holds of the space `id` named `name`.
pub fn read(state: &State, id: SpaceId, name: &str) -> Result<state::Accepted, String> {
    state
        .accepted(id)
        .map_err(text)?
        .ok_or_else(|| format!("this device dropped the space {name} meanwhile"))
}

/// Accepts the revisions of `given` that follow `held`, what `state` held of the space `id` named
/// `name` when the run read it, with `devices`, its accounts', and `own`, and records them. While
/// another run accepted revisions of the space first, it accepts again from what `state` then
/// holds. Gives the space's last accepted revision and the warnings.
fn settle(
    state: &State,
    (id, name): (SpaceId, &str),
    mut held: state::Accepted,
    given: &[Roster],
    devices: &SpaceDevices,
    own: &Own,
) -> Result<(Roster, Vec<String>), String> {
    for _ in 0..ATTEMPTS {
        let last = decode(&held.last, name)?;
        let Followed {
            accepted,
            earlier,
            warnings,
        } = follow(&last, &held.hashes, given, devices, own);
        let mut recorded = earlier;
        let newest = match accepted.split_last() {
            None if recorded.is_empty() => return Ok((last, warnings)),
            None => last,
            Some((newest, before)) => {
                recorded.push((last.body().revision, last.hash()));
                recorded.extend(
                    before
                        .iter()
                        .map(|roster| (roster.body().revision, roster.hash())),
                );
                newest.clone()
            }
        };
        if state
            .accept(id, &held.last, &recorded, &newest.to_cbor())
            .map_err(text)?
        {
            return Ok((newest, warnings));
        }
        held = read(state, id, name)?;
    }
    Err(format!(
        "other skald runs kept accepting revisions of the space {name}; run the command again"
    ))
}

/// The revision of the space `name` in `bytes`, as the local state holds it.
pub fn decode(bytes: &[u8], name: &str) -> Result<Roster, String> {
    Roster::from_cbor(bytes)
        .map_err(|err| state::Error::Malformed(format!("the revision of the space {name}: {err}")))
        .map_err(text)
}

/// What following a space's revisions gives.
#[derive(Debug, Default, PartialEq)]
pub struct Followed {
    /// The revisions newly accepted, in order.
    pub accepted: Vec<Roster>,
    /// The number and hash of each revision newly taken before the earliest accepted one, as
    /// [`earlier`] gives them.
    pub earlier: Vec<(u64, [u8; 32])>,
    /// What the device warns its user of.
    pub warnings: Vec<String>,
}

/// Accepts the revisions of `given`, all those the server gives of a space, that follow `last`,
/// the space's last accepted revision, the hashes of the earlier ones it accepted in `hashes`, with
/// `devices`, its accounts', and the master key `own` pinned; takes those [`earlier`] gives.
pub fn follow(
    last: &Roster,
    hashes: &BTreeMap<u64, [u8; 32]>,
    given: &[Roster],
    devices: &SpaceDevices,
    own: &Own,
) -> Followed {
    let name = last.body().name.as_str();
    let accepts = |previous: &Roster, next: &Roster| {
        accept_next(previous, next, &devices.certs, own.account, &own.master)?;
        signer_unrevoked(previous, next.body().signer, devices, own)
    };
    let mut known = hashes.clone();
    known.insert(last.body().revision, last.hash());
    let mut followed = Followed {
        earlier: earlier(last, &known, given),
        ..Followed::default()
    };
    known.extend(followed.earlier.iter().copied());
    let mut sorted: Vec<&Roster> = given.iter().collect();
    sorted.sort_by_key(|roster| roster.body().revision);
    let mut current = last;
    for &roster in &sorted {
        let number = roster.body().revision;
        let hash = roster.hash();
        let next = current.body().revision.checked_add(1);
        match known.get(&number) {
            Some(accepted) if *accepted == hash => {}
            Some(_) if number == 1 => followed.warnings.push(format!(
                "the server gives another revision 1 of the space {name} than the one this device \
                 holds as its anchor; it has no effect"
            )),
            Some(_) => {
                // The revision this device accepted before it, if it holds it or the server gives
                // it.
                let previous = known.get(&(number - 1)).and_then(|accepted| {
                    std::iter::once(last)
                        .chain(&followed.accepted)
                        .chain(sorted.iter().copied())
                        .find(|roster| roster.hash() == *accepted)
                });
                followed.warnings.push(match previous.map(|previous| accepts(previous, roster)) {
                    Some(Ok(())) => format!(
                        "the server forked the roster of the space {name}: it gives another valid \
                         revision {number} than the one this device accepted, which it keeps"
                    ),
                    Some(Err(err)) => format!(
                        "revision {number} of the space {name} is another than the one this \
                         device accepted and fails: {err}; it has no effect"
                    ),
                    None => format!(
                        "revision {number} of the space {name} is another than the one this \
                         device accepted and builds on none it accepted; it has no effect"
                    ),
                });
            }
            None if Some(number) == next => match accepts(current, roster) {
                Ok(()) => {
                    followed.warnings.extend(changed_masters(current, roster));
                    known.insert(number, hash);
                    followed.accepted.push(roster.clone());
                    current = roster;
                }
                Err(err) => followed.warnings.push(format!(
                    "revision {number} of the space {name} fails: {err}; it has no effect, and \
                     this device keeps revision {}",
                    current.body().revision
                )),
            },
            // After the next revision, or before the earliest accepted one and off the chain of
            // `prev` back from it.
            None => followed.warnings.push(format!(
                "revision {number} of the space {name} builds on no revision this device \
                 accepted; it has no effect, and this device keeps revision {}",
                current.body().revision
            )),
        }
    }
    followed
}

/// The number and hash of each revision of `given` before the earliest of `known`, the revisions
/// accepted by number, that the device takes: going back from that earliest one, as `last` or
/// `given` holds it, each whose hash is the `prev` of the one after it, down to revision 1 or the
/// first that `given` lacks.
fn earlier(
    last: &Roster,
    known: &BTreeMap<u64, [u8; 32]>,
    given: &[Roster],
) -> Vec<(u64, [u8; 32])> {
    let Some((&first, &hash)) = known.first_key_value() else {
        return Vec::new();
    };
    let with = |number: u64, hash: [u8; 32]| {
        std::iter::once(last)
            .chain(given)
            .find(|roster| roster.body().revision == number && roster.hash() == hash)
    };
    let mut taken = Vec::new();
    let mut after = with(first, hash);
    while let Some(roster) = after {
        let (Some(prev), Some(number)) =
            (roster.body().prev, roster.body().revision.checked_sub(1))
        else {
            break;
        };
        after = with(number, prev);
        if after.is_some() {
            taken.push((number, prev));
        }
    }
    taken
}

/// Check 1 of "Roster" with the revocations of `devices`: the `signer` of the revision after
/// `previous`, whose certificate passes checks 1 and 2 against the master key of an admin's account
/// in `previous`, is not revoked.
fn signer_unrevoked(
    previous: &Roster,
    signer: DeviceId,
    devices: &SpaceDevices,
    own: &Own,
) -> Result<(), RosterError> {
    let revoked = previous.body().members.iter().any(|admin| {
        master_of(previous, own, admin.account).is_some_and(|master| {
            check_device(&master, &devices.certs, signer)
                .is_ok_and(|cert| cert.body().account == admin.account)
                && check_valid_device(&master, &devices.certs, &devices.revocations, signer)
                    .is_err()
        })
    });
    if revoked {
        return Err(RosterError::Signer(signer));
    }
    Ok(())
}

/// The warnings of `next`, accepted after `previous`, for each member whose `master` it changes.
fn changed_masters(previous: &Roster, next: &Roster) -> Vec<String> {
    let body = next.body();
    body.members
        .iter()
        .filter(|member| {
            previous
                .body()
                .member(member.account)
                .is_some_and(|before| before.master != member.master)
        })
        .map(|member| {
            format!(
                "revision {} of the space {} changes the master key of the member {}: only its \
                 devices with certificates from the new key count",
                body.revision,
                body.name.as_str(),
                member.handle.as_str()
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;
    use skald_crypto::{SigningKey, issue};
    use skald_proto::{CertBody, DeviceKind, Member, Name, Role, RosterBody, SpaceId, Unknown};

    /// An account with its master key and one device, certified by that key.
    struct Person {
        account: AccountId,
        handle: Name,
        master: SigningKey,
        key: SigningKey,
        cert: Cert,
    }

    impl Person {
        fn new(handle: &str) -> Self {
            let (account, master, key) = (
                AccountId::generate(),
                SigningKey::generate(),
                SigningKey::generate(),
            );
            let device = DeviceId::from_signing_key(&key.public_key());
            let body = CertBody {
                account,
                device,
                kind: DeviceKind::Client,
                signing: key.public_key(),
                hpke: [5; 32],
                issuer: None,
                issued: 1_800_000_000,
                ssh: None,
                name: format!("{handle}-laptop"),
                unknown: Unknown::default(),
            };
            let cert = issue(&body, &master, &key).expect("a certificate");
            Person {
                account,
                handle: handle.parse().expect("a handle"),
                master,
                key,
                cert,
            }
        }

        fn device(&self) -> DeviceId {
            self.cert.body().device
        }

        /// Its entry in a roster, as `role`.
        fn member(&self, role: Role) -> Member {
            Member {
                account: self.account,
                handle: self.handle.clone(),
                role,
                master: self.master.public_key(),
                invite: None,
                excluded: None,
                unknown: Unknown::default(),
            }
        }

        /// What it trusts revisions by.
        fn own(&self) -> Own {
            Own {
                account: self.account,
                master: self.master.public_key(),
            }
        }

        /// Revision 1 of the space `acme`, which it created.
        fn creates(&self) -> Roster {
            let body = RosterBody::first(
                SpaceId::generate(),
                "acme".parse().expect("a name"),
                self.account,
                self.handle.clone(),
                self.master.public_key(),
                self.device(),
                1_800_000_000,
            );
            skald_crypto::sign_roster(&body, &self.key).expect("a revision")
        }

        /// The revision after `previous` that its device signs, its members as `change` leaves
        /// them.
        fn signs(&self, previous: &Roster, change: impl FnOnce(&mut Vec<Member>)) -> Roster {
            let mut body = previous.body().clone();
            body.revision += 1;
            body.prev = Some(previous.hash());
            body.signer = self.device();
            let mut members = body.members.iter().cloned().collect();
            change(&mut members);
            body.members = skald_proto::NonEmpty::from_vec(members).expect("a member");
            skald_crypto::sign_roster(&body, &self.key).expect("a revision")
        }
    }

    /// Ada creates `acme`; its revision 2, by Ada, adds Bob as an admin and Carol as a member.
    struct Acme {
        ada: Person,
        bob: Person,
        carol: Person,
        first: Roster,
        second: Roster,
    }

    impl Acme {
        fn new() -> Self {
            let (ada, bob, carol) = (Person::new("ada"), Person::new("bob"), Person::new("carol"));
            let first = ada.creates();
            let second = ada.signs(&first, |members| {
                members.push(bob.member(Role::Admin));
                members.push(carol.member(Role::Member));
            });
            Acme {
                ada,
                bob,
                carol,
                first,
                second,
            }
        }

        fn certs(&self) -> Vec<Cert> {
            [&self.ada, &self.bob, &self.carol]
                .iter()
                .map(|person| person.cert.clone())
                .collect()
        }

        /// What Ada's device gets of `given` after `last`, with the hashes of the revisions before
        /// it.
        fn follow(&self, last: &Roster, given: &[&Roster]) -> Followed {
            let hashes = [&self.first, &self.second]
                .iter()
                .map(|roster| (roster.body().revision, roster.hash()))
                .filter(|(revision, _)| *revision < last.body().revision)
                .collect();
            let given: Vec<Roster> = given.iter().map(|roster| (*roster).clone()).collect();
            follow(last, &hashes, &given, &self.devices(), &self.ada.own())
        }

        /// Their devices, with `revocations`.
        fn devices_with(&self, revocations: Vec<Revocation>) -> SpaceDevices {
            let certs = self.certs();
            SpaceDevices {
                ids: certs.iter().map(|cert| cert.body().device).collect(),
                certs,
                revocations,
            }
        }

        fn devices(&self) -> SpaceDevices {
            self.devices_with(Vec::new())
        }
    }

    /// A state that follows `acme` at its revision 2, what a run read of it then, and revisions 3
    /// by Bob and 4 by Ada after it.
    struct Race {
        state: State,
        id: SpaceId,
        stale: state::Accepted,
        third: Roster,
        fourth: Roster,
        _dir: TestDir,
    }

    impl Race {
        fn new(acme: &Acme, name: &str) -> Self {
            let dir = TestDir::new(name);
            let state = State::open(&dir).expect("a state");
            let id = acme.first.body().space;
            let first = acme.first.to_cbor();
            state.add_space(id, "acme", &first).unwrap();
            let hashes = [(1, acme.first.hash())];
            assert!(
                state
                    .accept(id, &first, &hashes, &acme.second.to_cbor())
                    .unwrap()
            );
            let stale = state.accepted(id).unwrap().unwrap();
            let third = acme.bob.signs(&acme.second, |_| {});
            let fourth = acme.ada.signs(&third, |_| {});
            Race {
                state,
                id,
                stale,
                third,
                fourth,
                _dir: dir,
            }
        }

        /// What the state holds now.
        fn held(&self) -> state::Accepted {
            self.state.accepted(self.id).unwrap().unwrap()
        }
    }

    /// What a state holds once it accepted `before`, then `last`.
    fn accepted(last: &Roster, before: &[&Roster]) -> state::Accepted {
        state::Accepted {
            last: last.to_cbor(),
            hashes: before
                .iter()
                .map(|roster| (roster.body().revision, roster.hash()))
                .collect(),
        }
    }

    /// What `settle` gives of `acme` in `state` for a run given `given` that read `held`.
    fn settled(
        acme: &Acme,
        state: &State,
        held: state::Accepted,
        given: &[&Roster],
    ) -> (Roster, Vec<String>) {
        let given: Vec<Roster> = given.iter().map(|roster| (*roster).clone()).collect();
        let id = (acme.first.body().space, "acme");
        settle(state, id, held, &given, &acme.devices(), &acme.ada.own()).expect("settled")
    }

    /// Sets the role of `account` in `members`.
    fn set_role(members: &mut [Member], account: AccountId, role: Role) {
        if let Some(member) = members.iter_mut().find(|member| member.account == account) {
            member.role = role;
        }
    }

    #[test]
    fn revisions_after_the_last_are_accepted_in_order_without_a_warning() {
        let acme = Acme::new();
        let third = acme.bob.signs(&acme.second, |members| {
            set_role(members, acme.ada.account, Role::Member);
        });

        let followed = acme.follow(&acme.first, &[&third, &acme.first, &acme.second]);

        assert_eq!(
            followed,
            Followed {
                accepted: vec![acme.second.clone(), third.clone()],
                earlier: vec![],
                warnings: vec![],
            }
        );
        assert_eq!(
            acme.follow(&acme.second, &[&acme.first, &acme.second, &third])
                .accepted,
            vec![third]
        );
        assert_eq!(
            acme.follow(&acme.second, &[&acme.first, &acme.second]),
            Followed::default()
        );
    }

    #[test]
    fn revisions_before_the_earliest_accepted_on_its_prev_chain_are_taken_without_a_warning() {
        let acme = Acme::new();
        let third = acme.bob.signs(&acme.second, |_| {});
        let given = [acme.first.clone(), acme.second.clone(), third.clone()];

        // A device linked at revision 3 holds no earlier one.
        let followed = follow(
            &third,
            &BTreeMap::new(),
            &given,
            &acme.devices(),
            &acme.ada.own(),
        );

        assert_eq!(
            followed,
            Followed {
                accepted: vec![],
                earlier: vec![(2, acme.second.hash()), (1, acme.first.hash())],
                warnings: vec![],
            }
        );
    }

    #[test]
    fn revision_before_the_earliest_accepted_off_its_prev_chain_is_not_taken() {
        let acme = Acme::new();
        let third = acme.bob.signs(&acme.second, |_| {});
        let other_second = acme.ada.signs(&acme.first, |members| {
            set_role(members, acme.carol.account, Role::Admin);
        });

        let followed = follow(
            &third,
            &BTreeMap::new(),
            &[acme.first.clone(), other_second, third.clone()],
            &acme.devices(),
            &acme.ada.own(),
        );

        assert_eq!(followed.accepted, vec![]);
        assert_eq!(followed.earlier, vec![]);
        assert_eq!(
            followed.warnings,
            [1, 2]
                .map(|number| format!(
                    "revision {number} of the space acme builds on no revision this device \
                     accepted; it has no effect, and this device keeps revision 3"
                ))
                .to_vec()
        );
    }

    #[test]
    fn linked_device_records_the_revisions_it_takes_before_its_anchor_once() {
        let acme = Acme::new();
        let dir = TestDir::new("follow-linked");
        let state = State::open(&dir).expect("a state");
        let third = acme.bob.signs(&acme.second, |_| {});
        let id = acme.first.body().space;
        state.add_space(id, "acme", &third.to_cbor()).unwrap();
        let given = [&acme.first, &acme.second, &third];

        let held = state.accepted(id).unwrap().unwrap();
        assert_eq!(
            settled(&acme, &state, held, &given),
            (third.clone(), vec![])
        );
        let now = state.accepted(id).unwrap().unwrap();
        assert_eq!(now, accepted(&third, &[&acme.first, &acme.second]));

        assert_eq!(settled(&acme, &state, now.clone(), &given), (third, vec![]));
        assert_eq!(state.accepted(id).unwrap().unwrap(), now);
    }

    #[test]
    fn failing_revision_and_those_built_on_it_have_no_effect() {
        let acme = Acme::new();
        let by_carol = acme.carol.signs(&acme.second, |members| {
            set_role(members, acme.carol.account, Role::Admin);
        });
        let built = acme.bob.signs(&by_carol, |_| {});

        let followed = acme.follow(
            &acme.second,
            &[&acme.first, &acme.second, &by_carol, &built],
        );

        assert_eq!(followed.accepted, vec![]);
        let carol = acme.carol.device();
        assert_eq!(
            followed.warnings,
            vec![
                format!(
                    "revision 3 of the space acme fails: the signer {carol} is no valid client or \
                     browser device of an admin; it has no effect, and this device keeps revision 2"
                ),
                "revision 4 of the space acme builds on no revision this device accepted; it has \
                 no effect, and this device keeps revision 2"
                    .to_owned(),
            ]
        );
    }

    #[test]
    fn revision_by_a_revoked_device_of_an_admin_fails() {
        let acme = Acme::new();
        let third = acme.bob.signs(&acme.second, |_| {});
        let (account, device) = (acme.bob.account, acme.bob.device());
        let body = skald_proto::RevocationBody {
            account,
            device,
            keep: None,
            signer: Some(device),
            timestamp: 1_800_000_000,
            also: None,
            unknown: skald_proto::Unknown::default(),
        };
        let revocation = skald_crypto::sign_revocation(&body, &acme.bob.key).expect("a revocation");
        let mut forged = body.clone();
        forged.timestamp += 1;
        let forged = skald_crypto::sign_revocation(&forged, &acme.carol.key).expect("a revocation");
        let follow = |revocations: Vec<Revocation>| {
            let devices = acme.devices_with(revocations);
            follow(
                &acme.second,
                &BTreeMap::new(),
                std::slice::from_ref(&third),
                &devices,
                &acme.ada.own(),
            )
        };

        let revoked = follow(vec![revocation]);

        assert_eq!(revoked.accepted, vec![]);
        assert_eq!(
            revoked.warnings,
            vec![format!(
                "revision 3 of the space acme fails: the signer {device} is no valid client or \
                 browser device of an admin; it has no effect, and this device keeps revision 2"
            )]
        );
        assert_eq!(follow(vec![forged]).accepted, vec![third.clone()]);
    }

    #[test]
    fn valid_revision_with_the_number_of_a_failing_one_is_accepted() {
        let acme = Acme::new();
        let by_carol = acme.carol.signs(&acme.second, |_| {});
        let by_bob = acme.bob.signs(&acme.second, |_| {});

        let followed = acme.follow(&acme.second, &[&by_carol, &by_bob]);

        assert_eq!(followed.accepted, vec![by_bob]);
        assert_eq!(followed.warnings.len(), 1, "{followed:?}");
    }

    #[test]
    fn another_valid_revision_with_an_accepted_number_is_a_fork_and_the_own_one_is_kept() {
        let acme = Acme::new();
        let other = acme.ada.signs(&acme.first, |members| {
            members.push(acme.bob.member(Role::Member));
        });
        let built = acme.ada.signs(&other, |_| {});

        let followed = acme.follow(&acme.second, &[&acme.first, &other, &built]);

        assert_eq!(followed.accepted, vec![]);
        assert_eq!(
            followed.warnings,
            vec![
                "the server forked the roster of the space acme: it gives another valid revision 2 \
                 than the one this device accepted, which it keeps"
                    .to_owned(),
                "revision 3 of the space acme fails: prev is not the hash of the previous \
                 revision; it has no effect, and this device keeps revision 2"
                    .to_owned(),
            ]
        );
    }

    #[test]
    fn two_valid_revisions_with_one_number_in_one_list_are_a_fork_and_the_first_is_kept() {
        let acme = Acme::new();
        let by_bob = acme.bob.signs(&acme.second, |_| {});
        let by_ada = acme.ada.signs(&acme.second, |_| {});

        let followed = acme.follow(&acme.second, &[&by_bob, &by_ada]);

        assert_eq!(followed.accepted, vec![by_bob]);
        assert_eq!(
            followed.warnings,
            vec![
                "the server forked the roster of the space acme: it gives another valid revision 3 \
                 than the one this device accepted, which it keeps"
                    .to_owned()
            ]
        );
    }

    #[test]
    fn another_revision_with_an_accepted_number_that_fails_or_follows_none_is_no_fork() {
        let acme = Acme::new();
        let by_bob = acme.bob.signs(&acme.first, |_| {});
        let third = acme.ada.signs(&acme.second, |_| {});
        let other_third = acme.ada.signs(&by_bob, |_| {});

        let followed = acme.follow(&third, &[&acme.first, &by_bob, &other_third]);

        assert_eq!(followed.accepted, vec![]);
        let bob = acme.bob.device();
        assert_eq!(
            followed.warnings,
            vec![
                format!(
                    "revision 2 of the space acme is another than the one this device accepted \
                     and fails: the signer {bob} is no valid client or browser device of an \
                     admin; it has no effect"
                ),
                "revision 3 of the space acme is another than the one this device accepted and \
                 builds on none it accepted; it has no effect"
                    .to_owned(),
            ]
        );
    }

    #[test]
    fn another_revision_1_than_the_anchor_has_no_effect() {
        let acme = Acme::new();
        let other = acme.ada.creates();

        let followed = acme.follow(&acme.first, &[&other]);

        assert_eq!(
            followed.warnings,
            vec![
                "the server gives another revision 1 of the space acme than the one this device \
                 holds as its anchor; it has no effect"
                    .to_owned()
            ]
        );
    }

    #[test]
    fn accepted_revision_that_changes_a_members_master_is_warned_of() {
        let acme = Acme::new();
        let third = acme.bob.signs(&acme.second, |members| {
            for member in members
                .iter_mut()
                .filter(|member| member.role == Role::Member)
            {
                member.master = [9; 32];
            }
        });

        let followed = acme.follow(&acme.second, &[&third]);

        assert_eq!(followed.accepted, vec![third]);
        assert_eq!(
            followed.warnings,
            vec![
                "revision 3 of the space acme changes the master key of the member carol: only \
                 its devices with certificates from the new key count"
                    .to_owned()
            ]
        );
    }

    #[test]
    fn run_whose_space_another_run_moved_on_shows_what_the_state_holds_without_a_warning() {
        let acme = Acme::new();
        let race = Race::new(&acme, "follow-moved-on");
        let given = [&acme.first, &acme.second, &race.third, &race.fourth];

        // Another run moves the space from revision 2 to 4.
        assert_eq!(
            settled(&acme, &race.state, race.held(), &given),
            (race.fourth.clone(), vec![])
        );
        let now = race.held();
        assert_eq!(
            now,
            accepted(&race.fourth, &[&acme.first, &acme.second, &race.third])
        );

        assert_eq!(
            settled(&acme, &race.state, race.stale.clone(), &given),
            (race.fourth.clone(), vec![])
        );
        assert_eq!(race.held(), now);
    }

    #[test]
    fn run_whose_space_another_run_moved_to_another_fork_keeps_that_one() {
        let acme = Acme::new();
        let race = Race::new(&acme, "follow-other-fork");
        let other = acme.ada.signs(&acme.second, |_| {});

        // Another run, given another revision 3, accepts it.
        let given = [&acme.first, &acme.second, &other];
        assert_eq!(
            settled(&acme, &race.state, race.held(), &given),
            (other.clone(), vec![])
        );

        let given = [&acme.first, &acme.second, &race.third, &race.fourth];
        assert_eq!(
            settled(&acme, &race.state, race.stale.clone(), &given),
            (
                other.clone(),
                vec![
                    "the server forked the roster of the space acme: it gives another valid \
                     revision 3 than the one this device accepted, which it keeps"
                        .to_owned(),
                    "revision 4 of the space acme fails: prev is not the hash of the previous \
                     revision; it has no effect, and this device keeps revision 3"
                        .to_owned(),
                ]
            )
        );
        assert_eq!(race.held(), accepted(&other, &[&acme.first, &acme.second]));
    }
}
