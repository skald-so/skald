# Skald protocol: devices, roster and channels

Part of the [Skald protocol](README.md), with its conventions and status. **No external audit.**

CDDL rules not defined here, such as `id`, `address` and `timestamp`, are in [README.md](README.md) and [delivery.md](delivery.md).

## Contents
- [Signed objects](#signed-objects)
- [Account](#account)
- [Device certificate](#device-certificate)
- [Linking a device](#linking-a-device)
- [Revocation](#revocation)
- [Device checks](#device-checks)
- [Trust levels](#trust-levels)
- [Roster](#roster)
- [Policy](#policy)
- [Invite](#invite)
- [Admin actions from a browser](#admin-actions-from-a-browser)
- [Channels](#channels)
- [History](#history)
- [Security considerations](#security-considerations)

## Signed objects
Every object here that a signature protects has the same form:

```cddl
signed<body> = [
  body: bstr .cbor body,
  signature: bstr .size 64,  ; over [label, body]
]
```

- The signature is Ed25519 over the encoded CBOR array of the object's label and `body` as a byte string ([Conventions](README.md#conventions)).
- The signer is a device, with the signing key of its certificate, or the account's master key where this document says so.
- A [certificate](#device-certificate) also carries a second signature, its device's `proof`.
- Labels: `skald account`, `skald cert`, `skald proof`, `skald link`, `skald anchors`, `skald revocation`, `skald roster`, `skald invite`, `skald acceptance`, `skald wrap`, `skald history`. The [SSH signature](#device-certificate) has its own form, with the label `skald ssh`.

## Account
Each account has two keys, both created on its first device:
- **Master key:** an Ed25519 key pair, the account's root of trust. It signs the account record and the first device's certificate. Devices do not keep its private key.
- **Recovery key:** 32 bytes from a cryptographically secure generator, shown to the user once. Shown and entered as base32 [[RFC 4648](https://www.rfc-editor.org/rfc/rfc4648), §6] without padding, in groups of four characters joined by `-`; input ignores case, spaces and `-`. Its HPKE key pair is `DeriveKeyPair(recovery key)` of DHKEM(X25519, HKDF-SHA256) [[RFC 9180](https://www.rfc-editor.org/rfc/rfc9180), §7.1.3].

```cddl
account = signed<account-body>   ; label "skald account", by the master key

account-body = {
  1 => id,                 ; account
  2 => bstr .size 32,      ; master: the master public key
  3 => bstr .size 32,      ; recovery: the recovery HPKE public key
  4 => uint .ge 1,         ; revision
  * int => any,            ; fields of later versions
}

master-backup = [
  enc: bstr .size 32,      ; the HPKE encapsulated key
  ct: bstr,                ; the 32-byte master private key, sealed
]
```

- The server stores the account record and the master backup.
- **Master backup:** the master private key sealed with HPKE in base mode to the recovery HPKE key, `info` `skald master backup`, `aad` the account ID. The recovery key unlocks it; the client checks that the opened key matches `master`.
- A new recovery key: a device unlocks the master key with the old one, stores a new backup and signs an account record with the next `revision`. A client uses the valid record with the highest `revision`. The old recovery key still opens the master key: [Security considerations](#security-considerations).
- A client MUST reject an account record whose signature does not verify with `master`, or whose `master` is not the one it pinned: its own account's, or the one the [roster](#roster) gives for a member.

## Device certificate
A certificate binds a device's keys to its account.

```cddl
cert = [
  body: bstr .cbor cert-body,
  signature: bstr .size 64,  ; by `issuer`, over ["skald cert", body]
  proof: bstr .size 64,      ; by `signing`, over ["skald proof", body]
]

cert-body = {
  1 => id,                 ; account
  2 => id,                 ; device: derived from `signing`
  3 => device-kind,
  4 => bstr .size 32,      ; signing: the Ed25519 public key
  5 => bstr .size 32,      ; hpke: the X25519 public key
  ? 6 => id,               ; issuer: the device that signed it; absent, the master key
  7 => timestamp,          ; issued
  ? 8 => bstr,             ; ssh: an SSH signature of the device key
  9 => tstr .size (1..64), ; name: set on the device, e.g. "Phone"
  * int => any,            ; fields of later versions
}

device-kind = &(client: 1, browser: 2, service: 3, custodian: 4, auditor: 5)
```

- Kinds: [Identifiers](README.md#identifiers). Keys are generated on the device and never leave it.
- **Proof check:** a client MUST discard a certificate unless `device` is the ID derived from `signing` ([Identifiers](README.md#identifiers)) and `proof` verifies with `signing`. A discarded certificate counts nowhere and is not shown: the certificate of a device ID, a device's chain and issuers, the signer of an object and the devices a device certified come only from certificates that pass this check, here and in [README.md](README.md). So only the holder of a device's signing key makes a certificate for its ID: no other device can certify an existing device's ID, with other keys or with its own.
- A device signs `proof` for one certificate only, the one issued for it when it joins the account. Certificates of one `device` with the same `body` are one certificate: a client uses a copy whose `signature` passes check 1 of [Device checks](#device-checks). A client that sees two certificates of one `device` with different bodies that pass the proof check treats neither as valid and warns its user.
- A device ID thus names one device everywhere it appears, such as a revocation's `device`, `keep` and `also`, a member's `excluded` and an MLS credential.
- Issuers:
  - the master key: a `client` device, the account's first one or one linked with the recovery key;
  - a `client` device of the same account: any kind.
- A `service` device is certified on the account of the admin who creates it, a `custodian` device on that of the admin who enables managed-keys mode, an `auditor` device on an organization admin's account.
- `name` and `issued` are for display; nothing depends on them. A client shows a device only by its certificate's `name`, `kind` and `issued`, never by a name the server gives.
- **SSH signature** (`ssh`, optional): an SSH signature [[OpenSSH PROTOCOL.sshsig](https://github.com/openssh/openssh-portable/blob/master/PROTOCOL.sshsig)] with namespace `skald.so` over the encoded `["skald ssh", account, device, signing]`, all byte strings. It is made with one of the user's SSH keys on GitHub. A client that checks it fetches the user's keys from GitHub, not from the Skald server; a signature that does not verify is a key change.
- An MLS credential names the device: [MLS profile](#mls-profile).

## Linking a device
A new device generates its keys, derives its ID from its signing key and registers its public keys and `name` with the server, which refuses an ID it already has. It then gets a certificate:
- **From a linked device:** the user confirms the link on a `client` device of the account, which issues the certificate.
  - Both devices show the link fingerprint: the first 10 bytes of SHA-256 over the encoded `["skald fingerprint", master, signing, hpke, name]`, in base32 as above. The new device computes it with its keys, its `name` and the master key the server gave it; the issuing device with the keys and `name` it received and its own master key, and certifies that `name`. The user confirms they match, by eye or by scanning a QR code that carries it.
  - The issuing device stores the certificate's `body` and `signature`; the new device checks that `body` holds its account, ID, keys and `name`, signs `proof` and stores the complete certificate.
  - A `browser` is linked only this way.
  - With the certificate, the issuing device stores a link object for the new device: the [roster](#roster) revisions it starts from.
- **With the recovery key**, entered only in the CLI or `skald ui`: the new device unlocks the master key and issues its own certificate with it. It shows its user the account's devices, revoked or not, and in this order:
  1. revokes with the master key each device its user names as stolen ([Revocation](#revocation));
  2. takes its [anchors](#anchors);
  3. revokes with its own key each device its user names as lost or wiped;
  4. deletes the master private key.
- The first device creates the master key, the recovery key, the account record and the master backup, issues its own certificate with the master key, then deletes the master private key.

```cddl
link = signed<link-body>         ; label "skald link", by `issuer`

link-body = {
  1 => id,                 ; device: the new device
  2 => id,                 ; issuer
  3 => [* [space: id, revision: bstr .size 32]],  ; anchors: hash of the latest revision the issuer accepted of each of its spaces
  * int => any,            ; fields of later versions
}
```

- The new device MUST reject a link object unless `device` is itself and the signature verifies with the certificate of `issuer`, a valid `client` device of its account. Each anchor is a revision it starts from: [Roster](#roster).
- A device linked with the recovery key has no link object: it starts from [anchors](#anchors).

### Anchors
Each `client` device keeps on the server an anchors object: the roster revisions a device linked with the recovery key starts from.

```cddl
anchors = signed<anchors-body>   ; label "skald anchors", by `device`

anchors-body = {
  1 => id,                 ; account
  2 => id,                 ; device: a `client` device of the account
  3 => [* [space: id, revision: bstr .size 32]],  ; as in a link object
  * int => any,            ; fields of later versions
}
```

- A `client` device signs a new anchors object when it accepts a revision; the server keeps the latest per device.
- A device linked with the recovery key takes the anchors objects after it revokes the stolen devices. It MUST reject one unless `account` is its own and the signature verifies with the certificate of `device`, a `client` device whose certificate passes checks 1 and 2 of [Device checks](#device-checks) and that no master-key revocation marks as stolen. Other revocations do not matter here, so anchors of lost or wiped devices count.
- For each space, it starts from the anchor with the lowest revision number and accepts later revisions as [Roster](#roster) says. Every other anchor of the space MUST be a revision it so accepts; otherwise it warns its user and does not join the space. It does not join a space no valid anchors object names.

## Revocation
A revocation removes a device from its account.

```cddl
revocation = signed<revocation-body>   ; label "skald revocation", by `signer`

revocation-body = {
  1 => id,                 ; account
  2 => id,                 ; device: the revoked device
  ? 3 => [+ id],           ; keep: certified devices that stay valid
  ? 4 => id,               ; signer: absent, the master key
  5 => timestamp,          ; for display
  ? 6 => [+ id],           ; also: devices that stay revoked; only with the master key
  * int => any,            ; fields of later versions
}
```

- The signer is a `client` or `browser` device of the account, or its master key.
- A revocation revokes `device`, each device in `also`, and every device one of them certified, except those in `keep`. The revoking device shows its user the certified devices to choose from.
- Of several counting revocations of one device, a device it certified stays valid only if every one lists it in `keep`.
- A client MUST ignore a revocation unless its signature verifies with the master key, or it has no `also` and its signature verifies with the certificate of `signer`, a `client` or `browser` device of `account` whose certificate passes checks 1 and 2 of [Device checks](#device-checks). Otherwise it counts, whether or not its signer is revoked itself, until a master-key revocation voids it.
- A master-key revocation marks as stolen `device` and every device whose chain passes through it other than through a device in `keep`, and voids every revocation they signed. The device that signs it shows its user the devices the voided revocations named; it lists in `also` those the user keeps revoked, so one object voids and re-revokes.
- Only voiding undoes a revocation; a master-key revocation is never undone.
- The server stores revocations and gives them with the certificates.

## Device checks
A server signature alone never admits a device: a client trusts a device only through its certificate chain to the account's master key.

A certificate is valid when:
1. it passes the [proof check](#device-certificate), and `signature` verifies: with the master key when `issuer` is absent; otherwise with the signing key of the issuer's certificate;
2. its chain, the certificate and those of its issuers in turn, ends at the master key without repeating a device; every certificate of the chain passes check 1, and every issuer is a `client` device of the same account;
3. no counting revocation names the device in `device` or `also`;
4. for each revoked issuer in the chain, every counting revocation of it lists in `keep` the next device down the chain.

The master key is the one the client pinned: its own account's on linking, or the `master` of the member in the [roster](#roster). A device is trusted only with a valid certificate; a client MUST NOT send to, accept content from or admit to a channel any other device.

A client MUST warn its user when:
- an account's device set changes: a device is added, revoked or restored by a voided revocation;
- an account record's `recovery` changes;
- a member's `master` changes in the roster;
- an `ssh` signature it checks does not verify;
- two valid devices of an account have the same `name`.

## Trust levels
A recipient device sets the level of each message it accepts from the sending device: the one its scheme authenticates ([Envelope](README.md#envelope)). The level sets the restricted mode of the [policy](#policy) for a task.

| Level | The sending device |
|---|---|
| `own` | A `client` or `browser` device of the recipient's account |
| `member` | A `client` or `browser` device of an account the recipient's space roster lists as a member |
| `service` | Any other: a `service` or `custodian` device, or a member of the other space of a [shared channel](#shared-channel) |

- The level comes from the sending device's valid certificate, its `account` and `kind`, and the roster of the recipient's space; never from the content.
- `own` is above `member`, which is above `service`.

## Roster
A space's roster lists its members, service agents and channels, and holds its policy. Every revision is a full snapshot, signed by an admin's device.

```cddl
roster = signed<roster-body>     ; label "skald roster", by `signer`

roster-body = {
  1 => id,                 ; space
  2 => tstr,               ; name: the space in addresses
  3 => uint .ge 1,         ; revision
  ? 4 => bstr .size 32,    ; prev: SHA-256 of the previous revision's roster; absent in revision 1
  5 => id,                 ; signer: an admin's device
  6 => timestamp,          ; for display
  7 => [+ member],
  8 => [* service-agent],
  9 => [* channel],
  10 => policy,
  * int => any,            ; fields of later versions
}

member = {
  1 => id,                 ; account
  2 => tstr,               ; handle: the owner in its agents' addresses
  3 => role,
  4 => bstr .size 32,      ; master: the account's master public key
  ? 5 => id,               ; invite: the invite it joined with
  ? 6 => [+ id],           ; excluded: its devices an admin barred from the space
  * int => any,
}

role = &(admin: 1, member: 2)

service-agent = {
  1 => id,                 ; device: a `service` device
  2 => tstr,               ; owner: e.g. "ci"
  3 => tstr,               ; agent: e.g. "deploy-bot"
  * int => any,
}

channel = {
  1 => id,                 ; channel
  2 => [* channel-member],
  3 => [* id],             ; services: `service` devices in its MLS group
  ? 4 => shared,           ; only in a shared channel
  * int => any,
}

channel-member = [
  account: id,
  history: bool,           ; whether it reads the history from before it joined
]
```

- A space's creator signs revision 1 and is its first admin.
- A revision's hash is SHA-256 over its encoded `roster`.
- The server keeps every revision. It stores revision n+1 only if it has revision n, so of two revisions signed at once it keeps the first; it refuses the other, and its signer applies its change to the new revision and signs again.
- It sends a [`space-notice`](delivery.md#frames) to the space's devices when it stores a revision.
- A `service` device sends only as `space/owner/agent` from its entry ([Envelope](README.md#envelope)).

A client starts from an anchor: revision 1 of a space it created, the revision its [invite](#invite) link names, or one from its [link object](#linking-a-device) or [anchors](#anchors). It MUST NOT accept revision n+1 before it has accepted revision n, nor unless:
1. its signature verifies with the certificate of `signer`, a valid `client` or `browser` device of an account that is an admin in revision n;
2. `space` and `name` are those of revision n, `revision` is n+1, and `prev` is the hash of revision n;
3. it has at least one admin; accounts, devices, channels and invites are not repeated; handles and owners are valid names ([Agent address](README.md#agent-address)), unique among both;
4. each `service-agent` device has a valid certificate of kind `service` on a member's account; each `channel-member` is a member; `policy` passes [its checks](#policy).

A revision that fails, and every revision built on it, has no effect: the client keeps the last revision it accepted, warns its user and keeps working with that one. Two valid revisions with the same number mean the server forked the roster: the client keeps the one it accepted and warns its user.

### Shared channel
A shared channel ([features.md §3](../dev/features.md#3-spaces-and-shared-channels)) is in the rosters of both its spaces, each entry with the other side in `shared`.

```cddl
shared = [
  space: id,               ; the other space
  revision: bstr .size 32, ; hash of a revision of its roster the signer accepted
]
```

- The channel is shared only while both rosters list it, each naming the other.
- Its members are those of both entries. A client checks the other space's roster from the revision in `shared` onward, as it checks its own.
- A shared channel's messages carry `space` of the sender's side ([Envelope](README.md#envelope)).

## Policy
A space's policy travels in its roster, so an admin's device signs every change.

```cddl
policy = {
  1 => [own: mode, member: mode, service: mode],  ; restrictions: per trust level
  2 => uint .ge 86400,     ; history: the history key's longest age, in seconds
  3 => edit-action,
  ? 4 => id,               ; custodian: managed-keys mode, with this `custodian` device
  ? 5 => id,               ; auditor: the `auditor` device that reads the space
  * int => any,            ; fields of later versions
}

mode = &(read-only: 1, no-shell: 2, no-network: 3, full: 4)
edit-action = &(notify: 1, warn: 2, ask: 3, block: 4)
```

- `restrictions`: the mode a task runs in, by the [trust level](#trust-levels) of its sender. `read-only` reads files; `no-shell` also edits them; `no-network` also runs commands, without network; `full` has no limits.
- `history`: [History key](#history-key).
- `edit-action`: [features.md §11](../dev/features.md#11-cross-machine-edit-coordination).
- `custodian` and `auditor`: the space mode, [Custodian, auditor and federation](README.md#custodian-auditor-and-federation). Without `custodian`, the space is end-to-end encrypted.
- A new space starts with `[full, read-only, read-only]`, 30 days, `warn`, and neither `custodian` nor `auditor`.
- Checks: `custodian` and `auditor` name devices of their kind with valid certificates on members' accounts.

## Invite
An admin invites a person by link. The link's secret proves the newcomer is the one invited and never reaches the server.

```cddl
invite = signed<invite-body>     ; label "skald invite", by `signer`

invite-body = {
  1 => id,                 ; invite: assigned by the server
  2 => id,                 ; space
  3 => bstr .size 32,      ; key: the Ed25519 public key derived from the secret
  4 => role,
  5 => [* [channel: id, history: bool]],  ; channels to add the newcomer to
  6 => id,                 ; signer: an admin's device
  7 => timestamp,          ; expires
  * int => any,            ; fields of later versions
}

acceptance = signed<acceptance-body>  ; label "skald acceptance", by the invite key

acceptance-body = {
  1 => id,                 ; invite
  2 => id,                 ; account: the newcomer
  3 => bstr .size 32,      ; master: the newcomer's master public key
  4 => tstr,               ; handle: the handle it asks for
  * int => any,
}
```

- **Secret:** 32 bytes from a cryptographically secure generator on the admin's device. The invite key pair has the Ed25519 private key `HKDF-SHA256(salt = "", IKM = secret, info = "skald invite", L = 32)` [[RFC 5869](https://www.rfc-editor.org/rfc/rfc5869)].
- **Link:** `https://<server>/invite/<invite>#<fragment>`; the fragment is base64url without padding of the secret followed by the hash of the roster revision the admin's device last accepted. Browsers do not send a fragment to the server.
- The server stores the invite and the acceptance. Revoking an invite deletes it on the server.

The newcomer's device MUST, from the link:
1. take the roster revision with the hash in the fragment, check its hash and accept it as its anchor, then accept later revisions as [Roster](#roster) says;
2. check the invite's signature with `signer`, a valid `client` or `browser` device of an admin per those revisions, and that `key` is the one derived from the secret;
3. only then prove it holds the secret: sign an acceptance with the invite key, for its own account, master key and handle.

An admin's device, `client` or `browser`, MUST NOT admit a newcomer unless it:
1. checks the invite: its signature with the certificate of `signer`, a valid `client` or `browser` device of an admin in the latest revision, `expires` not passed, no member of a revision it accepted with its `invite`;
2. checks the acceptance's signature with the invite's `key`;
3. checks that the account is not a member and the handle is free;
4. checks each of the newcomer's devices: a valid certificate chained to `master` from the acceptance, not to one the server gives.

It then signs the next revision with the member, its `invite` and the invite's role and channels. The newcomer's devices join the channels: [Group changes](#group-changes).

## Admin actions from a browser
A linked browser is not an MLS member, but an admin's browser signs revisions like a `client` device: invites, accepting newcomers, roles, removing members, policy.

- The browser stores the revision on the server; the server sends a `space-notice`.
- Member devices bring the channels' MLS groups in line with the accepted revision, as for any revision: [Group changes](#group-changes). Nothing waits for the admin's `client` device.

## Channels
A channel is an MLS group [[RFC 9420](https://www.rfc-editor.org/rfc/rfc9420)] whose `group_id` is the channel ID. The server is its Delivery Service: it stores KeyPackages and GroupInfo, and orders Commits.

### MLS profile
- Ciphersuite `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519` (`0x0001`).
- Credential: `basic`, its `identity` the 16-byte device ID. A leaf's `signature_key` MUST equal the `signing` key of the device's certificate, so `identity` is the ID derived from `signature_key`.
- Members: devices of kind `client`, `custodian` and `auditor`, and `service` devices that keep state. A `browser` and a stateless `service` device are never members.
- Application messages are PrivateMessages. Proposals and Commits are PublicMessages, so the server knows each epoch's member devices.
- Each Commit's sender uploads the GroupInfo of the new epoch, with the `ratchet_tree` and `external_pub` extensions, over HTTPS. Any member device of the current epoch uploads one if the server has none. The server keeps the latest.

### Admitted devices
A device is admitted to a channel when its certificate is valid and it is one of:
- a `client` device of an account in the channel's `channel-member` entries in the latest revision the client accepted, unless that member's `excluded` lists it;
- a device in the channel's `services`;
- the `custodian` or `auditor` device the [space mode](README.md#custodian-auditor-and-federation) admits.

For a shared channel, both spaces' entries count.

A member device:
- MUST NOT commit the Add of a device that is not admitted;
- before it treats a device a Commit adds as not admitted, MUST fetch the space's latest revisions and accept them as [Roster](#roster) says;
- after it processes a Commit or accepts a revision, MUST commit the Remove of each member that is not admitted, and MUST NOT send application messages in the channel until no such member is left;
- MUST warn its user when a Commit by another device added a device that is not admitted, and when a `custodian` or `auditor` device in the channel stops being admitted.

### Creating a channel
1. An admin's device gets a channel ID from the server and signs a revision with the channel.
2. The first admitted device to accept the revision creates the channel's group at epoch 0, adds every admitted device it has a KeyPackage for and commits: epoch 1.
3. The server keeps the first Commit of epoch 0 it receives for the channel and refuses later ones with `epoch-mismatch` ([Send](delivery.md#send)). A device whose group is refused drops it and joins the kept one.

### KeyPackages
- Every device that can be a member keeps unused KeyPackages on the server: when fewer than 10 remain, it uploads enough to have 20. Their `lifetime` is at most 60 days; a device replaces a KeyPackage 7 days before it expires.
- Each device also keeps one last-resort KeyPackage, which the server hands out only when no other remains and does not delete. The device replaces it once it was used, and as the others.
- The server hands out each other KeyPackage once, then deletes it, and deletes expired ones. It hands them only to devices of accounts that share a space with the KeyPackage's device.
- A device MUST NOT add a KeyPackage unless its MLS signature verifies, its `lifetime` holds, its ciphersuite is `0x0001`, and its `identity` and `signature_key` match an admitted device's valid certificate.

### Commit ordering
- The server keeps a channel's current epoch from the Commits it accepts. It accepts a Commit only if:
  1. its `epoch` is the channel's current epoch; otherwise it refuses it with `epoch-mismatch`;
  2. its sender is a member of that epoch whose `identity` is the device sending it, or, for an external Commit, the joining device is the one sending it; otherwise `not-a-member`.
- So of Commits sent at once, the first stored wins. The others' senders process it, then commit again if their change is still needed.
- A Commit and its Welcome travel in envelopes: the Commit to the channel, the Welcome with `recipients` the added devices ([Envelope](README.md#envelope)). Their mailbox entries do not expire by time ([Mailbox](delivery.md#mailbox)).
- Member devices process a channel's handshake messages in its [order](delivery.md#order).

### Group changes
| Change | Who commits it |
|---|---|
| A newcomer's devices, from an invite | The admin's `client` device that signed the revision, with their KeyPackages. Otherwise, each device joins by itself |
| A member's new device | The `client` device that certified it, in the channels it is in. Otherwise, the new device joins by itself |
| A new channel member | The device that signed the revision, if it is a member device of the channel. Otherwise, each device joins by itself |
| A device revoked or excluded, a member removed | Any member device that learns of it: Remove |
| A device that is not admitted | Any member device: Remove |
| A rotation of the history key | [History key](#history-key) |

- A Commit with the Remove of a revoked device, or the empty Commit after a browser's revocation ([History key](#history-key)), carries in its `authenticated_data` the encoded `[+ revocation]`: the revocations by which its sender holds each such device revoked. Every member device that processes the Commit MUST check them as [Revocation](#revocation) says and apply those that count before it judges the new epoch ([Clean epochs](#clean-epochs)).
- **Joining by itself:** an admitted device that is not in the group fetches the latest GroupInfo, checks that its signer is a member and the members it lists, and joins with an external Commit [[RFC 9420](https://www.rfc-editor.org/rfc/rfc9420), §12.4.3.2]. Members check the joiner as an Add.
- A device that gets a Welcome checks the signer and every member it lists the same way.

### Rejoining
- **After missing Commits:** a device that cannot process a channel's next handshake message, or that a Commit removed while it is admitted, rejoins with an external Commit that also removes its old leaf. It MUST warn its user if a Commit removed it while it was admitted.
- **After a wipe:** the device lost its keys and counters, so it is a new device: it is linked again ([Linking a device](#linking-a-device)) and joins as a member's new device. The user revokes the old one.
- **Offline past the mailbox retention:** handshake messages do not expire by time, so the device processes the Commits it missed in order. The application messages that expired are reported by [gap detection](delivery.md#gap-detection); the device reads them from the [history](#history).

## History
The server keeps a copy of each channel message under the channel's history key, so new devices, linked browsers and the recovery key read past messages.

### History key
- The history key of epoch e of a channel's group is `MLS-Exporter("skald history", "", 16)` in that epoch [[RFC 9420](https://www.rfc-editor.org/rfc/rfc9420), §8.5]: every member device of the epoch derives it, so it needs no distribution of its own.
- The key rotates with every epoch:
  - **revocation:** a Remove of a revoked device starts an epoch; when a browser of a channel member's account is revoked, a member device commits an empty Commit before it next sends in the channel;
  - **schedule:** when the current epoch is older than the policy's `history`, the first member device to notice commits an empty Commit.
- An empty Commit has an UpdatePath and no proposals.

### Clean epochs
An epoch is clean when every member device of it is [admitted](#admitted-devices). A member device judges each epoch once, when it enters it, by the latest revision it accepted. A member device MUST NOT upload a link, wrap or entry of an epoch that is not clean.

### Key chain
A link of a clean epoch e holds the key of `prev`, the latest epoch before e that its uploader judged clean. A device linked with the recovery key also counts as judged clean each epoch before its first epoch in the channel whose key a [wrap](#wraps) gives it. A reader follows every link it can open, so a device holding a key reaches every earlier epoch that an uploader of a link judged clean.

```cddl
history-link = {
  1 => id,                 ; channel
  2 => uint .ge 2,         ; epoch e
  3 => uint .ge 1,         ; prev: the latest epoch before e its uploader judged clean
  4 => bstr .size 12,      ; nonce
  5 => bstr,               ; the key of epoch `prev`, encrypted
  * int => any,
}
```

- AES-128-GCM [[RFC 5116](https://www.rfc-editor.org/rfc/rfc5116)] with the key of epoch e, a random `nonce` and associated data the encoded `["skald history link", channel, e, prev]`, with `e` and `prev` unsigned integers.
- Epoch 0 holds only the creator and no messages and is never `prev`, so a clean epoch with no clean epoch before it but 0, such as epoch 1, has no link.

### Wraps
Browsers and recovery keys, which are outside the group, get each epoch's key wrapped to them.

```cddl
history-wrap = signed<history-wrap-body>  ; label "skald wrap", by `signer`

history-wrap-body = {
  1 => id,                 ; channel
  2 => uint .ge 1,         ; epoch
  3 => wrap-recipient,
  4 => id,                 ; signer: a member device of the epoch
  5 => bstr .size 32,      ; enc: the HPKE encapsulated key
  6 => bstr,               ; ct: the key, sealed
  * int => any,
}

wrap-recipient = [1, device: id] / [2, account: id]  ; a browser, or an account's recovery key
```

- HPKE in base mode as in [Scheme `hpke`](README.md#scheme-hpke), to the `hpke` key of a `browser` device's certificate or to the recovery HPKE key of an account's [record](#account); `info` `skald history wrap`, `aad` the encoded `[channel, epoch, recipient]`.
- Each clean epoch's key is wrapped to the recovery key of each account among the channel's members and to each browser of those accounts with a valid certificate. `client` devices take keys from MLS and the key chain and need no wrap.
- A browser linked later gets wraps of the current epochs from member devices that find them missing.

A reader MUST drop a wrap unless `ct` opens with its HPKE key and the signature verifies with the certificate of `signer`, revoked or not. That certificate MUST pass checks 1 and 2 of [Device checks](#device-checks) with the master key of an account that an accepted revision lists as a member of the space or, for a shared channel, of the other space.

A browser writes entries only in the epoch of the channel's latest GroupInfo, under the key of a wrap of that epoch. It MUST NOT write any unless that GroupInfo is of the channel and the wrap's epoch, its signature verifies with the `signature_key` of a member leaf, it lists `signer` as a member, and every member it lists is admitted.

### Uploads
- The sender of each Commit uploads the new epoch's link and wraps. Any member device of an epoch uploads those it finds missing and can make.
- Devices can judge an epoch differently, for example when only one has seen a revocation, so each uploads the link its own judgment gives.
- The server keeps the first link per channel, epoch and `prev` and the first wrap per recipient, and takes uploads only from member devices of the epoch.

### History entry
The sending device writes a copy of each message with an `a2a` or [`task-result`](queues.md#results) body it sends in the channel, under the key of its current epoch; a browser as [Wraps](#wraps) says. A device without a history key, such as a stateless `service` device, writes none.

```cddl
history-entry = {
  1 => id,                 ; channel
  2 => uint .ge 1,         ; epoch
  3 => bstr .size 12,      ; nonce
  4 => bstr,               ; history-plaintext, encrypted
  * int => any,
}

history-plaintext = signed<history-body>  ; label "skald history", by `sender`

history-body = {
  1 => id,                 ; sender: the sending device
  2 => id,                 ; channel
  3 => id,                 ; msg_id
  4 => timestamp,          ; when it was sent, as the sender states it
  5 => bstr .cbor content,
  * int => any,
}
```

- AES-128-GCM with the epoch's key, a random `nonce` and associated data the encoded `["skald history entry", channel, epoch]`.
- The server keeps one entry per sending device and `msg_id`. It takes an entry only for the channel's current or previous epoch, and only from a member device of the current epoch or a browser with a wrap of the current epoch.

A reader MUST drop an entry unless:
1. it decrypts with the epoch's key;
2. `channel` matches the entry's, and `signature` verifies with the certificate of `sender`, revoked or not: a device of an account that an accepted revision lists among the channel's members, or one of its `services`, whose certificate passes checks 1 and 2 of [Device checks](#device-checks) with the `master` that revision gives for its account;
3. its content passes checks 2 and 4 of [Envelope](README.md#envelope), with `sender` as the sending device.

### Access
- A member's device reads the history through its keys: from its first epoch back along the key chain.
- A channel member whose `history` is false reads only epochs from the one whose Commit added its account. The server gives its devices neither the links of that epoch nor any link, wrap or entry of an earlier one.
- The recovery key reads every epoch wrapped to it: a device linked with it reads the account's history before it rejoins the channels.

## Security considerations
- A device ID keeps 122 bits of a hash of the signing key. Taking an existing device's ID means finding a key for it, about 2^122 hashes. Two keys with one ID take about 2^61; both are then their finder's, and a client that sees both certificates treats neither as valid.
- The server gives the certificates, revocations and revisions. It cannot forge them, but it can withhold them: a hidden revocation keeps a device trusted for those who do not see it, and a hidden revision keeps clients on an older roster.
- A stolen device can revoke the account's other devices and certify new ones, but not under an existing device's ID, and its revocations count even after another device revokes it. The recovery key restores the account: a device linked with it revokes the stolen one with the master key. That one object voids every revocation the stolen device and the devices it certified signed, and lists in `also` the devices that stay revoked, so the server cannot restore them by withholding a second object.
- The master key cannot be rotated. A leaked recovery key, current or old, with any copy of a master backup it opens, gives away the master key for good: its holder can certify and revoke devices and sign an account record with its own `recovery`, so member devices wrap history keys to it. A new recovery key does not lock it out: the holder signs a record with a higher `revision` again. Clients warn when `recovery` changes.
- A device linked with the recovery key trusts the revisions anchored by its account's `client` devices other than those its user names as stolen. A stolen device the user does not name counts as honest: in a space no other device anchored, it can make the device start from a revision the server forged. Where other devices anchored the space, its anchor off their chain only stops the device from joining. The server can withhold anchors objects: the device then starts from older revisions, or joins no space.
- A newcomer trusts the roster revision named in the invite link; the link's integrity rests on the channel it was sent over.
- A channel member whose `history` is false holds its join epoch's key, which opens the links to the epochs before it: only the server withholding those links keeps it from earlier history.
- The server holds GroupInfo, so it can join a channel by an external Commit from its own device. Members remove the device, and its epoch is not clean: no link opens an earlier key with that epoch's key, and no member writes under it.
- A browser writes only under the key of a wrap whose signer the epoch's GroupInfo lists, with every member admitted, so the server cannot choose the key a browser writes under.
- A reader takes a wrap signed by any device of a member account, revoked or not. A member device can upload a wrong link or wrap first. That breaks reading the history, not its authenticity: every entry is signed.
- A revoked device reads a channel until a member device that holds its revocation commits its Remove, or the empty Commit for a browser; the server delays this by withholding the revocation from every member device. A member device that did not process that Commit, such as one that joined later, learns the revocation only from the server: if the server withholds it and lets the device rejoin, that member's links open to it keys of epochs after the Remove.
- An entry's `timestamp` is its sender's claim. The server, not the reader, keeps a removed member from adding entries, and a member from adding them to epochs before the previous one.
- A member can make a channel unusable with a Commit the others cannot process; the channel is then replaced by a new one.
- Everything the server sees: [threat model, "The server sees"](../threat-model.md#the-server-sees).
