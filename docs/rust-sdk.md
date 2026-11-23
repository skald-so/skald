# Rust message SDK

Add the `skald-client` crate from a Skald checkout to your Rust application.

```toml
[dependencies]
skald-client = { path = "../skald/crates/client" }
```

Enroll a device with `skald sign-in` and finish linking it. Pass the device's state directory, its key storage and a timeout to `DeviceContext::one_shot`. Use `KeyStorage::Auto` for the system store with the usual file fallback, or `KeyStorage::File` for a device enrolled with `--key-storage file`. Use the channel ID from `skald channels <space>`. [Device enrollment](../README.md#2-connect-a-machine)

```rust
use std::{io, path::PathBuf, time::Duration};
use skald_client::{ChannelId, Client, DeviceContext, KeyStorage, MessageError, ReceivedMessage};

fn exchange(
    dir: PathBuf,
    channel: ChannelId,
    save: impl FnMut(ReceivedMessage) -> io::Result<()>,
) -> Result<(), MessageError> {
    let context = DeviceContext::one_shot(dir, KeyStorage::Auto, Duration::from_secs(30));
    let mut client = Client::new(context);
    let id = client.send(channel, "When does the morning shipment leave?")?;
    println!("Sent {id}");
    client.receive(save)
}
```

The calls block and use the CLI's state lock, device proof, transport and encryption. A one-shot call runs in the application process and waits for the lock if another process or the helper owns it. It uses the timeout for the lock wait, native requests and send retries. The [fixture application](../crates/client/examples/rust_message_fixture/main.rs) shows a complete command-line program.

## Send and retry

`send` addresses the channel as the enrolled device's person, like `skald send --all`. A successful call returns a `MessageId` after the server confirms storage. Recipient devices acknowledge delivery separately. The protocol identifies each envelope by sending device, message ID and, for a device-specific copy, recipient device. Retries after a lost response use the same envelope and ID. [Delivery protocol](protocol/delivery.md#send)

`MessageError::SendRefused` carries the message ID, the server's problem code and an optional recipient device. This describes one envelope. Other envelopes of the same message may already have been stored or may still be pending. `MessageError::SendPending` identifies a message awaiting an answer in the local outbox. A later send or receive retries pending envelopes. Another call to `send` creates a new message.

## Receive and hand over

`receive` catches up the mailbox once and calls the application for each pending text message in the person's inbox. Each record carries the original message ID, sending device, sender address, space, channel, text, trust level and optional reply ID. Delivery statuses, task results, permission prompts and old text-only records stay available to the CLI.

The server's delivery receipt means the device has persisted the envelope. Local handover is recorded only after the callback returns `Ok(())`. The callback runs while the lock is held. A callback error returns `MessageError::Handover` and leaves the message for the next receive. A crash before the handover is recorded may repeat a message. Deduplicate with the sending device and message ID. The application and `skald recv` both consume the same person's inbox.

`MessageError::StateLock` reports a state lock failure, `MessageError::AccountDeleted` reports a deleted account, and `MessageError::Operation` gives details of another native client failure.
