//! Skald channel scenarios that drive each MLS prototype through one library-neutral [`Device`].
//!
//! A prototype implements [`Device`] on its library and runs the suite with [`scenario_tests!`].
//! Every step loads the device from its saved state and saves it afterwards, as one-shot mode does:
//! [`Fleet::step`]. Scenarios and findings: `docs/dev/mls-prototypes.md`.

pub mod browser;
pub mod cbor;
pub mod certificate;
pub mod delivery;
pub mod identity;
pub mod interop;
pub mod key_packages;
pub mod object;
pub mod revocation;
pub mod roster;
pub mod scenarios;
pub mod state;
pub mod wire;

mod fleet;

pub use delivery::{DeliveryService, Refusal};
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
    /// The encoded `MLSMessage` with the GroupInfo of the Commit's new epoch, with the
    /// `ratchet_tree` and `external_pub` extensions (`docs/protocol/membership.md`, "MLS profile").
    pub group_info: Vec<u8>,
}

/// The label and length of a channel's history key: `MLS-Exporter("skald history", "", 16)`
/// (`docs/protocol/membership.md`, "History key").
pub const HISTORY_LABEL: &str = "skald history";
pub type HistoryKey = [u8; 16];

/// A Commit by another member that the device has processed but not merged yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    /// The leaf of its sender: for an external Commit, the joiner's new leaf.
    pub sender: Leaf,
    /// The leaves it removes, as they are in the current epoch.
    pub removed: Vec<Leaf>,
    /// The leaves of the devices it adds, from their KeyPackages; for an external Commit, the
    /// joiner's new leaf, which members check as an Add (`docs/protocol/membership.md`, "Group
    /// changes").
    pub added: Vec<Leaf>,
    pub authenticated_data: Vec<u8>,
}

/// What a device reads of a group before it joins it, from the GroupInfo it joins by external
/// Commit or the one in its Welcome: the GroupInfo's signer and the members its tree lists. The
/// library has verified the GroupInfo's signature with the signer's leaf in that tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Joining {
    pub signer: Leaf,
    /// The leaves of the group's members, in leaf order.
    pub members: Vec<Leaf>,
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
    /// The library's party outside the group, for the [`DeliveryService`].
    type Observer: Observer;

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
    /// Drops the device's pending Commit: the group stays in its epoch. Fails if there is none.
    fn clear_pending_commit(&mut self, group_id: &[u8]) -> Result<()>;
    /// Drops the group and everything the device keeps for it.
    fn delete_group(&mut self, group_id: &[u8]) -> Result<()>;
    /// Joins a group from a Welcome once `check` passes on its GroupInfo; returns its
    /// `group_id`. Fails, and joins nothing, if the check fails.
    fn join_with(
        &mut self,
        welcome: &[u8],
        check: impl FnOnce(&Joining) -> Result<()>,
    ) -> Result<Vec<u8>>;
    /// Joins a group from a Welcome without a check of its GroupInfo; returns its `group_id`.
    fn join(&mut self, welcome: &[u8]) -> Result<Vec<u8>> {
        self.join_with(welcome, |_| Ok(()))
    }
    /// Joins the group of `group_info`, an encoded `MLSMessage` with a GroupInfo with the
    /// `ratchet_tree` and `external_pub` extensions, by an external Commit [RFC 9420, §12.4.3.2]
    /// once `check` passes on it. Fails, and joins nothing, if the check fails. If the tree has a
    /// leaf with the device's signature key, the Commit also removes it ("Rejoining"); the new group
    /// replaces the state the device had of it. The device is in the Commit's new epoch at once:
    /// if the Delivery Service refuses the Commit, the device drops the group with
    /// [`Device::delete_group`].
    fn join_external(
        &mut self,
        group_info: &[u8],
        check: impl FnOnce(&Joining) -> Result<()>,
    ) -> Result<Commit>;
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

/// A party outside a group that follows its epoch and members from its GroupInfo and
/// PublicMessages only, with the library's support for one: what [`DeliveryService`] knows of a
/// channel.
pub trait Observer: Sized {
    /// The group as an encoded `MLSMessage` with a GroupInfo and the `ratchet_tree` extension
    /// gives it. Fails unless the GroupInfo's signature verifies with its signer's leaf in that
    /// tree and the tree is valid.
    fn observe(group_info: &[u8]) -> Result<Self>;
    /// Processes a Commit, an encoded `MLSMessage` with a PublicMessage, of the current epoch:
    /// the library verifies it, then `check` gets the leaf of its sender, a member, or for an
    /// external Commit the joiner's new leaf. Only if both pass does the observer move to the
    /// Commit's new epoch.
    fn process_commit(
        &mut self,
        commit: &[u8],
        check: impl FnOnce(&Leaf) -> Result<()>,
    ) -> Result<()>;
    /// The group's current epoch.
    fn epoch(&self) -> u64;
    /// The encoded `GroupContext` of the current epoch [RFC 9420, §8.1].
    fn context(&self) -> Result<Vec<u8>>;
    /// The leaves of the group's members, in leaf order.
    fn members(&self) -> Result<Vec<Leaf>>;
}

/// Defines one test per scenario of [`scenarios`] for the [`Device`] type `$device`: a `#[test]`
/// on native targets, a `#[wasm_bindgen_test]` on WASM, where the test crate depends on
/// `wasm-bindgen-test`.
#[macro_export]
macro_rules! scenario_tests {
    ($device:ty) => {
        $crate::each_scenario!(@tests $device);
    };
}

/// Runs every scenario of [`scenarios`] for the [`Device`] type `$device`, in order; panics on the
/// first that fails.
#[macro_export]
macro_rules! run_scenarios {
    ($device:ty) => {
        $crate::each_scenario!(@run $device);
    };
}

/// Every scenario of [`scenarios`], for [`scenario_tests!`] and [`run_scenarios!`].
#[doc(hidden)]
#[macro_export]
macro_rules! each_scenario {
    (@tests $device:ty; $($scenario:ident),+) => {
        $(
            #[cfg_attr(not(target_arch = "wasm32"), test)]
            #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
            fn $scenario() {
                $crate::scenarios::$scenario::<$device>();
            }
        )+
    };
    (@run $device:ty; $($scenario:ident),+) => {
        $($crate::scenarios::$scenario::<$device>();)+
    };
    (@$arm:ident $device:ty) => {
        $crate::each_scenario!(
            @$arm $device;
            creating_a_channel,
            member_adds_a_device,
            members_exchange_messages,
            replayed_message_is_rejected,
            removed_device_reads_nothing_later,
            revocations_are_read_before_merging,
            empty_commit_starts_a_new_epoch,
            history_key_rotates_with_each_epoch,
            device_keeps_key_packages,
            key_packages_that_fail_a_check_are_refused,
            devices_that_are_not_admitted_are_refused,
            last_resort_key_package_adds_to_two_groups,
            device_added_without_admission_is_removed,
            commits_carry_the_new_epochs_group_info,
            two_commits_in_one_epoch_converge,
            three_commits_in_one_epoch_converge,
            commit_for_an_old_epoch_is_refused,
            commit_from_a_non_member_is_refused,
            two_creators_end_in_the_kept_group,
            device_joins_by_itself,
            joiner_that_is_not_admitted_is_removed,
            group_info_that_fails_a_check_is_refused,
            welcome_that_fails_a_check_is_refused,
            device_that_missed_a_commit_rejoins,
            offline_device_catches_up,
            browser_reads_the_members_from_the_group_info,
            group_info_that_fails_the_browser_check_is_rejected
        );
    };
}
