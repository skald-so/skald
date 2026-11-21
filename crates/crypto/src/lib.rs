//! Skald end-to-end encryption core.

mod account;
mod anchors;
mod cert;
mod cpace;
mod grant;
mod history;
mod hpke;
mod invite;
mod link;
mod mls;
mod presence;
mod queue;
mod revocation;
mod roster;
mod sealed;
mod sign;
mod ssh;
mod trust;
mod wrap;

pub use account::{
    AccountError, RecoveryKey, RecoveryKeyError, check_account, open_master_backup,
    seal_master_backup, seal_master_backup_with_ephemeral, sign_account,
};
pub use anchors::{check_anchors, check_link, sign_anchors, sign_link};
pub use cert::{ChainProblem, DeviceCheckError, check_device, issue, passes_proof_check};
pub use cpace::{CpaceError, CpaceInitiator, CpaceResponder, Isk};
pub use grant::{GrantError, check_grant};
pub use history::{HistoryError, HistoryKey, HistoryMessage, follow_links};
pub use hpke::{HpkeError, HpkeKey, HpkeMessage, open_message, seal_message};
pub use invite::{
    InviteLink, InviteLinkError, InviteSecret, check_acceptance, check_invite, sign_acceptance,
    sign_invite,
};
pub use link::{LinkCode, LinkCodeError, LinkKey, LinkOpenError, link_session};
pub use mls::{
    ChannelView, CheckedKeyPackage, Decrypted, GroupChange, Joining, KeyPackage, MemberDevice,
    MlsContent, MlsDevice, MlsError, MlsFraming, Processed, check_key_package,
};
pub use presence::{PresenceError, PresenceKey};
pub use queue::{
    QueueError, QueueKeyPair, check_queue_key, check_task, seal_task, seal_task_with_ephemeral,
    sign_queue_key, sign_task,
};
pub use revocation::{check_valid_device, counts, marked_stolen, revoked_devices, sign_revocation};
pub use roster::{RosterError, accept_next, check_roster, sign_roster};
pub use sealed::{SealedState, StateKey};
pub use sign::{SignatureError, SigningKey, verify};
pub use ssh::{SshCheck, check_ssh_signature};
pub use trust::trust_level;
pub use wrap::{OpenedWrap, open_wrap};
