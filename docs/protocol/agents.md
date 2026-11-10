# Skald protocol: presence and `ask_user`

Part of the [Skald protocol](README.md), with its conventions and status. **No external audit.**

CDDL rules not defined here, such as `id`, `address`, `delegation`, `signed` and `timestamp`, are in [README.md](README.md), [delivery.md](delivery.md) and [membership.md](membership.md).

## Contents
- [Person](#person)
- [Presence](#presence)
- [`ask_user`](#ask_user)
- [Security considerations](#security-considerations)

## Person
- An address whose agent is `user` is a person address: `acme/alice/user@skald.example` names the member of space `acme` whose handle is `alice`, not an agent. No agent is named `user`.
- Only a `client` or `browser` device of that member's account sends as it: what the person enters in the CLI or a linked browser, such as an [answer](#ask_user). Check: [Envelope](README.md#envelope), check 2.
- Content whose `to` is a person address is for that person: a device of the person's account shows it to its user and hands it to no agent. Devices issue no receipt for it.
- An agent's person is the member whose handle is the owner in its address; a service agent's, the member on whose account its `service` device is certified ([Device certificate](membership.md#device-certificate)).

## Presence
Each device publishes, per channel, the status of its agents there. The value is encrypted under the channel's epoch and signed by the device; the server keeps only the latest value of each device in each channel.

```cddl
presence = {               ; what a device uploads; the server keeps and returns it unchanged
  1 => id,                 ; device: the uploading device
  2 => id,                 ; channel
  3 => uint .ge 1,         ; epoch: whose presence key encrypts `ct`
  4 => bstr .size 12,      ; nonce
  5 => bstr,               ; ct: presence-plaintext, padded and encrypted
  * int => any,            ; fields of later versions
}

presence-plaintext = signed<presence-body>   ; label "skald presence", by `device`

presence-body = {
  1 => id,                 ; device
  2 => id,                 ; channel
  3 => uint .ge 1,         ; epoch
  4 => uint .ge 1,         ; revision: one more than the device's previous value in the channel
  5 => timestamp,          ; expires
  6 => [* agent-status],   ; agents: this device's agents in the channel
  * int => any,
}

agent-status = {
  1 => address,            ; agent
  2 => agent-state,
  3 => timestamp,          ; since: when the agent entered `state`
  ? 4 => tstr .size (1..128),  ; note: what it works on, e.g. "reviewing PR 12"
  ? 5 => tstr,             ; runtime: the agent CLI, a `name` such as "claude-code"
  ? 6 => delegation,       ; only from a custodian device
  * int => any,
}

agent-state = &(available: 1, busy: 2, waiting: 3, offline: 4)
```

- `state`: `available` takes messages; `busy` works on a turn or a task; `waiting` waits for its person, such as on a [question](#ask_user); `offline` has no running session.
- `revision`: a device keeps it as durably as its [counters](delivery.md#gap-detection).
- `expires`: at most 900 seconds after the upload.
- `delegation`: required in each agent of a `custodian` device, absent otherwise: [Custodian device](README.md#custodian-device).

**Key and encryption.**
- The presence key of epoch e is `HKDF-SHA256(salt = "", IKM = the history key of e, info = "skald presence", L = 16)` [[RFC 5869](https://www.rfc-editor.org/rfc/rfc5869)].
- So every device that holds the epoch's [history key](membership.md#history-key) reads the epoch's presence: member devices of the epoch, an `auditor` and a `custodian` among them, and devices that reach the key through the [key chain](membership.md#key-chain) or a [wrap](membership.md#wraps).
- `ct` is AES-128-GCM [[RFC 5116](https://www.rfc-editor.org/rfc/rfc5116)] with the presence key of `epoch`, a random `nonce` and associated data the encoded `["skald presence", device, channel, epoch]`, with `epoch` an unsigned integer.
- The plaintext is the encoded `presence-plaintext` followed by the fewest zero bytes that make its length a multiple of 4096.

**Uploads.**
- A member device of kind `client`, `service` or `custodian` keeps a current value in every channel whose group it is a member of, while its helper runs or a one-shot run of it has an agent. Where it runs no agent, `agents` is empty.
- It uploads new values in all those channels together when an agent's state changes and before `expires` passes, so uploads do not show the server where its agents are. On entering a new epoch, it uploads a new value in that channel. On stopping, it SHOULD upload its agents as `offline`.
- It seals a value only under its current epoch, one it judged [clean](membership.md#clean-epochs); after a browser's revocation, it first commits the empty Commit, as before it sends ([History key](membership.md#history-key)).
- Other devices upload no value: a `browser` and an `auditor` run no agents, and a `service` device that is not a member holds no history key.
- The server takes an upload only from a member device of the channel's current epoch, of kind `client`, `service` or `custodian`, with `device` itself and `epoch` the current one. It refuses others: with `not-a-member` from another device, with `epoch-mismatch` for another epoch, and with `too-large` for a `ct` over 65552 bytes: 16 blocks of 4096 and the tag.
- It keeps the latest upload per channel and device, and deletes it when a Commit it accepts removes the device from the group, or the device is revoked.
- It gives a channel's values only to devices that may send in the channel ([Send](delivery.md#send)).
- Paths and bodies of the presence requests belong to the OpenAPI spec, [api/openapi.json](../../api/openapi.json).

A reader MUST drop a value unless:
1. `ct` decrypts with the presence key of `epoch`, with the associated data from the value's `device`, `channel` and `epoch`, and every byte after the decoded `presence-plaintext` is zero;
2. the body's `device`, `channel` and `epoch` are the value's; `signature` verifies with the certificate of `device`, a valid device ([Device checks](membership.md#device-checks)) of kind `client`, `service` or `custodian`, [admitted](membership.md#admitted-devices) to the channel;
3. `revision` is above the highest the reader accepted for this channel and device, or the value is the one it accepted with that revision;
4. `expires` has not passed;
5. each `agent` passes, as the `from` of content from `device`, check 2 of [Envelope](README.md#envelope), a custodian's with its `delegation` as [Custodian device](README.md#custodian-device) says; none is a [person](#person) or repeated.

- A value that fails only check 4: the reader shows its agents as `offline`, last seen before `expires`.
- It labels each agent with `device` and its [trust level](membership.md#trust-levels). A device that hands presence to an agent, such as through an MCP tool, hands it as data, labelled as inbound content is ([architecture.md, "Security"](../dev/architecture.md#security)).

## `ask_user`
An agent asks its [person](#person) a question and waits for the answer. The person answers on one of their `client` or `browser` devices, which signs the answer.

```cddl
$$body //= (13 => ask)

ask = {
  1 => tstr,               ; question: plain text for the person
  ? 2 => [1*24 tstr .size (1..64)],  ; choices; absent, the answer is free text
  3 => timestamp,          ; expires: when the agent stops waiting
  * int => any,            ; fields of later versions
}

$$body //= (14 => ask-answer)

ask-answer = {             ; exactly one of `choice`, `text` and `declined`
  ? 1 => uint .lt 24,      ; choice: the index of the chosen one in `choices`
  ? 2 => tstr,             ; text: the answer to a question without `choices`
  ? 3 => true,             ; declined: the person will not answer
  * int => any,
}
```

**Question.** Content with `from` the asking agent, `to` the address of its person, `hops` as for a message and the `ask` body.
- `expires`: at most 7 days after sending. A `service` device that keeps no state between runs sets it within its run.
- The asking device sends it under [`hpke`](README.md#scheme-hpke), with `report` true, to each `client` and `browser` device of the person's account with a valid certificate that the member's `excluded` does not list, except itself. Only devices that can receive in the channel get it, so the person must be a member of the channel; the server refuses others with `unknown-recipient` ([Send](delivery.md#send)).
- It triggers the person's `ask-user` notification ([features.md §12](../dev/features.md#12-web-ui)), giving the server only the event class.
- If it is a `client` device of the person, it also offers the question in its own CLI.

A device of the person MUST drop a question unless:
1. it passes the checks of [Envelope](README.md#envelope) under `hpke`;
2. `to` is the person address of the device's account in `space`;
3. `from` is not a person, and its person is the device's account;
4. `expires` has not passed and is at most 7 days ahead.

It offers the question to its user with `from`, the asking device and its [trust level](membership.md#trust-levels), until the question is answered on one of the account's devices or `expires` passes. It hands the question to no agent.

**Answer.** Content with `from` the question's `to`, `to` its `from`, `reply_to` its `msg_id`, `hops` 0 and the `ask-answer` body. `choice` answers a question with `choices`, `text` one without.
- A device sends at most one answer per question, before `expires`, under `hpke` to the asking device. On the asking device itself, the answer travels in no envelope.
- It SHOULD also send it under `hpke` to the account's other devices the question went to, so they stop offering it. Such a device takes it as closing the question if it passes check 1 below, its `reply_to` is a question the device holds and its `from` is that question's `to`; it hands it to no agent.

The asking device MUST drop an answer unless:
1. it passes the checks of [Envelope](README.md#envelope) under `hpke`, and its sending device is a `client` or `browser` device of the person's account;
2. `reply_to` is the `msg_id` of a question this device sent in `channel` and keeps, `from` is that question's `to` and `to` its `from`.

A question this device keeps is in one of these states:

| From | Event | To |
|---|---|---|
| — | Sent | `pending`; `expired` at once if no device of the person has it: the asking device is not a `client` device of the person, and there is no device to send it to or the server refused it for each |
| `pending` | An answer that passes the checks above, or one the person enters on this device, holds exactly one of `choice`, `text` and `declined`, as the question allows | `answered` |
| `pending` | `expires` passes | `expired` |

- `answered`: the device hands the answer to the asking agent as [Device to agent](delivery.md#device-to-agent) says, whatever the agent's inbound mode and allowlist, with the answering device's name and kind, and issues `read` to the answering device if the answer came in an envelope.
- `expired`: the device tells the agent there is no answer.
- Any other answer that passes the checks above: the device issues `refused`, with a reason saying the question was answered, has expired or got an answer it does not allow, and hands the answer to no agent.
- `answered` and `expired` are final. Questions and answers are not copied to the [history](membership.md#history-entry).

## Security considerations
- The server cannot read, forge or move a presence value: it is encrypted, signed by its device and bound by the associated data and the body to its device, channel and epoch. It can withhold, delete or delay values, and give a reader that has not seen a newer value an older one until that one expires, at most 900 seconds after its upload.
- Presence uploads show the server when each member device runs and when a value changes, not where its agents are, since each such device uploads in all its channels together on every change and refresh. A value over 4096 bytes, such as one with many agents, shows its size in steps of 4096 bytes.
- A status and its `note` are the word of the device that signs them; a device lists any agent of its account, as it may send as one. A `note` is data for the agents that read it, not an instruction.
- A device that holds an epoch's history key reads that epoch's presence. A revoked device reads the values sealed under epochs it held, if the server gives them to it; devices upload under the new epoch when they enter it.
- An answer proves the device, not the person. An agent with a shell on a `client` device of the person can answer in its CLI; a browser runs no agent. The asking agent gets the answering device's name and kind.
- A stolen `client` or `browser` device of the person answers the person's questions until its revocation; its answers then fail the device checks. Only an agent of the person asks the person: other members, their agents and service agents certified on other accounts put no question on the person's devices.
- The server can drop or delay a question or an answer, so the question expires, and it chooses which of two answers from different devices of the person arrives first. Gap detection reports a dropped one once a later message from the same device arrives.
- Questions and answers travel under `hpke`, without forward secrecy ([architecture.md, "Cryptography"](../dev/architecture.md#cryptography)). They are not channel messages: no other device of the channel, an `auditor` included, gets them, and the history does not keep them.
- The server sees a device send envelopes to its own account's devices and trigger an `ask-user` notification, so which envelopes likely carry a question, and when an answer follows. A choice and a decline have one size; a text answer shows its length. The notification carries only its class.
- A leaked service token alone triggers the `ask-user` notifications of the service agent's person, without content, within the server's rate limits; it asks nothing. A whole leaked CI secret also asks the person questions as the service agent, labelled with the `service` trust level, and reads their answers. A `service` device never answers. A leaked token of a member `service` device also replaces the device's presence value with one readers drop, so its agent shows as `offline` once the last valid value expires.
- In managed-keys mode, the `custodian` device reads presence and asks questions as the agents delegated to it; it never answers, since only `client` and `browser` devices send as a person.
- Everything the server sees: [architecture.md, "Cryptography"](../dev/architecture.md#cryptography).
