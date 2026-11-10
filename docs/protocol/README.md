# Skald protocol

**No external audit.** No independent party has reviewed this protocol. Report flaws privately: [SECURITY.md](../../SECURITY.md).

Status: draft. Current protocol version: 1.

Documents:
- this file: identifiers, addresses, the envelope, encoding and versions;
- [delivery.md](delivery.md): mailbox, delivery, receipts and attachments.

## Contents
- [Conventions](#conventions)
- [Identifiers](#identifiers)
- [Agent address](#agent-address)
- [Envelope](#envelope)
- [Content](#content)
- [Encoding](#encoding)
- [Protocol version](#protocol-version)
- [A2A extension](#a2a-extension)
- [Custodian, auditor and federation](#custodian-auditor-and-federation)
- [Related work](#related-work)
- [Security considerations](#security-considerations)

## Conventions
- The key words MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are to be interpreted as described in BCP 14 [[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119)] [[RFC 8174](https://www.rfc-editor.org/rfc/rfc8174)] when, and only when, they appear in all capitals.
- Binary structures are in CDDL [[RFC 8610](https://www.rfc-editor.org/rfc/rfc8610)]; text forms are in ABNF [[RFC 5234](https://www.rfc-editor.org/rfc/rfc5234)].
- Account, device, agent, space and channel: [architecture.md, "Data model"](../dev/architecture.md#data-model).
- A signature is Ed25519 [[RFC 8032](https://www.rfc-editor.org/rfc/rfc8032)] with the signing key of the signer's device certificate. It covers the encoded CBOR array of a label and byte strings, such as `["skald hpke", header, content]`; the label keeps a signature made for one use from passing for another.

## Identifiers
- An ID is a UUID [[RFC 9562](https://www.rfc-editor.org/rfc/rfc9562)]: 16 bytes on the wire, the lowercase hyphenated form in text, e.g. `01a1c4c4-94cb-7a3e-9c41-5d2e8f6b7a10`. Any other text form is invalid.
- The issuer generates a UUIDv7 with random bits from a cryptographically secure generator, so IDs of different servers do not collide in practice.
- Receivers treat IDs as opaque. An ID is not a secret and grants no access.

| Entity | Identified by | Issued by |
|---|---|---|
| Account | ID | Server, at sign-up |
| Device | ID | Server, at registration |
| Space | ID; its name appears in addresses | Server, at creation |
| Channel | ID, which is also the `group_id` of its MLS group | Server, at creation |
| Agent | [Agent address](#agent-address) | Its owner |
| Message | ID (`msg_id`), which is also its A2A `messageId` | Sending device |

- A service agent holds its own key, so it is also a device, with a device ID.
- Every device has a kind, fixed in its certificate: `client` (a `skald` installation), `browser` (a linked browser), `service` (a service agent), `custodian` or `auditor` ([below](#custodian-auditor-and-federation)).

## Agent address
An address names an agent on any server: `acme/alice/claude-api@skald.example`.

```abnf
address = space "/" owner "/" agent "@" server
space   = name                 ; unique on its server
owner   = name                 ; unique in the space: a member's handle, or a group of service agents such as "ci"
agent   = name                 ; unique within its owner
server  = name *( "." name )   ; the DNS name of the server hosting the space
name    = alnum [ *61( alnum / "-" ) alnum ]
alnum   = %x61-7A / %x30-39    ; a-z, 0-9
```

- `server` is at most 253 characters, in A-label form for internationalized names; its last label is not all digits.
- A form the grammar does not produce is invalid: uppercase letters, other characters, empty or extra segments, a port, a trailing dot.
- A client MAY lowercase user input and complete a short form such as `alice/claude-api` from the current space. The wire always carries the full address.
- Envelopes carry agent names only inside the ciphertext.

## Envelope
A message travels as an envelope: a plaintext header the server routes by and a payload only its recipient devices decrypt.

```cddl
envelope = [
  header,
  payload: bstr,       ; encrypted as `scheme` says
]

header = {
  1 => version,        ; v
  2 => id,             ; space
  3 => id,             ; channel
  4 => id,             ; msg_id
  5 => scheme,
  ? 6 => [+ id],       ; recipients: device IDs
  ? 7 => uint,         ; ttl: seconds
  * int => any,        ; fields of later versions
}

version = uint .ge 1
id = bstr .size 16     ; a UUID
scheme = &(mls: 1, hpke: 2)
```

- `v`: the protocol version of the sending device.
- `space`: the space the sending device sends from; in a shared channel, the sender's side.
- `msg_id`: unique per sending device; a retried send keeps it, and the per-device envelopes of one `hpke` message share it.
- `scheme`: how the payload is encrypted: [`mls`](#scheme-mls) or [`hpke`](#scheme-hpke).
- `recipients`: absent, the envelope goes to every member device of the channel except the sending one. Under `mls`, application messages always go to the whole channel, and `recipients` reaches devices outside the group, such as a joining device receiving an MLS Welcome. Under `hpke`, it holds exactly one device.
- `ttl`: how long an undelivered message is kept; absent or above the server's retention, the server's retention applies.
- The message size is the payload length; there is no size field.
- The server stores and forwards an envelope as the bytes it received. What the server adds, such as the mailbox `seq`, travels beside the envelope: [Frames](delivery.md#frames).

A recipient routes by the header but trusts only what the payload authenticates. It MUST reject an application message unless:
1. the payload authenticates the sending device as its scheme says: [`mls`](#scheme-mls), [`hpke`](#scheme-hpke);
2. the content's `from` is an agent of the sending device's account; a `service` device sends only as itself, a `custodian` device only as an agent delegated to it ([Custodian device](#custodian-device));
3. the sender may send in `channel`:
   - a `service` device: its content carries a [grant](#service-grant) for `channel`;
   - any other device: the account that owns `from` is a member of `channel`, that is, one of its devices is in the channel's MLS group; for a `custodian` device, that is the delegating member's account;
4. the A2A `messageId` is `msg_id` in text form.

A recipient outside the channel's MLS group, such as a linked browser, takes the group's members from the latest GroupInfo with the ratchet tree [[RFC 9420](https://www.rfc-editor.org/rfc/rfc9420), §12.4.3], signed by a member device.

### Scheme `mls`
- The payload is an MLSMessage [[RFC 9420](https://www.rfc-editor.org/rfc/rfc9420), §6]. An application message is a PrivateMessage whose application data is the [content](#content); handshake messages travel in envelopes too.
- It authenticates the sending device when it decrypts in the MLS group whose `group_id` is `channel`; the sending device is the member that sent it.

### Scheme `hpke`
For messages from or to a device outside the channel's MLS group, such as a linked `browser` or an ephemeral `service` agent. The payload is sealed to one device and signed by the sending device.

```cddl
hpke-payload = [
  enc: bstr .size 32,  ; the HPKE encapsulated key
  ct: bstr,            ; hpke-plaintext, sealed
]

hpke-plaintext = [
  sender: id,          ; the sending device
  content: bstr .cbor content,
  signature: bstr .size 64,
]
```

- A message for several devices travels as one envelope per device.
- Sealing: HPKE [[RFC 9180](https://www.rfc-editor.org/rfc/rfc9180)] in base mode, single-shot, with DHKEM(X25519, HKDF-SHA256), HKDF-SHA256 and AES-128-GCM, the primitives of the MLS ciphersuite. The key is the HPKE key in the recipient's device certificate; `info` is the ASCII string `skald hpke`, `aad` the bytes of the encoded header.
- `signature`: by `sender` over `["skald hpke", header, content]`, with the encoded header and `content` as byte strings.
- An `hpke` payload always holds content; handshake messages use `mls`.

It authenticates `sender` as the sending device when:
1. the recipient is the device in `recipients`, and the payload opens with its HPKE key;
2. `signature` verifies with the certificate of `sender`, an unrevoked device;
3. `channel` is a channel of `space`, and `from` names an agent of `space`.

## Content
The content is what an application message carries inside its payload.

```cddl
content = {
  1 => address,        ; from: the sending agent
  ? 2 => address,      ; to: the recipient agent
  ? 3 => uint .ge 1,   ; counter: per-sender counter, for gap detection
  4 => uint,           ; hops: hop counter, for loop protection
  ? 5 => id,           ; reply_to: msg_id of the message this one answers
  $$body,
  ? 7 => delegation,   ; only from a custodian device
  ? 8 => grant,        ; only from a service device
  ? 10 => [+ attachment], ; only with the a2a body
  * int => any,        ; fields of later versions
}

address = tstr         ; an agent address
$$body //= (6 => tstr) ; a2a: one A2A Message in JSON
```

- `to`: absent, the message is for every agent in the channel.
- `counter`: 1 for the first application message a device sends in a channel, one more for each next one; used by [gap detection](delivery.md#gap-detection). Under `hpke`, the sending device keeps a separate counter for each recipient device. Only a `service` device that keeps no state between runs, such as an ephemeral CI agent, omits it: a recipient MUST reject content without `counter` unless it came under `hpke` from a `service` device.
- `hops`: 0 for a message a human asked for; a message an agent sends in reaction to another carries that message's `hops` plus one.
- `delegation`: required in content from a `custodian` device, absent otherwise: [Custodian device](#custodian-device).
- `grant`: required in content from a `service` device, absent otherwise: [Service grant](#service-grant).
- `attachments`: the encrypted files the A2A message refers to: [Attachments](delivery.md#attachments).
- A content carries exactly one body. Other bodies, such as [receipts](delivery.md#receipts), get their own keys.
- The thread, text and attachment parts are in the A2A message: [A2A extension](#a2a-extension).

### Service grant
A space admin lets a `service` device send in channels of the space by signing a grant on one of their `client` or `browser` devices. The service device puts it in every content it sends.

```cddl
grant = [
  body: bstr .cbor grant-body,
  signature: bstr .size 64,  ; by `signer`, over ["skald grant", body]
]

grant-body = {
  1 => id,             ; service: the service device
  2 => [+ id],         ; channels: the channels it may send in
  3 => id,             ; signer: the admin's device
  4 => uint,           ; expires: seconds since the Unix epoch
  * int => any,        ; fields of later versions
}
```

A recipient MUST reject content from a `service` device unless:
1. the content's `grant` names the sending device as `service` and lists `channel` in `channels`;
2. `signer` is an unrevoked `client` or `browser` device of an admin of `space`, and `signature` verifies with its certificate;
3. `expires` has not passed.

## Encoding
- API: JSON over HTTPS, described by the OpenAPI spec, [api/openapi.json](../../api/openapi.json).
- CBOR data items, such as envelopes, travel as the same bytes on every transport:
  - WebSocket [[RFC 6455](https://www.rfc-editor.org/rfc/rfc6455)]: one item per binary frame;
  - HTTPS, for sending without a WebSocket and for long-poll answers: one item per request or response body, of type `application/cbor`;
  - SSE [[HTML, "Server-sent events"](https://html.spec.whatwg.org/multipage/server-sent-events.html)]: one item per event, its `data` the item in base64url without padding [[RFC 4648](https://www.rfc-editor.org/rfc/rfc4648), §5], because SSE carries only text.
- Which transport a client uses: [architecture.md, "Network"](../dev/architecture.md#network).
- CBOR [[RFC 8949](https://www.rfc-editor.org/rfc/rfc8949)] uses the core deterministic encoding (§4.2.1): shortest forms, definite lengths, map keys in bytewise order of their encodings. Text strings are UTF-8. Tags and floating-point values are not used.
- A decoder MUST reject an item that breaks these rules or repeats a map key, and MUST ignore map keys it does not know.

## Protocol version
- An integer from 1, separate from the release version of `skald` and `skald-server`.
- It changes only for a change a peer on the previous version cannot handle. Fields peers may ignore do not change it.
- Adjacent versions interoperate: each reads every envelope the other sends. Replacing an envelope field takes two versions: the first adds the new field, the next drops the old one.
- Each version pins one A2A version ([A2A extension](#a2a-extension)). A new version moves only to an A2A version whose messages the previous one reads.

### Stating the version
- HTTPS: every request a Skald client (`skald`, the web app, an SDK) makes to the API carries `Skald-Protocol-Version: <n>`. Requests that do not depend on the protocol carry none, and the server does not check it on them:
  - `/health` and `/openapi.json`;
  - what a browser opens or is redirected to: web pages and their static files, sign-in, OAuth and OIDC callbacks, the device flow verification page;
  - downloads: install scripts and binaries.
- WebSocket: the client offers the subprotocol `skald.<n>` in its opening request; the server selects it.
- An envelope's `v` is the version of the connection or request it is sent on. The server MUST refuse a sent envelope with another `v`.
- A stored envelope keeps its `v`. The server delivers it only on a connection or request whose version is `v` or adjacent to it; on another, its mailbox entry expires: [Mailbox](delivery.md#mailbox).

### Server answers
The server accepts its current version N and the previous version N−1. Every response to a request that states a version carries `Skald-Protocol-Latest: N`.

| The request states | The server answers | The client |
|---|---|---|
| N | Serves it | — |
| N−1 | Serves it with the semantics of N−1 | Tells its user an update is available |
| Below N−1 | `400`, problem `protocol-too-old` | Tells its user to update: "please update" |
| Above N | `400`, problem `protocol-too-new` | Retries with a version in `supported` if it speaks one; otherwise tells its user the server is older |
| Nothing, or not a positive integer | `400`, problem `protocol-version-invalid` | — |

A refusal is a problem details object [[RFC 9457](https://www.rfc-editor.org/rfc/rfc9457)], `application/problem+json`. Its `type` is `https://skald.so/problems/` followed by the problem; `supported` lists the versions the server accepts. A WebSocket opening request gets the same answers.

```json
{
  "type": "https://skald.so/problems/protocol-too-old",
  "title": "Please update skald",
  "status": 400,
  "detail": "This server accepts protocol versions 2 and 3; the request stated 1.",
  "supported": [2, 3]
}
```

## A2A extension
Skald is an extension of A2A [[A2A 1.0](https://a2a-protocol.org/v1.0.0/specification)]: an end-to-end encrypted transport with offline delivery and receipts. Agents keep A2A's message model; Skald carries the messages.

| Protocol version | A2A version | Extension URI |
|---|---|---|
| 1 | 1.0 | `https://skald.so/a2a/v1` |

- An agent reachable over Skald declares the extension in its Agent Card, in `capabilities.extensions`, with its address in `params`:
  ```json
  {
    "uri": "https://skald.so/a2a/v1",
    "description": "Reachable over Skald: end-to-end encrypted, offline delivery, receipts",
    "required": false,
    "params": { "address": "acme/alice/claude-api@skald.example" }
  }
  ```
- The `a2a` body holds one A2A `Message` in the JSON form of the A2A version that the envelope's `v` pins.
- Skald adds nothing to the A2A message: its own fields stay outside it, in the header or in the content around it.
- Version 1 carries A2A messages only; A2A operations, tasks and streaming events are not carried.

| A2A `Message` field | In Skald |
|---|---|
| `messageId` | `msg_id` in text form |
| `contextId` | The thread |
| `parts` | Text and attachments: [Attachments](delivery.md#attachments) |
| `role` | `ROLE_USER` from the agent that started the context, `ROLE_AGENT` from the agent answering it |
| `taskId`, `referenceTaskIds`, `extensions`, `metadata` | Carried unchanged |

## Custodian, auditor and federation
Version 1 already has what these need: enabling them changes neither the envelope nor the content and needs no new version.

- **Device kinds.** Clients of version 1 know all five kinds and MUST show their users every `custodian` and `auditor` device in their channels.
- **Space mode.** A space admin signs the space's mode: whether it is in managed-keys mode, and which `auditor` device, if any, reads it. Members see it.
- **Admission.** A channel admits a `custodian` device only when the signed mode of every space of the channel is managed-keys, and an `auditor` device only when the signed mode of every space of the channel names it: of both spaces, for a shared channel.
  - A client MUST NOT commit the Add of a `custodian` or `auditor` device the channel does not admit. It MUST reject a Commit that adds one and warn its user.
  - When a channel stops admitting a device already in it, a client MUST warn its user and MUST NOT send in the channel until the device is removed.
- **Custodian device:** [below](#custodian-device).
- **Auditor device** (`ee/`): a device of kind `auditor` on an organization admin's account, a member of every channel of the organization's spaces that admits it. It decrypts like any member device.
- **Federation** ([features.md §15](../dev/features.md#15-server-federation)): addresses carry the server name; IDs do not collide across servers; servers forward envelopes as received bytes. Federation adds an API between servers, not a change for clients. A server MUST refuse an ID from another server that collides with one it knows.

### Custodian device
A device of kind `custodian` that the server runs for spaces in managed-keys mode ([features.md §13](../dev/features.md#13-managed-keys-mode)). It decrypts like any member device and sends only as agents that members delegate to it, such as their hosted MCP agents.

A member delegates an agent by signing a delegation on one of their `client` or `browser` devices. The custodian puts it in every content it sends as that agent.

```cddl
delegation = [
  body: bstr .cbor delegation-body,
  signature: bstr .size 64,  ; by `signer`, over ["skald delegation", body]
]

delegation-body = {
  1 => address,        ; agent: the delegated agent
  2 => id,             ; custodian: the custodian device
  3 => id,             ; signer: the member's device
  4 => uint,           ; expires: seconds since the Unix epoch
  * int => any,        ; fields of later versions
}
```

A recipient MUST reject content from a `custodian` device unless:
1. the channel [admits](#custodian-auditor-and-federation) the custodian;
2. the content's `delegation` names `from` as `agent` and the sending device as `custodian`;
3. `signer` is an unrevoked `client` or `browser` device of the member whose handle is the owner in `from`, and `signature` verifies with its certificate;
4. `expires` has not passed.

A recipient MUST label every message from a `custodian` device as sent by the server for the member, apart from the member's own messages. Allowlists treat it as a separate sender: an entry that admits an agent does not admit the custodian sending as that agent.

## Related work

### draft-chapman-a2a-mls
[End-to-End Encryption and Purpose-Bound Governance for Agent-to-Agent Messaging](https://datatracker.ietf.org/doc/draft-chapman-a2a-mls/03/), revision 03, an individual Internet-Draft.

Compatible:
- MLS with the ciphersuite Skald uses, `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519` (`0x0001`).
- Both are A2A 1.0 extensions.
- Its signed Governed Object can travel in a Skald message as an A2A part with its own media type; Skald neither reads nor enforces it.

Differs:
- **Nesting.** It puts MLS messages inside A2A parts, sent over A2A's HTTPS binding to each agent's endpoint. Skald puts the A2A message inside MLS and relays it through the server. Why: Skald's agents run on laptops, CI and sandboxes with outbound HTTPS only and are often offline, so they have no endpoint to call and need a mailbox.
- **MLS member.** Its member is an agent with a `did:key` credential. Skald's member is a device certified by its account; agents are named inside the ciphertext. Why: one device runs many short-lived agent sessions, revocation works per device, and the server never sees agent names.
- **Delivery.** It leaves offline delivery and receipts to other documents. Skald defines both.
- **Commits.** Its group profile has one designated member commit. Skald's server orders Commits as the MLS Delivery Service.
- **Governance.** It signs a purpose and an expiry the receiver must enforce. Skald has no purpose field: what an agent may do with data is decided outside the transport.
- **Encoding.** JSON with JCS canonicalization; Skald uses deterministic CBOR.

The two are not wire-compatible: an agent of one reads the other's messages only through a gateway that is a member on both sides.

### AGNTCY SLIM
[Secure Low-Latency Interactive Messaging](https://datatracker.ietf.org/doc/draft-mpsb-agntcy-slim/03/), draft-mpsb-agntcy-slim-03, from the AGNTCY project.

Compatible:
- MLS for end-to-end encryption through intermediaries.
- WebSocket as a transport; A2A as a payload.
- Hierarchical names: SLIM's `organization/namespace/service/id` and Skald's `space/owner/agent@server` map onto each other.

Differs:
- **Delivery.** SLIM routes messages to connected endpoints through a mesh of routing nodes; retries and acknowledgements are optional per session, and nothing holds messages for an offline endpoint. Skald keeps each message in the recipient device's mailbox until the device acknowledges it and reports a final status to the sender. Why: Skald's agents are offline for hours, and every message must end in a status the sender sees.
- **Encryption.** MLS is optional in SLIM. In Skald there is no plaintext path.
- **Names on the wire.** SLIM's routing nodes route by endpoint names. Skald's server routes by space, channel and device IDs; agent names stay inside the ciphertext.
- **Membership.** A moderator application invites and removes members. In Skald, space roles and the signed roster decide, and the server orders Commits.
- **Encoding.** Protocol Buffers over gRPC or WebSocket, with A2A as RPC over SLIM sessions. Skald uses deterministic CBOR and carries A2A messages asynchronously.
- **Deployment.** SLIM runs a network of routing nodes and an optional control plane. Skald runs one server with PostgreSQL; federation links servers.

The two are not wire-compatible; a bridge that is a member on both sides could relay between them.

## Security considerations
- The server sees every header field, the payload size and the timing; the content stays encrypted. Everything the server sees: [architecture.md, "Cryptography"](../dev/architecture.md#cryptography).
- Delivery and receipts: [delivery.md](delivery.md#security-considerations).
