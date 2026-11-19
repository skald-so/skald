//! Skald end-to-end encryption core.

mod account;
mod anchors;
mod cert;
mod cpace;
mod grant;
mod history;
mod hpke;
mod link;
mod mls;
mod revocation;
mod roster;
mod sealed;
mod sign;
mod wrap;

pub use account::{
    AccountError, RecoveryKey, RecoveryKeyError, check_account, open_master_backup,
    seal_master_backup, seal_master_backup_with_ephemeral, sign_account,
};
pub use anchors::{check_anchors, sign_anchors};
pub use cert::{ChainProblem, DeviceCheckError, check_device, issue};
pub use cpace::{CpaceError, CpaceInitiator, CpaceResponder, Isk};
pub use grant::{GrantError, check_grant};
pub use history::{HistoryError, HistoryKey, HistoryMessage, follow_links};
pub use hpke::{HpkeError, HpkeKey, HpkeMessage, open_message, seal_message};
pub use link::{LinkCode, LinkCodeError, LinkKey, LinkOpenError};
pub use mls::{
    ChannelView, CheckedKeyPackage, Decrypted, GroupChange, Joining, KeyPackage, MemberDevice,
    MlsContent, MlsDevice, MlsError, MlsFraming, Processed, check_key_package,
};
pub use revocation::{check_valid_device, counts, revoked_devices, sign_revocation};
pub use roster::{RosterError, accept_next, check_roster, sign_roster};
pub use sealed::{SealedState, StateKey};
pub use sign::{SignatureError, SigningKey, verify};
pub use wrap::{OpenedWrap, open_wrap};
