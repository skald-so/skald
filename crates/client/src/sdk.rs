use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use skald_proto::VersionRange;

use crate::{
    Address, ChannelId, DeviceId, KeyStorage, MessageId, SpaceId, TrustLevel, connection, deletion,
    forms, link_code, lock, mailbox, output, recv, send,
};

/// Local state and settings for an already enrolled Skald device.
pub struct DeviceContext {
    state_dir: PathBuf,
    key_storage: KeyStorage,
    timeout: Duration,
}

impl DeviceContext {
    /// Use `state_dir` and the key storage chosen during enrollment.
    ///
    /// Each operation runs in the calling process. It waits for the native state lock for
    /// up to `timeout`, which also sets the native request and send retry timeout.
    pub fn one_shot(state_dir: PathBuf, key_storage: KeyStorage, timeout: Duration) -> Self {
        Self {
            state_dir,
            key_storage,
            timeout,
        }
    }
}

/// Blocking access to an enrolled device's messaging operations.
pub struct Client {
    context: DeviceContext,
}

impl Client {
    /// Creates a client. The device state is opened when an operation runs.
    pub fn new(context: DeviceContext) -> Self {
        Self { context }
    }

    /// Sends `text` as the person to `channel`, with the CLI's `send --all` addressing.
    ///
    /// Returns the message ID after the server confirms storage. Native retries use the same
    /// envelope and ID. Calling this method again creates a new message.
    pub fn send(&mut self, channel: ChannelId, text: &str) -> Result<MessageId, MessageError> {
        self.with_device(|dir, storage, settings| {
            send::send_message(dir, storage, settings, channel, text)
        })
    }

    /// Catches up the mailbox and hands over pending messages from the person's inbox.
    ///
    /// The callback runs while the device state is locked. Return `Ok(())` after taking
    /// responsibility for the message. A callback error leaves the message pending and stops
    /// handover. A crash before the local handover is recorded can repeat that message on
    /// restart. Use its sending device and message ID to deduplicate.
    pub fn receive(
        &mut self,
        mut hand_over: impl FnMut(ReceivedMessage) -> io::Result<()>,
    ) -> Result<(), MessageError> {
        self.with_device(|dir, storage, settings| {
            recv::receive_messages(dir, storage, settings, &mut hand_over)
        })
    }

    fn with_device<T>(
        &self,
        run: impl FnOnce(&Path, KeyStorage, (VersionRange, Duration)) -> Result<T, MessageError>,
    ) -> Result<T, MessageError> {
        let dir = std::path::absolute(&self.context.state_dir).map_err(|err| {
            MessageError::StateLock(format!(
                "cannot use the state directory {}: {err}",
                self.context.state_dir.display()
            ))
        })?;
        let DeviceContext {
            key_storage,
            timeout,
            ..
        } = self.context;
        deleted(&dir)?;
        let lock = lock::take(&dir, false, timeout).map_err(MessageError::StateLock)?;
        deleted(&dir)?;
        let _lock =
            lock.ok_or_else(|| MessageError::Operation(crate::devices::NOT_SIGNED_IN.to_owned()))?;
        let speaks = VersionRange::default();
        let _refusals = connection::track_refusals();
        let _close = CloseMailbox;
        output::into(
            |_, _| Ok(()),
            || {
                link_code::finish(&dir, key_storage, speaks, timeout, true);
                let ran = run(&dir, key_storage, (speaks, timeout));
                if connection::session_refused()
                    && let Some(reason) = deletion::learned(&dir, (key_storage, speaks, timeout))
                    && !matches!(ran, Err(MessageError::Handover(_)))
                {
                    return Err(MessageError::AccountDeleted(reason));
                }
                ran
            },
        )
    }
}

fn deleted(dir: &Path) -> Result<(), MessageError> {
    if deletion::reported(dir) {
        return Err(MessageError::AccountDeleted(deletion::said(dir)));
    }
    Ok(())
}

struct CloseMailbox;

impl Drop for CloseMailbox {
    fn drop(&mut self) {
        mailbox::close_kept();
    }
}

/// An authenticated text message handed over from local state.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReceivedMessage {
    /// The protocol message ID.
    pub id: MessageId,
    /// The sending device's ID.
    pub sender: DeviceId,
    /// The sending person or agent's address.
    pub from: Address,
    /// The space containing the channel.
    pub space: SpaceId,
    /// The channel that carried the message.
    pub channel: ChannelId,
    /// The message ID this message answers, if it is a reply.
    pub reply_to: Option<MessageId>,
    /// The message text.
    pub text: String,
    /// The trust level the device assigned the sender.
    pub trust: TrustLevel,
    /// Whether messages from the sender in this channel were missing before this one.
    pub after_missing: bool,
}

impl ReceivedMessage {
    pub(crate) fn from_record(opened: &[u8]) -> Option<Self> {
        let record: forms::Message = serde_json::from_slice(opened).ok()?;
        if record.result.is_some() {
            return None;
        }
        let reply_to = record.reply_to.map(|id| id.parse()).transpose().ok()?;
        Some(Self {
            id: record.msg_id.parse().ok()?,
            sender: record.device.parse().ok()?,
            from: record.from.parse().ok()?,
            space: record.space.parse().ok()?,
            channel: record.channel.parse().ok()?,
            reply_to,
            text: record.text,
            trust: record.level,
            after_missing: record.after_missing,
        })
    }
}

/// An error from a messaging operation.
#[derive(Debug)]
#[non_exhaustive]
pub enum MessageError {
    /// The native state lock could not be acquired.
    StateLock(String),
    /// The server refused an envelope for this message.
    SendRefused {
        /// The message ID shared by its envelopes.
        id: MessageId,
        /// The recipient of a device-specific envelope, or `None` for the channel envelope.
        recipient: Option<DeviceId>,
        /// The server's protocol problem code. Unrecognized codes are preserved.
        problem: String,
    },
    /// At least one of the message's envelopes remains in the outbox awaiting an answer.
    SendPending {
        /// The message ID retained on retry.
        id: MessageId,
        /// The last attempt's failure, if known.
        reason: Option<String>,
    },
    /// The application callback returned an error.
    Handover(io::Error),
    /// The server confirmed the account associated with this device was deleted.
    AccountDeleted(String),
    /// Another error opening the device, processing a message or reaching the server.
    Operation(String),
}

impl From<String> for MessageError {
    fn from(reason: String) -> Self {
        Self::Operation(reason)
    }
}

impl fmt::Display for MessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StateLock(reason) | Self::AccountDeleted(reason) | Self::Operation(reason) => {
                f.write_str(reason)
            }
            Self::SendRefused {
                id,
                recipient,
                problem,
            } => {
                write!(f, "the server refused the message {id}")?;
                if let Some(device) = recipient {
                    write!(f, " to {device}")?;
                }
                write!(f, ": {}", connection::printable(problem))
            }
            Self::SendPending { id, reason } => {
                write!(f, "the message {id} waits in the outbox")?;
                if let Some(reason) = reason {
                    write!(f, ": {reason}")?;
                }
                Ok(())
            }
            Self::Handover(err) => write!(f, "the application did not take the message: {err}"),
        }
    }
}

impl std::error::Error for MessageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Handover(err) => Some(err),
            _ => None,
        }
    }
}
