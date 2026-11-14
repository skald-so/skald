# Skald protocol: delivery

Part of the [Skald protocol](README.md), with its conventions and status. **No external audit.**

CDDL rules not defined here, such as `envelope` and `id`, are in [README.md](README.md).

## Contents
- [Hops](#hops)
- [Frames](#frames)
- [Connecting](#connecting)
- [Send](#send)
- [Mailbox](#mailbox)
- [Device to agent](#device-to-agent)
- [Receipts](#receipts)
- [Gap detection](#gap-detection)
- [Order](#order)
- [Attachments](#attachments)
- [Security considerations](#security-considerations)

## Hops
A message reaches an agent in three hops: the sending device → the server → each recipient device → the agent.

- Every hop is at-least-once with deduplication. There is no end-to-end exactly-once.
- Each side keeps a message until the next one has it: the sending device in its outbox, the server in mailboxes, the recipient device in its local store.
- Every message ends in a status the sender sees: [Receipts](#receipts).
- `read` means handed to the agent: it shows neither that the agent acted on the message nor that it did not. Work that must be done goes through the [queue](queues.md#tasks).

## Frames
Client and server exchange envelopes, answers and statuses as frames: CBOR items carried as [Encoding](README.md#encoding) says.

```cddl
frame = auth / send / stored / send-refused / sync / message / status / ack / batch / space-notice

; client → server
auth = [10, challenge: bstr, device: id, signature: bstr .size 64]
send = [1, envelope: bstr .cbor envelope, report: bool]
sync = [4, cursor: uint]
ack = [7, seq]

; server → client
stored = [2, send-key, timestamp]
send-refused = [3, send-key, problem: tstr]
message = [5, seq, envelope: bstr .cbor envelope]
status = [6, seq, delivery-status]
batch = [8, [* (message / status / space-notice)]]
space-notice = [9, space: id]

send-key = [
  msg_id: id,
  ? recipient: id,     ; under `hpke`, the recipient device
]

delivery-status = {
  1 => id,             ; msg_id: the message it reports on
  2 => id,             ; device: the recipient device
  3 => server-status,
  4 => timestamp,      ; when it happened
  * int => any,        ; fields of later versions
}

server-status = &(delivered: 1, expired: 2)
seq = uint .ge 1
timestamp = uint       ; seconds since the Unix epoch
```

- `report`: whether the server issues `delivered` and `expired` for the envelope ([Receipts](#receipts)).
- `space-notice`: the space has a new roster revision or invite acceptance ([membership.md](membership.md#roster)); the client fetches it over HTTPS. It is not a mailbox entry: on connecting, the client fetches what it missed.
- The client and the server each ignore a frame of a type they do not know.
- The server closes a connection on a frame that does not decode.
- On each transport:
  - WebSocket: every frame, both ways, on one connection.
  - HTTPS: `send` and `ack` are request bodies; the answer to a `send` is its `stored` or `send-refused`. A long-poll request states the cursor; its answer is a `batch`, empty if nothing arrived before its timeout.
  - SSE: one `message`, `status` or `space-notice` per event, the event's `id` the `seq` of a `message` or `status`; `Last-Event-ID` states the cursor. The client sends `send` and `ack` over HTTPS.
- HTTPS paths and parameters belong to the OpenAPI spec, [api/openapi.json](../../api/openapi.json). A refusal of the request itself, such as an unsupported protocol version, is a problem details object: [Server answers](README.md#server-answers).

## Connecting
A WebSocket carries other frames only once its first frame, `auth`, proves the key of the connection's device.

- Before opening it, the client gets a challenge for its device session: `POST /session/challenge`.
- `auth` names the device and signs, with the label `skald auth`, `["skald auth", challenge, origin]`. `origin` is the origin [[RFC 6454](https://www.rfc-editor.org/rfc/rfc6454)] of the server URL the client connects to, such as `https://skald.example`, its ASCII serialization as a byte string.
- The server takes the connection, as the device's, only if:
  - `auth` is the first frame and comes within a few seconds of the opening;
  - the challenge is one it issued less than a minute ago to a session that has not ended, and no connection has used it before; a challenge works once, used or refused;
  - the device is a `client` device with a certificate on the server, of the challenge's session's account;
  - `signature` verifies with the device's signing key over `origin` the server's public address.
- Otherwise it closes the connection with code 1008 (policy violation) and the reason, checked in this order:

| Reason | The connection |
|---|---|
| `auth-required` | Sends another frame first, or none in time |
| `challenge-refused` | Names a challenge that is unknown, used, expired or of an ended session |
| `device-refused` | Names a device that is not a `client` device with a certificate of the session's account |
| `signature-refused` | Carries a signature that does not verify over the server's public address |

- A connection closes, with code 1008 and the reason `session-ended`, when the session of its challenge ends.
- A server that fails to check a proof, such as when its database fails, closes the connection with code 1011 (internal error).

## Send
- The sending device keeps each envelope in its outbox and sends it, with backoff between attempts, until the server answers `stored` or `send-refused`. A retry carries the same bytes.
- The idempotency key of an envelope is its sending device, its `msg_id` and, under `hpke`, its recipient device. It has no field of its own: `msg_id` is unique per sending device, and the server knows the sending device from the authenticated connection.
- The server stores at most one envelope per idempotency key and keeps the key for at least its mailbox retention. It answers a retry as it answered the first send, whatever the retry's bytes.
- `stored` is sent only after the envelope and its mailbox entries are committed. In the cloud, after the replica has the write: [infrastructure.md, "Cloud"](../dev/infrastructure.md#cloud).
- `send-refused` names one of these problems. A client treats a problem it does not know as a refusal too.

| Problem | The envelope |
|---|---|
| `version-mismatch` | Has a `v` other than the version of its connection or request |
| `not-a-member` | Comes from a device that may not send in `channel`, or holds a Commit from a device that may not commit it ([Commit ordering](membership.md#commit-ordering)) |
| `epoch-mismatch` | Holds a Commit for an epoch other than the channel's current one ([Commit ordering](membership.md#commit-ordering)) |
| `unknown-recipient` | Names in `recipients` a device that cannot receive in `channel` |
| `too-large` | Has a payload over the server's limit (cloud: [architecture.md, "Cloud limits"](../dev/architecture.md#cloud-limits)) |
| `quota-exceeded` | Is over a limit of its space (cloud: [architecture.md, "Cloud limits"](../dev/architecture.md#cloud-limits)) |

- A refused envelope is not stored. The client does not retry it and tells its user.

## Mailbox
- The server keeps a mailbox for each device. An entry is a `message`, an envelope for the device, or a `status`, a server status for the device as a sender.
- Storing an envelope adds a `message` entry to the mailbox of each of its recipient devices: `recipients` in [Envelope](README.md#envelope).
- `seq` numbers a mailbox's entries: 1 for the first, one more for each next one. A `seq` is never reused.
- **Cursor:** the highest `seq` the device has written to its local store. The device updates it in the same transaction as the entries it covers.
- **Ack:** the device acks by stating its cursor: on connecting, and in `ack` after writing entries. The server deletes every entry up to it.
- **Stating the cursor on connecting:** on WebSocket, in `sync`; on HTTPS, in each long-poll request; on SSE, in `Last-Event-ID`.
  - On WebSocket, the server sends no `message` or `status` on a connection before its first `sync`.
  - Every `sync` acks like `ack`. Only the first `sync` of a connection starts catch-up; a later one only acks.
- **Catch-up:** after the cursor a device states on connecting, the server sends the remaining entries in `seq` order, then pushes new entries as they are added.
- A device MAY hold several connections. The server sends each of them the entries in `seq` order.
- **Expiry:** an entry expires when it is not acked in time:
  - a `message` at its storing time plus the envelope's `ttl`, capped by the server's retention (cloud: [architecture.md, "Cloud limits"](../dev/architecture.md#cloud-limits)), unless it holds a handshake message;
  - a `status` at its adding time plus the retention;
  - a `message` whose envelope's `v` is neither the connection's version nor adjacent to it, when the server reaches it while sending;
  - every entry of a revoked device, at revocation.
- A handshake message is an `mls` payload that is a Welcome, or a PublicMessage or PrivateMessage whose `content_type` is `proposal` or `commit` [[RFC 9420](https://www.rfc-editor.org/rfc/rfc9420), §6]. Its entry does not expire by time; the other rules apply to it.
- The server deletes an expired entry and does not send it.
- The server deletes an envelope once no mailbox entry refers to it.

A recipient device drops, without a receipt:
- an entry with a `seq` up to its cursor: a redelivery after a lost ack;
- an `mls` message whose key it has already used: a replay;
- content whose `counter` [gap detection](#gap-detection) does not accept, or content without `counter` whose `msg_id` it already has from the same sending device: a duplicate or a replay.

## Device to agent
- A recipient device hands a message over to the agents it is for: the agent in `to` if it runs on this device; without `to`, each agent of this device in the channel. Content to a [person](agents.md#person) goes to no agent.
- The agent's inbound mode decides first, except for an [`ask_user` answer](agents.md#ask_user):
  - `accept`: the device hands the message over;
  - `hold`: the device issues `held` and keeps the message until a human decides; then it hands the message over or issues `refused`;
  - `refuse`: the device issues `refused`.
- The [trust level](membership.md#trust-levels) of the sending device sets how the text reaches the agent's session:
  - `own`: the device MAY put the text into the session, labelled as a request from the person's own agent;
  - `peer` and `service`: the device puts only a signal into the session, a fixed text, without the sender or the text, that tells the agent messages wait; the agent takes the text with the `inbox` tool, labelled as data from its sender.
- A way into the session whose text other local accounts can read carries only a signal, whatever the level.
- How a handover wakes the agent's session is the device user's setting per channel and trust level: `now` wakes it at once, `batch` once per window with every message that arrived in it, `pull` never, so messages wait for `inbox`. The default is `now`.
  - A device MUST NOT use a mode more waking than the space [policy](membership.md#policy)'s `wakes` for the level, in the order `now`, `batch`, `pull`; under `batch`, it uses `batch` with a window at least as long as the policy's, or `pull`.
  - An [`ask_user` answer](agents.md#ask_user) wakes the session at once in every mode; a task's [signal](queues.md#running-a-task) follows the mode as a message from its sender does.
- `inbox` is also the fallback, the way for agents that nothing pushes to, and for messages sent again.
- The device marks a message handed over after the handover. A crash in between repeats the handover on restart; it never loses it.
- Each handover carries `msg_id`, so an agent or adapter that keeps earlier ones drops a repeat.
- After marking a message with `to` handed over, the device issues `read`.
- Only `a2a`, [`task-result`](queues.md#results) and [`ask-answer`](agents.md#ask_user) bodies are handed over as messages; receipts are not.
- How each agent and surface takes messages: [Delivery to agent sessions](../agent-sessions.md).

## Receipts
| Status | Meaning | Issued by | Reaches the sender as |
|---|---|---|---|
| `stored` | The server has committed the envelope | Server | The answer to the send |
| `delivered` | A recipient device has written the envelope | Server, on the device's [ack](#mailbox) | A `status` entry |
| `expired` | A recipient device's entry expired before its ack | Server | A `status` entry |
| `held` | The `to` agent's inbound mode holds the message for a human | The `to` agent's device | A receipt message |
| `refused` | The `to` agent's inbound mode or a human refused the message | The `to` agent's device | A receipt message |
| `read` | The message was handed to the `to` agent | The `to` agent's device | A receipt message |
| `replied` | The `to` agent answered | The reply itself | The reply: content from the `to` agent whose `reply_to` is the message's `msg_id` |

- **Server statuses** (`stored`, `delivered`, `expired`) are per recipient device and unsigned: the sending device takes them on the server's word, over its authenticated connection.
  - The server adds `delivered` and `expired` as a `status` entry to the mailbox of the envelope's sending device, only for an envelope sent with `report` true.
  - A device sends receipts and handshake messages with `report` false.
- **Receipt messages** (`held`, `refused`, `read`) are application messages with the `receipt` body. They are signed and encrypted like any content and pass the same checks: [Envelope](README.md#envelope).
  - `from` is the `to` agent of the messages it reports on; `to` is their `from`; `hops` is 0.
  - It goes in the scheme its messages came in: under `mls` to the channel, under `hpke` to their sending device.
  - A device issues them only for messages whose `to` is an agent, not a [person](agents.md#person).
- The sending device takes a receipt or a reply as a status only from the message's `to` agent.
- Per recipient device a message goes `stored` → `delivered` or `expired`. For the `to` agent it goes `read`, `refused`, or `held` → `read` or `refused`; `replied` follows `read`.
- Every stored envelope sent with `report` true ends with each recipient device at `delivered` or `expired`. A refused send ends in its problem.

```cddl
$$body //= (9 => receipt)

receipt = {
  1 => receipt-status,
  2 => [+ id],         ; messages: the msg_id of each message it reports on
  ? 3 => tstr,         ; reason: why a message was refused, for its sender
  * int => any,        ; fields of later versions
}

receipt-status = &(held: 1, refused: 2, read: 3)
```

- The messages of one receipt come from one sending device in the receipt's channel.

## Gap detection
The content's `counter` ([Content](README.md#content)) lets a recipient device notice a message it did not get.

- A recipient keeps, per channel, sending device and scheme, the highest `counter` it has accepted and the counters it is missing. Under `hpke`, these are the counters the sending device keeps for this recipient.
- The baseline: under `hpke`, the highest starts at 0; under `mls`, the first message from the sending device since the recipient last joined the channel sets it.
- For content with `counter` c that passed the checks in [Envelope](README.md#envelope):
  1. c above the highest: accepted; the counters between the highest and c become missing; c becomes the highest.
  2. c missing: accepted; c is no longer missing.
  3. Otherwise: a duplicate or a replay; dropped.
- The recipient MUST tell its user about missing counters, with the channel and the sending device: those messages were refused, expired or dropped by the server. It MUST mark the next message from that sending device it hands to an agent as following missing ones.
- A sending device has a channel's messages stored in `counter` order, per recipient device under `hpke`: over one connection it sends them in that order; over HTTPS it sends each after the previous one is answered. With [Order](#order), a missing counter is then a message the recipient did not get, not one still on its way.
- A device keeps its counters as durably as its keys. A device that loses them registers as a new device.
- Content without `counter`, from a stateless `service` device, has no gap detection.

## Order
- Within a channel the order is total and set by the server: the order in which it stores the channel's envelopes. It adds them to every mailbox in that order.
- The server stores the sends of one connection in the order it receives them.
- There is no order across channels.
- A recipient device processes a channel's envelopes in its mailbox order.

## Attachments
A file, such as a diff or a log, travels as an encrypted blob uploaded apart from the message and referenced from its content.

```cddl
attachment = {
  1 => id,             ; blob: the blob's ID, assigned by the server
  2 => bstr .size 16,  ; key: the blob's AES-128-GCM key
  3 => bstr .size 32,  ; digest: SHA-256 of the blob
  4 => uint,           ; size: the file's length in bytes
  * int => any,        ; fields of later versions
}
```

- The sending device encrypts each file with a fresh random key: AES-128-GCM [[RFC 5116](https://www.rfc-editor.org/rfc/rfc5116)], a nonce of 12 zero bytes, since each key encrypts one file, and no associated data. The blob is the ciphertext with its tag.
- It uploads each blob over HTTPS for the channel and gets its `blob` ID and expiry; then it sends the message.
- The content lists the attachments under `attachments`. Each is also one file part of the A2A message, with its own `filename` and `mediaType` and the `url` `urn:uuid:<blob>`, the blob ID in text form [[RFC 9562](https://www.rfc-editor.org/rfc/rfc9562)].
- A recipient device downloads a blob by its ID only from the blob endpoint of the server it receives the channel's envelopes from. It MUST NOT fetch any URL the sender wrote. An A2A file part whose `url` is not the `urn:uuid:` of an attachment of the content fails the check.
- After the download it checks `digest`, then decrypts the blob with `key` and checks `size`. It hands the agent the file in place of the part's `url`. A part or blob that fails a check is not handed over; the device tells its user.
- `digest` keeps a channel member who knows `key` from having the server swap the blob for another one under the same key.
- **Size limit:** the server refuses to store a blob over its limit (cloud: [architecture.md, "Cloud limits"](../dev/architecture.md#cloud-limits)).
- **Expiry:** the server keeps a blob as long as it keeps its channel's messages: its mailbox retention, or the channel's history retention if longer. Then it deletes the blob; the device tells its user the attachment expired.
- Only devices that receive the channel's envelopes can download its blobs.

## Security considerations
- A session alone reads and acks no mailbox: a connection takes the device's key. The signature covers the origin the client connects to, so a malicious server cannot relay another server's challenge: that server refuses the proof.
- `stored`, `delivered` and `expired` are the server's word. Only receipt messages and replies are end-to-end: encrypted, and signed by the recipient device.
- Gap detection catches a message the server drops once a later message from the same sending device in the same channel under the same scheme reaches the recipient. It does not catch the drop of:
  - a sender's latest messages in a channel under a scheme, until it sends another there under it; for a message with `to`, the sender still sees no `read`;
  - under `mls`, the messages a sending device sent after the recipient joined and before the first one the recipient got;
  - content without `counter`.
- Attachments carry a blob ID, not a URL. A sender, such as a malicious channel member or an agent steered by prompt injection, cannot make a recipient device fetch from a host of its choice: that would leak the device's credentials, IP address and timing, or reach internal addresses (SSRF). `digest` is checked only after the download, so it does not prevent this.
- Everything the server sees, delivery included: [threat model, "The server sees"](../threat-model.md#the-server-sees).
