//! Skald channel scenarios that drive each MLS prototype through one library-neutral [`Device`].
//!
//! A prototype implements [`Device`] on its library and runs the suite with [`scenario_tests!`].
//! Every step loads the device from its saved state and saves it afterwards, as one-shot mode does:
//! [`Fleet::step`]. Scenarios and findings: `docs/dev/mls-prototypes.md`.

pub mod cbor;
pub mod certificate;
pub mod identity;
pub mod key_packages;
pub mod object;
pub mod revocation;
pub mod roster;
pub mod scenarios;
pub mod state;
pub mod wire;

mod fleet;

pub use fleet::Fleet;
pub use identity::{DeviceId, Identity};
pub use roster::{Admitted, Roster};

/// An error of a prototype or its library.
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The leaf of a group member: its `basic` credential's `identity` and its `signature_key`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Leaf {
    pub identity: Vec<u8>,
    pub signature_key: Vec<u8>,
}

/// A Commit made by a device, pending until the device merges it.
#[derive(Debug, Clone)]
pub struct Commit {
    /// The encoded `MLSMessage` with the Commit.
    pub commit: Vec<u8>,
    /// The encoded `MLSMessage` with the Welcome for the added devices, if it adds any.
    pub welcome: Option<Vec<u8>>,
}

/// The label and length of a channel's history key: `MLS-Exporter("skald history", "", 16)`
/// (`docs/protocol/membership.md`, "History key").
pub const HISTORY_LABEL: &str = "skald history";
pub type HistoryKey = [u8; 16];

/// A Commit by another member that the device has processed but not merged yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    /// The leaf of its sender.
    pub sender: Leaf,
    /// The leaves it removes, as they are in the current epoch.
    pub removed: Vec<Leaf>,
    /// The leaves of the devices it adds, from their KeyPackages.
    pub added: Vec<Leaf>,
    pub authenticated_data: Vec<u8>,
}

/// What a device got from processing a group's message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Received {
    /// A decrypted application message and the leaf of its sender.
    Application { sender: Leaf, data: Vec<u8> },
    /// A Commit by another member, merged: the device is in the Commit's new epoch.
    Commit { sender: Leaf },
}

/// One device on one MLS library, with the Skald profile (`docs/protocol/membership.md`,
/// "MLS profile"). Groups are named by their `group_id`, the channel ID.
///
/// The device's signing key comes from outside the library: [`Device::new`] and [`Device::load`]
/// get it from the [`Identity`], like a key kept in the OS key store.
pub trait Device: Sized {
    /// The library's name in test output and the report.
    const LIBRARY: &'static str;

    /// A new device without groups or KeyPackages.
    fn new(identity: &Identity) -> Result<Self>;
    /// The device from the state [`Device::save`] gave.
    fn load(identity: &Identity, state: &[u8]) -> Result<Self>;
    /// The device's whole MLS state: groups, pending Commits and KeyPackage private keys.
    fn save(&self) -> Result<Vec<u8>>;

    /// A new KeyPackage as an encoded `MLSMessage`, with [`key_packages::lifetime`] from now and
    /// the `last_resort` extension in its capabilities. The device keeps its private keys until a
    /// Welcome uses it.
    fn key_package(&mut self) -> Result<Vec<u8>>;
    /// A new last-resort KeyPackage: as [`Device::key_package`], with the `last_resort`
    /// extension. The device keeps its private keys after a Welcome uses it.
    fn last_resort_key_package(&mut self) -> Result<Vec<u8>>;
    /// Creates a group at epoch 0 with the device as its only member.
    fn create_group(&mut self, group_id: &[u8]) -> Result<()>;
    /// Commits the Add of every KeyPackage, each checked first as `docs/protocol/membership.md`,
    /// "KeyPackages", says: its MLS signature verifies, its `lifetime` holds, its ciphersuite is
    /// `0x0001`, and its `identity` and `signature_key` are an admitted device's. Fails, and
    /// commits nothing, if one fails. The Commit stays pending until
    /// [`Device::merge_pending_commit`].
    fn add(
        &mut self,
        group_id: &[u8],
        key_packages: &[Vec<u8>],
        admitted: &Admitted,
    ) -> Result<Commit>;
    /// Commits the Remove of the members whose credential's `identity` is in `devices`, with
    /// `authenticated_data`. Pending until [`Device::merge_pending_commit`].
    fn remove(
        &mut self,
        group_id: &[u8],
        devices: &[Vec<u8>],
        authenticated_data: &[u8],
    ) -> Result<Commit>;
    /// Commits an empty Commit, with an UpdatePath and no proposals, with `authenticated_data`.
    /// Pending until [`Device::merge_pending_commit`].
    fn empty_commit(&mut self, group_id: &[u8], authenticated_data: &[u8]) -> Result<Commit>;
    /// Merges the device's pending Commit: the group moves to its new epoch.
    fn merge_pending_commit(&mut self, group_id: &[u8]) -> Result<()>;
    /// Joins a group from a Welcome; returns its `group_id`.
    fn join(&mut self, welcome: &[u8]) -> Result<Vec<u8>>;
    /// Encrypts an application message as an encoded `MLSMessage`.
    fn encrypt(&mut self, group_id: &[u8], data: &[u8]) -> Result<Vec<u8>>;
    /// Processes a group's message from another member. A Commit is merged only if `check`
    /// passes on it: the device reads the Commit before it enters the new epoch.
    fn process_with(
        &mut self,
        group_id: &[u8],
        message: &[u8],
        check: impl FnOnce(&Staged) -> Result<()>,
    ) -> Result<Received>;
    /// Processes a group's message from another member; merges a Commit at once.
    fn process(&mut self, group_id: &[u8], message: &[u8]) -> Result<Received> {
        self.process_with(group_id, message, |_| Ok(()))
    }

    /// The group's current epoch.
    fn epoch(&self, group_id: &[u8]) -> Result<u64>;
    /// The leaves of the group's members, in leaf order.
    fn members(&self, group_id: &[u8]) -> Result<Vec<Leaf>>;
    /// The history key of the group's current epoch, from the MLS exporter with
    /// [`HISTORY_LABEL`].
    fn history_key(&self, group_id: &[u8]) -> Result<HistoryKey>;
}

/// Defines one `#[test]` per scenario of [`scenarios`] for the [`Device`] type `$device`.
#[macro_export]
macro_rules! scenario_tests {
    ($device:ty) => {
        #[test]
        fn creating_a_channel() {
            $crate::scenarios::creating_a_channel::<$device>();
        }

        #[test]
        fn member_adds_a_device() {
            $crate::scenarios::member_adds_a_device::<$device>();
        }

        #[test]
        fn members_exchange_messages() {
            $crate::scenarios::members_exchange_messages::<$device>();
        }

        #[test]
        fn replayed_message_is_rejected() {
            $crate::scenarios::replayed_message_is_rejected::<$device>();
        }

        #[test]
        fn removed_device_reads_nothing_later() {
            $crate::scenarios::removed_device_reads_nothing_later::<$device>();
        }

        #[test]
        fn revocations_are_read_before_merging() {
            $crate::scenarios::revocations_are_read_before_merging::<$device>();
        }

        #[test]
        fn empty_commit_starts_a_new_epoch() {
            $crate::scenarios::empty_commit_starts_a_new_epoch::<$device>();
        }

        #[test]
        fn history_key_rotates_with_each_epoch() {
            $crate::scenarios::history_key_rotates_with_each_epoch::<$device>();
        }

        #[test]
        fn device_keeps_key_packages() {
            $crate::scenarios::device_keeps_key_packages::<$device>();
        }

        #[test]
        fn key_packages_that_fail_a_check_are_refused() {
            $crate::scenarios::key_packages_that_fail_a_check_are_refused::<$device>();
        }

        #[test]
        fn devices_that_are_not_admitted_are_refused() {
            $crate::scenarios::devices_that_are_not_admitted_are_refused::<$device>();
        }

        #[test]
        fn last_resort_key_package_adds_to_two_groups() {
            $crate::scenarios::last_resort_key_package_adds_to_two_groups::<$device>();
        }

        #[test]
        fn device_added_without_admission_is_removed() {
            $crate::scenarios::device_added_without_admission_is_removed::<$device>();
        }
    };
}
