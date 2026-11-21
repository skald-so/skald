//! `skald fingerprint` (`docs/protocol/membership.md`, "Account fingerprint"): shows the
//! fingerprint of the device's own account, from the master key it pinned, or of a member of a
//! space it follows, from its last accepted revision.
//!
//! - Run by its person at a terminal for a member, it then asks whether the fingerprint matches
//!   what the member's own device shows, as `skald link` takes its person's answers
//!   ([`crate::link_code::at_terminal`]), and on yes marks the account verified for that master key.
//!   Through a pipe or as an agent it only shows the fingerprint: such a run never takes [`run`].
//! - Each step around the person's answer goes to the helper, or runs here holding the local
//!   state's lock, as other commands do ([`Step`]). The answer marks the master key the run showed:
//!   if a revision changed it meanwhile, nothing is marked.
//! - `skald members` shows which accounts are verified; an accepted revision that changes a
//!   verified account's `master` marks it no longer verified and warns ([`crate::follow`]).

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use skald_crypto::account_fingerprint;
use skald_proto::{AccountId, Roster, VersionRange};

use crate::connection::printable;
use crate::devices::text;
use crate::follow::{self, Own};
use crate::person::Person;
use crate::secrets;
use crate::spaces::{self, Scope, Synced};
use crate::state::State;

/// A step of `skald fingerprint` around its person's answer, which its process hands to the helper
/// or runs here holding the local state's lock.
#[derive(Serialize, Deserialize)]
pub enum Step {
    /// Accepts the revisions the server has of the space named `space`.
    Sync { space: String },
    /// Marks the account `account` of the member `handle` of the space named `space` verified for
    /// the master key `master`, unless its last accepted revision gives the member another.
    Verify {
        space: String,
        handle: String,
        account: String,
        master: [u8; 32],
    },
}

/// A member's account and the master key its fingerprint takes.
struct Shown {
    account: AccountId,
    master: [u8; 32],
    /// Whether it is the device's own account.
    own: bool,
}

/// `skald fingerprint`: shows the fingerprint of the account of the device of the machine whose
/// state is in `dir`, from the master key it pinned.
pub fn own(dir: &Path) -> Result<(), String> {
    let state = State::open(dir).map_err(text)?;
    let own = pinned(&state)?;
    println!(
        "Your account's fingerprint: {}",
        account_fingerprint(own.account, &own.master)
    );
    Ok(())
}

/// `skald fingerprint <space> <handle>` that marks nothing, as an agent or through a pipe: shows
/// the fingerprint of the member `handle` of `synced`'s space.
pub fn show(synced: &Synced, handle: &str) -> Result<(), String> {
    let space = synced.space.name.as_str();
    let shown = member(&synced.roster, &synced.follower.own, space, handle)?;
    println!("{}", line(space, handle, &shown));
    Ok(())
}

/// `skald fingerprint <space> <handle>` by its person at a terminal, on the machine whose state is
/// in `dir`: shows the member's fingerprint after the `Sync` step, asks whether it matches and on
/// yes has `step` run the `Verify` step ([`Step`]); gives the exit code.
pub fn run(
    dir: &Path,
    space: &str,
    handle: &str,
    mut step: impl FnMut(Step) -> ExitCode,
) -> ExitCode {
    let synced = step(Step::Sync {
        space: space.to_owned(),
    });
    if synced != ExitCode::SUCCESS {
        return synced;
    }
    let shown = match State::open(dir)
        .map_err(text)
        .and_then(|state| accepted(&state, space, handle))
    {
        Ok(shown) => shown,
        Err(err) => return crate::exit(Err(err)),
    };
    println!("{}", line(space, handle, &shown));
    if shown.own {
        return ExitCode::SUCCESS;
    }
    let question = format!(
        "Does it match what skald fingerprint shows on {}'s own device?",
        printable(handle)
    );
    match Person::Terminal.ask(&question) {
        Ok(Some(true)) => step(Step::Verify {
            space: space.to_owned(),
            handle: handle.to_owned(),
            account: shown.account.to_string(),
            master: shown.master,
        }),
        Ok(Some(false)) => {
            println!(
                "Not verified. If they differ, the key may be swapped: trust {} only once they \
                 match.",
                printable(handle)
            );
            ExitCode::SUCCESS
        }
        Ok(None) => crate::exit(Err(
            "the terminal's input ended; nothing is verified".to_owned()
        )),
        Err(err) => crate::exit(Err(err)),
    }
}

/// Runs `step` of `skald fingerprint` on the machine whose state is in `dir`, its secrets in
/// `storage`.
pub fn step(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    step: Step,
) -> Result<(), String> {
    match step {
        Step::Sync { space } => {
            spaces::synced(dir, storage, speaks, timeout, &space, Scope::All).map(drop)
        }
        Step::Verify {
            space,
            handle,
            account,
            master,
        } => {
            let state = State::open(dir).map_err(text)?;
            let shown = accepted(&state, &space, &handle)?;
            if shown.own || shown.account.to_string() != account || shown.master != master {
                return Err(format!(
                    "the master key of {} changed meanwhile, so nothing is verified: run skald \
                     fingerprint {} {} again",
                    printable(&handle),
                    printable(&space),
                    printable(&handle)
                ));
            }
            state
                .set_verified(shown.account, &shown.master, true)
                .map_err(text)?;
            println!("{} is verified on this device.", printable(&handle));
            Ok(())
        }
    }
}

/// What the run shows of the member `handle`'s fingerprint in the space named `space`.
fn line(space: &str, handle: &str, shown: &Shown) -> String {
    format!(
        "The fingerprint of {} in the space {}: {}",
        printable(handle),
        printable(space),
        account_fingerprint(shown.account, &shown.master)
    )
}

/// The member `handle` of the space named `space`, by the last revision of it `state` accepted.
fn accepted(state: &State, space: &str, handle: &str) -> Result<Shown, String> {
    let own = pinned(state)?;
    let followed = (state.spaces().map_err(text)?.into_iter())
        .find(|followed| followed.name == space)
        .ok_or_else(|| {
            format!(
                "this device follows no space named \"{}\"",
                printable(space)
            )
        })?;
    let roster = follow::decode(&followed.accepted, space)?;
    member(&roster, &own, space, handle)
}

/// The member `handle` of `roster`, the last accepted revision of the space named `space`, with
/// the master key `own` pinned for its own account, and the one `roster` gives any other.
fn member(roster: &Roster, own: &Own, space: &str, handle: &str) -> Result<Shown, String> {
    let member = (roster.body().members.iter())
        .find(|member| member.handle.as_str() == handle)
        .ok_or_else(|| {
            format!(
                "the space {} has no member {}",
                printable(space),
                printable(handle)
            )
        })?;
    let own_account = member.account == own.account;
    Ok(Shown {
        account: member.account,
        master: if own_account {
            own.master
        } else {
            member.master
        },
        own: own_account,
    })
}

/// The device's own account and the master key it pinned, once it holds its certificate.
fn pinned(state: &State) -> Result<Own, String> {
    let (_, master, cert) = spaces::certified(state)?
        .ok_or_else(|| "this device has no certificate yet: run skald sign-in".to_owned())?;
    Ok(Own {
        account: cert.body().account,
        master,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spaces::tests::{first, member as entry};
    use skald_crypto::SigningKey;

    #[test]
    fn a_member_takes_the_rosters_master_key_and_the_own_account_the_pinned_one() {
        let (ada, bob) = (AccountId::generate(), AccountId::generate());
        let (pinned, bobs) = ([1; 32], [2; 32]);
        let mut body = first(ada).body().clone();
        body.revision = 2;
        body.members.push(entry(bob, "bob", bobs));
        let roster = skald_crypto::sign_roster(&body, &SigningKey::generate()).unwrap();
        let own = Own {
            account: ada,
            master: pinned,
        };

        let shown = member(&roster, &own, "acme", "bob").unwrap();
        let mine = member(&roster, &own, "acme", "ada").unwrap();

        assert_eq!((shown.account, shown.master, shown.own), (bob, bobs, false));
        assert_eq!((mine.account, mine.master, mine.own), (ada, pinned, true));
        assert_eq!(
            member(&roster, &own, "acme", "carol").err().unwrap(),
            "the space acme has no member carol"
        );
        assert!(
            line("acme", "bob", &shown).ends_with(&account_fingerprint(bob, &bobs)),
            "{}",
            line("acme", "bob", &shown)
        );
    }
}
