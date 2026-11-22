//! Notices of new roster revisions and of devices waiting for linking: `docs/protocol/delivery.md`,
//! "Frames". Once the server has stored a revision, each connection of a device of an account that
//! gets the space, on this instance, sends a `space-notice`; once a run of a link code opens, each
//! connection of a device session of the run's account sends a `link-notice`. A notice is no
//! mailbox entry: a connection that is not open then gets none. A long-poll answers a notice of
//! each revision stored, and a `link-notice` if a run opened, since its device's previous long-poll
//! answer ([`since_last_answer`], [`crate::link_runs::since_last_answer`]).

use std::collections::HashMap;
use std::mem;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use skald_proto::{AccountId, DeviceId, Push, SpaceId};
use sqlx::PgPool;
use tokio::sync::Notify;

use crate::service_tokens::{self, Token};

/// The connections of this instance that get notices, by their device's account.
#[derive(Default)]
pub(crate) struct Notices {
    by_account: Mutex<HashMap<AccountId, Vec<Connection>>>,
    next: AtomicU64,
}

/// A connection that gets notices: its number and its notices not sent yet.
type Connection = (u64, Arc<Pending>);

/// The notices a connection has not sent yet.
#[derive(Default)]
struct Pending {
    noticed: Mutex<Noticed>,
    added: Notify,
}

/// Notices not sent yet: the spaces they name, in the order they came, and whether a device of the
/// account waits for linking.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Noticed {
    pub(crate) spaces: Vec<SpaceId>,
    pub(crate) linking: bool,
}

impl Noticed {
    /// Its frames for a connection of `token`, if it has one: the `space-notice` of each space the
    /// token sees, and a `link-notice` for a device session only.
    pub(crate) fn pushes(self, token: Option<&Token>) -> Vec<Push> {
        let linking = (self.linking && token.is_none()).then_some(Push::LinkNotice);
        (service_tokens::seen(token, self.spaces).into_iter())
            .map(|space| Push::SpaceNotice { space })
            .chain(linking)
            .collect()
    }
}

impl Notices {
    /// Gives each connection of a device of each of `accounts` a notice of `space`.
    pub(crate) fn notify(&self, accounts: impl IntoIterator<Item = AccountId>, space: SpaceId) {
        let by_account = self.by_account.lock().expect("the notices");
        for account in accounts {
            for (_, pending) in by_account.get(&account).into_iter().flatten() {
                let mut noticed = pending.noticed.lock().expect("a connection's notices");
                // One notice of a space tells the client all it needs to fetch.
                if !noticed.spaces.contains(&space) {
                    noticed.spaces.push(space);
                }
                pending.added.notify_one();
            }
        }
    }

    /// Gives each connection of a device of `account` a notice that one of its devices waits for
    /// linking.
    pub(crate) fn link(&self, account: AccountId) {
        let by_account = self.by_account.lock().expect("the notices");
        for (_, pending) in by_account.get(&account).into_iter().flatten() {
            pending
                .noticed
                .lock()
                .expect("a connection's notices")
                .linking = true;
            pending.added.notify_one();
        }
    }

    /// The accounts with connections that get notices.
    pub(crate) fn accounts(&self) -> Vec<AccountId> {
        let by_account = self.by_account.lock().expect("the notices");
        by_account.keys().copied().collect()
    }

    /// The notices of a connection of a device of `account`, from now on.
    pub(crate) fn listen(self: &Arc<Self>, account: AccountId) -> Listening {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let pending = Arc::<Pending>::default();
        self.by_account
            .lock()
            .expect("the notices")
            .entry(account)
            .or_default()
            .push((id, Arc::clone(&pending)));
        Listening {
            notices: Arc::clone(self),
            account,
            id,
            pending,
        }
    }
}

/// A connection's notices; dropping it stops them.
pub(crate) struct Listening {
    notices: Arc<Notices>,
    account: AccountId,
    id: u64,
    pending: Arc<Pending>,
}

impl Listening {
    /// Waits for notices, and gives them.
    pub(crate) async fn next(&self) -> Noticed {
        loop {
            let noticed =
                mem::take(&mut *self.pending.noticed.lock().expect("a connection's notices"));
            if noticed != Noticed::default() {
                return noticed;
            }
            // A notice added since the take leaves a permit, so this returns at once.
            self.pending.added.notified().await;
        }
    }
}

impl Drop for Listening {
    fn drop(&mut self) {
        let mut by_account = self.notices.by_account.lock().expect("the notices");
        if let Some(listening) = by_account.get_mut(&self.account) {
            listening.retain(|(id, _)| *id != self.id);
            if listening.is_empty() {
                by_account.remove(&self.account);
            }
        }
    }
}

/// The spaces of `account` with a revision that lists it stored since the previous call for
/// `device`, each space of `account` at the first call; records their latest such revisions, so
/// that the next call gives only later ones.
pub(crate) async fn since_last_answer(
    db: &PgPool,
    device: DeviceId,
    account: AccountId,
) -> Result<Vec<SpaceId>, sqlx::Error> {
    let found: Vec<(String,)> = sqlx::query_as(
        "WITH latest AS (
             SELECT space_id, max(revision) AS revision FROM roster_members
             WHERE account_id = $2::uuid GROUP BY space_id
         )
         INSERT INTO long_poll_revisions (device_id, space_id, revision)
         SELECT $1::uuid, space_id, revision FROM latest
         ON CONFLICT (device_id, space_id) DO UPDATE SET revision = EXCLUDED.revision
         WHERE long_poll_revisions.revision < EXCLUDED.revision
         RETURNING space_id::text",
    )
    .bind(device.to_string())
    .bind(account.to_string())
    .fetch_all(db)
    .await?;
    let mut spaces: Vec<SpaceId> = found
        .into_iter()
        .map(|(space,)| space.parse().expect("a stored space ID"))
        .collect();
    spaces.sort();
    Ok(spaces)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn account(byte: u8) -> AccountId {
        AccountId::from_bytes([byte; 16])
    }

    fn space(byte: u8) -> SpaceId {
        SpaceId::from_bytes([byte; 16])
    }

    /// The spaces `listening` has notices of now.
    fn now(listening: &Listening) -> Vec<SpaceId> {
        mem::take(&mut listening.pending.noticed.lock().unwrap().spaces)
    }

    /// Whether `listening` has a `link-notice` now.
    fn linking(listening: &Listening) -> bool {
        mem::take(&mut listening.pending.noticed.lock().unwrap().linking)
    }

    /// Two connections of account 1 and one of account 2, listening to `notices`.
    fn two_accounts(notices: &Arc<Notices>) -> [Listening; 3] {
        [account(1), account(1), account(2)].map(|account| notices.listen(account))
    }

    #[test]
    fn a_notice_reaches_each_connection_of_its_accounts_only() {
        let notices = Arc::new(Notices::default());
        let [first, second, other] = two_accounts(&notices);

        notices.notify([account(1)], space(1));

        assert_eq!(now(&first), vec![space(1)]);
        assert_eq!(now(&second), vec![space(1)]);
        assert_eq!(now(&other), Vec::new());
    }

    #[test]
    fn notices_of_one_space_are_sent_once() {
        let notices = Arc::new(Notices::default());
        let listening = notices.listen(account(1));

        notices.notify([account(1)], space(1));
        notices.notify([account(1)], space(2));
        notices.notify([account(1)], space(1));

        assert_eq!(now(&listening), vec![space(1), space(2)]);
    }

    #[test]
    fn a_dropped_connection_gets_no_notice() {
        let notices = Arc::new(Notices::default());
        let first = notices.listen(account(1));
        let second = notices.listen(account(1));

        drop(first);
        assert_eq!(notices.by_account.lock().unwrap()[&account(1)].len(), 1);
        notices.notify([account(1)], space(1));
        assert_eq!(now(&second), vec![space(1)]);
        drop(second);

        assert!(notices.by_account.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn next_waits_for_a_notice() {
        let notices = Arc::new(Notices::default());
        let listening = notices.listen(account(1));
        let waiting = tokio::spawn({
            let notices = Arc::clone(&notices);
            async move {
                tokio::task::yield_now().await;
                notices.notify([account(1)], space(1));
            }
        });

        let next = tokio::time::timeout(Duration::from_secs(5), listening.next()).await;
        assert_eq!(next.expect("a notice within 5 s").spaces, vec![space(1)]);
        waiting.await.unwrap();
    }

    #[test]
    fn a_link_notice_reaches_each_connection_of_its_account_only() {
        let notices = Arc::new(Notices::default());
        let [first, second, other] = two_accounts(&notices);

        notices.link(account(1));

        assert!(linking(&first));
        assert!(linking(&second));
        assert!(!linking(&other));
        assert_eq!(now(&first), Vec::new());
    }

    #[tokio::test]
    async fn next_gives_a_link_notice() {
        let notices = Arc::new(Notices::default());
        let listening = notices.listen(account(1));
        notices.link(account(1));

        let next = tokio::time::timeout(Duration::from_secs(5), listening.next()).await;
        let noticed = next.expect("a notice within 5 s");
        assert!(noticed.linking);
        assert!(noticed.spaces.is_empty());
    }
}
