//! 30 days after the deletion, no row of the database holds the account's data but what the spaces
//! it shared keep, whether or not every channel has removed its devices.

use crate::erasure::{erased_within, leftovers};
use crate::shared::Shared;

/// In the shared scenario, bob's laptop has removed alice's from `general`. Before her deletion,
/// rows hold her data beyond what `acme` keeps; 30 days after it, within the erasure's time, none
/// does: every table read, no row holds her username, subject, account record, anchors object or a
/// wrap to her recovery key; only rows holding `acme`'s revisions, her devices' certificates, her
/// revocation or her entry hold her devices' IDs or her master key; her ID stays only beside
/// `acme`'s and in one row, the mark of her deletion.
#[test]
fn thirty_days_after_the_deletion_no_table_holds_the_accounts_data_but_what_its_spaces_keep() {
    let shared = Shared::new(3_304_010_001, true);
    let before = leftovers(shared.db(), &shared.alice, &shared.kept);
    assert!(!before.is_empty(), "no row held alice's data to erase");

    shared.deletes_long_ago();

    erased_within(shared.db(), &shared.alice, &shared.kept);
}

/// In the shared scenario, no device removed alice's laptop from `general`, so its group still has
/// her device. 30 days after her deletion, within the erasure's time, no row holds her data beyond
/// what `acme` keeps, her devices' IDs aside; bob's session still gets her entry.
#[test]
fn the_account_is_erased_also_while_a_channel_still_has_its_devices() {
    let shared = Shared::new(3_304_010_011, false);

    shared.deletes_long_ago();

    erased_within(shared.db(), &shared.alice, &shared.kept);
    let entries = shared.entries_for(&shared.f().team.bob);
    assert!(
        entries.contains(&shared.entry.bytes().to_vec()),
        "alice's entry for bob"
    );
}
