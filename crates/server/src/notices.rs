//! Notices of new roster revisions: `docs/protocol/delivery.md`, "Frames". Once the server has
//! stored a revision, each connection of a device of an account that gets the space, on this
//! instance, sends a `space-notice`. A notice is no mailbox entry: a connection that is not open
//! when the revision is stored gets none.

use std::collections::HashMap;
use std::mem;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use skald_proto::{AccountId, SpaceId};
use tokio::sync::Notify;

/// The connections of this instance that get notices, by their device's account.
#[derive(Default)]
pub(crate) struct Notices {
    by_account: Mutex<HashMap<AccountId, Vec<Connection>>>,
    next: AtomicU64,
}

/// A connection that gets notices: its number and its notices not sent yet.
type Connection = (u64, Arc<Pending>);

/// The spaces a connection has notices of that it has not sent yet.
#[derive(Default)]
struct Pending {
    spaces: Mutex<Vec<SpaceId>>,
    added: Notify,
}

impl Notices {
    /// Gives each connection of a device of each of `accounts` a notice of `space`.
    pub(crate) fn notify(&self, accounts: impl IntoIterator<Item = AccountId>, space: SpaceId) {
        let by_account = self.by_account.lock().expect("the notices");
        for account in accounts {
            for (_, pending) in by_account.get(&account).into_iter().flatten() {
                let mut spaces = pending.spaces.lock().expect("a connection's notices");
                // One notice of a space tells the client all it needs to fetch.
                if !spaces.contains(&space) {
                    spaces.push(space);
                }
                pending.added.notify_one();
            }
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
    /// Waits for notices, and gives the spaces they name, in the order they came.
    pub(crate) async fn next(&self) -> Vec<SpaceId> {
        loop {
            let spaces =
                mem::take(&mut *self.pending.spaces.lock().expect("a connection's notices"));
            if !spaces.is_empty() {
                return spaces;
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
        mem::take(&mut *listening.pending.spaces.lock().unwrap())
    }

    #[test]
    fn a_notice_reaches_each_connection_of_its_accounts_only() {
        let notices = Arc::new(Notices::default());
        let first = notices.listen(account(1));
        let second = notices.listen(account(1));
        let other = notices.listen(account(2));

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
        assert_eq!(next.expect("a notice within 5 s"), vec![space(1)]);
        waiting.await.unwrap();
    }
}
