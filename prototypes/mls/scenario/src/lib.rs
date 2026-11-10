//! Skald channel scenarios that drive each MLS prototype through one library-neutral [`Device`].
//!
//! A prototype implements [`Device`] on its library and runs the suite with [`scenario_tests!`].
//! Every step loads the device from its saved state and saves it afterwards, as one-shot mode does:
//! [`Fleet::step`]. Scenarios and findings: `docs/dev/mls-prototypes.md`.

pub mod identity;
pub mod scenarios;
pub mod state;
pub mod wire;

mod fleet;

pub use fleet::Fleet;
pub use identity::{DeviceId, Identity};

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

    /// A new KeyPackage as an encoded `MLSMessage`; the device keeps its private keys.
    fn key_package(&mut self) -> Result<Vec<u8>>;
    /// Creates a group at epoch 0 with the device as its only member.
    fn create_group(&mut self, group_id: &[u8]) -> Result<()>;
    /// Commits the Add of every KeyPackage. The Commit stays pending until
    /// [`Device::merge_pending_commit`].
    fn add(&mut self, group_id: &[u8], key_packages: &[Vec<u8>]) -> Result<Commit>;
    /// Merges the device's pending Commit: the group moves to its new epoch.
    fn merge_pending_commit(&mut self, group_id: &[u8]) -> Result<()>;
    /// Joins a group from a Welcome; returns its `group_id`.
    fn join(&mut self, welcome: &[u8]) -> Result<Vec<u8>>;
    /// Encrypts an application message as an encoded `MLSMessage`.
    fn encrypt(&mut self, group_id: &[u8], data: &[u8]) -> Result<Vec<u8>>;
    /// Processes a group's message from another member; merges a Commit at once.
    fn process(&mut self, group_id: &[u8], message: &[u8]) -> Result<Received>;

    /// The group's current epoch.
    fn epoch(&self, group_id: &[u8]) -> Result<u64>;
    /// The leaves of the group's members, in leaf order.
    fn members(&self, group_id: &[u8]) -> Result<Vec<Leaf>>;
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
    };
}
