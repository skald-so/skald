# Skald protocol: devices, roster and channels

Part of the [Skald protocol](README.md), with its conventions and status. **No external audit.**

CDDL rules not defined here, such as `id`, `address` and `timestamp`, are in [README.md](README.md) and [delivery.md](delivery.md).

## Contents
- [Signed objects](#signed-objects)
- [Account](#account)
- [Device certificate](#device-certificate)
- [Linking a device](#linking-a-device)
- [Service device](#service-device)
- [Revocation](#revocation)
- [Device checks](#device-checks)
- [Account fingerprint](#account-fingerprint)
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
- [Link code](#link-code) labels: `skald link code` in the CPace channel identifier; `skald link request` and `skald link grant` in key derivation.
- [Device key session](#device-key-session) label: `skald device session`.
- [Invite sign-up](#invite) label: `skald invite sign-up`.

## Account
Each account has two keys, both created on its first device:
- **Master key:** an Ed25519 key pair, the account's root of trust. It signs the account record and the first device's certificate. Devices do not keep its private key.
- **Recovery key:** 12 characters of [Crockford's base32](https://www.crockford.com/base32.html), `0123456789ABCDEFGHJKMNPQRSTVWXYZ`, from a cryptographically secure generator: 60 bits. Shown to the user once, in upper case, as 3 groups of 4 characters joined by `-`.
- **Recovery key input:** any case; O reads as 0, I and L as 1; a terminal's bracketed-paste markers, `ESC [200~` and `ESC [201~`, are removed whole, since their digits are in the alphabet; every other character outside the alphabet is skipped, such as spaces, `-`, line ends and blank lines. Exactly 12 characters of the alphabet must be left.
- **Derivation:** Argon2id [[RFC 9106](https://www.rfc-editor.org/rfc/rfc9106)], version `0x13`, 262144 KiB of memory, 8 passes, 1 lane, a 64-byte tag, no secret or associated data. Password: the key's 12 characters in upper case, ASCII. Salt: `skald recovery` followed by the account ID's 16 bytes. The tag's first 32 bytes are the master backup's AES-256-GCM key; its last 32 are `ikm` of the recovery HPKE key pair, `DeriveKeyPair(ikm)` of DHKEM(X25519, HKDF-SHA256) [[RFC 9180](https://www.rfc-editor.org/rfc/rfc9180), §7.1.3]. One derivation gives both, so the account record's `recovery` tests a guess at the key no cheaper than the backup. Its cost: [Security considerations](#security-considerations).

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
  nonce: bstr .size 12,    ; the AES-256-GCM nonce
  ct: bstr .size 48,       ; the 32-byte master private key, sealed, and the tag
]
```

- The server stores the account record and the master backup.
- **Master backup:** the master private key sealed with AES-256-GCM under the derived key, with a random `nonce`, `aad` the account ID. The recovery key unlocks it; the client checks that the opened key matches `master` and the derived recovery HPKE public key matches `recovery`.
- A new recovery key: a device unlocks the master key with the old one, stores a new backup for the new one and signs an account record with the next `revision`. A client uses the valid record with the highest `revision`. The old recovery key still opens the master key: [Security considerations](#security-considerations).
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
  ? 10 => uint,            ; github: the GitHub user ID whose key made `ssh`
  * int => any,            ; fields of later versions
}

device-kind = &(client: 1, browser: 2, service: 3)  ; 4 and 5 are reserved
```

- Kinds: [Identifiers](README.md#identifiers). Keys are generated on the device and never leave it, except a `service` device's: [Service device](#service-device).
- **Proof check:** a client MUST discard a certificate unless `device` is the ID derived from `signing` ([Identifiers](README.md#identifiers)) and `proof` verifies with `signing`. A discarded certificate counts nowhere and is not shown: the certificate of a device ID, a device's chain and issuers, the signer of an object and the devices a device certified come only from certificates that pass this check, here and in [README.md](README.md). So only the holder of a device's signing key makes a certificate for its ID: no other device can certify an existing device's ID, with other keys or with its own.
- A device signs `proof` for one certificate only, the one issued for it when it joins the account. Certificates of one `device` with the same `body` are one certificate: a client uses a copy whose `signature` passes check 1 of [Device checks](#device-checks). A client that sees two certificates of one `device` with different bodies that pass the proof check treats neither as valid and warns its user.
- **Kept certificates:** a client keeps the certificate of each device it trusted, one that passed checks 1 and 2 of [Device checks](#device-checks). When the server later gives that device with another certificate in its place, the client warns once of a key change, until the server gives the kept one again:
  - one that fails the proof check or check 1 still counts nowhere, so the device is not trusted until then;
  - one of another body that passes the proof check makes neither valid, as two bodies given at once do; the client keeps it too, so neither is valid also once the server gives only one of them.
- A device ID thus names one device everywhere it appears, such as a revocation's `device`, `keep` and `also`, a member's `excluded` and an MLS credential.
- Issuers:
  - the master key: a `client` device, the account's first one or one linked with the recovery key;
  - a `client` device of the same account: any kind.
- A `service` device is certified on the account of the admin who creates it, by the `client` device it is created on.
- `name` and `issued` are for display; nothing depends on them. A client shows a device only by its certificate's `name`, `kind` and `issued`, never by a name the server gives.
- The one exception: while a device of the account waits for linking, each command of a linked `client` device that reaches the server, and each tool call of its MCP server, says once that the server reports, unverified, a device of that `name` and kind waiting, and that `skald link` links it (`getWaitingDevices` in the OpenAPI spec, `link-notice` in [delivery.md](delivery.md#frames)). The name is escaped as other text from the server. Only a prompt: the certificate still goes only through the code its user types ([Link code](#link-code)).
- **SSH signature** (`ssh` and `github`, optional):
  - `ssh`: an SSH signature [[OpenSSH PROTOCOL.sshsig](https://github.com/openssh/openssh-portable/blob/master/PROTOCOL.sshsig)] in its binary form, the base64 of its armored text decoded, version 1, with namespace `skald.so` over the encoded `["skald ssh", account, device, signing]`, the label a text string and the rest byte strings ([Conventions](README.md#conventions)). It is made with one of the SSH keys of the GitHub user `github` names.
  - `github`: GitHub's user ID of that user.
  - A certificate carries both or neither; one with only one of them is malformed.
  - Only a `client` device's certificate carries them; a client ignores them on other kinds.
  - A `client` device that issues its own certificate, as the account's first device or with the recovery key, makes them with an SSH key GitHub lists for the GitHub user of the account's GitHub sign-in method, or one its person names, when it finds one; otherwise its certificate carries neither. A `client` device linked with a link code makes them the same way, once it has a session, and sends them in its request: [Link code](#link-code). One without a session learns its account only from the grant, so it sends neither and says so.
  - A client checks `ssh` against the SSH keys GitHub's API lists publicly for that user, authentication and signing keys, never against keys the Skald server gives:
    - it passes when its key is on the list and it verifies with that key;
    - it is a key change when it is malformed, of another version or namespace, by a key not on the list, or does not verify;
    - it counts as not checked when the client cannot check it, as with GitHub unreachable or a listed key it does not verify. A client verifies Ed25519 and ECDSA (P-256, P-384, P-521) keys, RSA keys of 1024 to 8192 bits with an exponent of 2 to 2^33 − 1, and the security keys `sk-ssh-ed25519@openssh.com` and `sk-ecdsa-sha2-nistp256@openssh.com`.
- An MLS credential names the device: [MLS profile](#mls-profile).

## Linking a device
A `service` device does not link: [Service device](#service-device). A new device generates its keys, derives its ID from its signing key and registers its public keys and `name` with the server, which refuses an ID it has with other keys or `name`, or on another account; the same registration sent again counts as stored. It then gets a certificate: as the account's first device, from a linked device, or with the recovery key. A new `client` device without a session links from a linked device without registering: its certificate registers it ([Device key session](#device-key-session)).

A new `client` device asks its user nothing; it asks the server whether the account has a master key:
- **None:** the device is the account's first. It creates the master key, the recovery key, the account record and the master backup, issues its own certificate with the master key, then deletes the master private key. Of two devices that create one at once, the server keeps the first master key it receives; the other device drops its own, with its recovery key, and links as below.
- **A master key:** the device links from a linked device or with the recovery key.
- A device to link with the recovery key MUST NOT create a master key: told there is none, it says so and stops.

- **From a linked device:** the new device shows a [link code](#link-code), and its user enters it on a `client` device of the account, which issues the certificate. The new device takes a certificate and the master key only through a link code or the recovery key, never from a device the server names.
  - A `browser` is linked only this way: it shows the code, and its user enters it on a linked `client` device.
  - With the certificate, the issuing device sends a link object for the new device: the [roster](#roster) revisions it starts from.
- **With the recovery key**, entered only in the CLI: the new device unlocks the master key and issues its own certificate with it. It MUST reject a [master backup](#account) that does not open with that recovery key, and pins `master` only from one that does. It shows its user the account's devices, revoked or not, and in this order:
  1. revokes with the master key each device its user names as stolen ([Revocation](#revocation));
  2. takes its [anchors](#anchors);
  3. revokes with its own key each device its user names as lost or wiped;
  4. deletes the master private key.

```cddl
link = signed<link-body>         ; label "skald link", by `issuer`

link-body = {
  1 => id,                 ; device: the new device
  2 => id,                 ; issuer
  3 => [* [space: id, revision: bstr .size 32]],  ; anchors: hash of the latest revision the issuer accepted of each of its spaces
  ? 4 => id,               ; personal: the account's personal space the issuer pinned
  * int => any,            ; fields of later versions
}
```

- The new device MUST reject a link object unless `device` is itself and the signature verifies with the certificate of `issuer`, a valid `client` device of its account. Each anchor is a revision it starts from: [Roster](#roster).
- It pins `personal`, if present, as its personal space.
- A device linked with the recovery key has no link object: it starts from [anchors](#anchors).

### Link code
A one-time code the new device shows keys a run of CPace [[draft-irtf-cfrg-cpace-21](https://datatracker.ietf.org/doc/draft-irtf-cfrg-cpace/21/)], a balanced PAKE, between the new device and the issuing device. The server relays the run and never sees the code. The issuing device takes the new device's keys only from the run, and the new device takes its certificate and the master key only from it.

- **Code:** 6 characters of the base32 alphabet [[RFC 4648](https://www.rfc-editor.org/rfc/rfc4648), §6], from a cryptographically secure generator: 30 bits. Shown in two groups of three joined by `-`, e.g. `K7Q-M2X`. Entry ignores case, spaces and `-`, and reads `0`, `1` and `8` as `O`, `I` and `B`.
- **Pairing:** a run of a new device without a session also has a pairing, 6 characters of the same alphabet that the server picks at random, unique among its open runs, and gives only to the new device, before its hello, since CI binds it. The new device shows the run's pairing and its code as one code of 12 characters, the pairing first, in three groups of four joined by `-`, e.g. `PQRS-TUK7-QM2X`. Each run has its own pairing, so the first 6 characters change with each run. Entry reads them as the code's.
- **Attempts at the new device:** a run is one attempt. The new device runs a code at most 3 times, each time with a fresh `session` and scalar, and spends an attempt when it takes a reply. After the third failed run the code ends: the new device tells its user that the code failed 3 times, and that someone else may have tried if its user did not.
- **Expiry:** a code ends 5 minutes after the new device shows it, and once a run with it succeeds. A run still open then fails.
- **Fresh codes:** the new device MUST NOT show a fresh code unless its user asks for one, after a code ends or at any time; asking ends the current code.
- **Attempts at the issuing device:** it MUST reject an entry that is not 6 or 12 characters of the alphabet, read as above, without a run. It runs each entry once. After 3 failed runs in a row it stops, tells its user to get a new code on the new device, and runs no entry until its user confirms the new device shows one. A run that fails on a key change in the request's `ssh` does not count: the code was right.

```cddl
link-message = link-hello / link-reply / link-request / link-grant / link-abort

link-hello = [1, session, share]     ; new device → issuing device
link-reply = [2, session, share]     ; issuing device → new device
link-request = [3, session, sealed]  ; new device → issuing device: link-request-body
link-grant = [4, session, sealed]    ; issuing device → new device: link-grant-body
link-abort = [5, session]            ; either way: the run failed

session = bstr .size 16    ; random, from the new device: the run's CPace sid
share = bstr .size 32      ; CPace Ya or Yb: an encoded ristretto255 element
sealed = bstr              ; AES-128-GCM ciphertext followed by its 16-byte tag

link-request-body = {
  1 => device-kind,        ; kind
  2 => bstr .size 32,      ; signing
  3 => bstr .size 32,      ; hpke
  4 => tstr .size (1..64), ; name
  ? 5 => bstr,             ; ssh: as a certificate's
  ? 6 => uint,             ; github: as a certificate's
  * int => any,            ; fields of later versions
}

link-grant-body = {
  1 => bstr .size 32,         ; master: the master key the issuing device pinned
  2 => bstr .cbor cert-body,  ; cert: the new device's certificate body
  3 => bstr .size 64,         ; signature: by the issuing device, over ["skald cert", cert]
  4 => link,                  ; link: the link object
  * int => any,               ; fields of later versions
}
```

- **Relay:** each message is a CBOR item carried as [Encoding](README.md#encoding) says. The new device sends a hello for each run, and the server relays the run's later messages by `session`. A device's new hello replaces its open run. Paths: [api/openapi.json](../../api/openapi.json).
  - With a session, the new device sends its hello as its account's, naming its device registered there; a `client` device of the account takes the account's latest hello.
  - Without a session, it first opens the run, naming its device, which has no certificate and is on no deleted account. The server answers with the run's pairing and a token for the run's messages, which is no session. The device then sends its hello as the run's first message with the token; until then no account pairs the run. The first account whose session enters the pairing takes the hello, and the run is then its account's. The server refuses the pairing to other accounts. Once the run's device registers on an account, only that account pairs the run, and a run paired with another account ends, as does the run once that account is deleted.
  - A run without a session is not given as waiting until its device registers on an account; it goes on after that.
  - The server bounds runs without a session: a run's token sends at most 8 later messages, each of at most 4096 bytes; an account enters at most 10 pairings the server does not hold within 15 minutes, then none until the oldest is 15 minutes old.
- **CPace:** cipher suite CPACE-RISTR255-SHA512, initiator-responder: the new device is A, the issuing device B. PRS is the code's 6 characters as read above, in uppercase ASCII; CI the encoded `["skald link code", account]`, the account ID as a byte string, or in a run started without a session `["skald link code", pairing]`, the pairing's 6 characters in uppercase ASCII as a byte string, also after its device signs in; sid `session`; ADa and ADb empty. A side MUST fail the run on a share that does not decode or a K that is the identity, and MUST NOT reuse a scalar.
- **Keys:** `HKDF-SHA256(salt = "", IKM = ISK, info = label, L = 16)` [[RFC 5869](https://www.rfc-editor.org/rfc/rfc5869)] over the run's ISK, with the label `skald link request` for the request and `skald link grant` for the grant. Each key MUST seal exactly one plaintext, with AEAD_AES_128_GCM [[RFC 5116](https://www.rfc-editor.org/rfc/rfc5116)], a nonce of 12 zero bytes and empty associated data; a resend repeats the same `sealed`.

The new device:
1. sends a hello and takes the first reply with its `session`;
2. sends the request: its kind, keys and `name`, and, when it has them, `ssh` and `github`, made as [Device certificate](#device-certificate) says, both or neither. Without a session it sends neither: it knows no account to sign for;
3. MUST fail the run unless the grant opens, `cert` holds its account, ID, kind, keys and `name`, and either `ssh` and `github` as its request carried them or neither, `signature` verifies with the certificate of `issuer`, that certificate passes checks 1 and 2 of [Device checks](#device-checks) with `master`, and the link object passes its check. Without a session it cannot list the issuer's certificate, so it keeps the grant as unchecked and checks `signature` and the link object once its device key session gives it one. If that check fails, it drops the grant, ends the session and pins nothing;
4. then signs `proof` and stores the complete certificate on the server: with its session, or without one in the request for its [device key session](#device-key-session). It pins `master` only once step 3 passed.

The issuing device:
1. tells its user to enter only a code shown on their own device, takes the account's latest hello, or for 12 characters the hello of their pairing, and sends a reply;
2. MUST fail the run unless the request opens, and takes the new device's kind, keys, `name`, `ssh` and `github` only from it, never from the server;
3. checks `ssh` against GitHub as [Device certificate](#device-certificate) says, over the new device's account, ID and `signing`. A key change fails the run, and it tells its user so;
4. shows its user that `name` and kind, with the GitHub login of an `ssh` that passed or that `ssh` is not checked, and once its user confirms, issues the certificate for them, with `ssh` and `github` for a `client` device, and sends the grant.

- A side whose run fails sends a `link-abort` and tells its user. A `link-abort` is not authenticated: it only ends the run, and the new device spent the attempt when it took the reply.

### Device key session
A `client` device with a certificate gets a device session of its account with its signing key and no sign-in: a new device without a session once a linked device certified it, or a device that signed out, with no code.

1. The device gets a challenge for its ID: 32 random bytes. The server gives one for any ID.
2. It signs, with the label `skald device session`, `["skald device session", challenge, origin]`, `origin` as in `auth` ([Connecting](delivery.md#connecting)), and sends the signature with its complete certificate.

The server issues the session only if:
- the challenge is one it issued for the device less than a minute ago, and no other request used it;
- the certificate is the one it holds of the device, or it holds none and stores the one sent;
- the device is a `client` device that checks 3 and 4 of [Device checks](#device-checks) do not revoke, against the account's revocations;
- the certificate's account is not deleted;
- the signature verifies with the certificate's `signing` over `origin` set to the origin of the server's public address.

It stores the certificate sent only if the certificate is of the device as a `client` device, issued by a `client` device of its account, passes the proof check and checks 1 and 2 of [Device checks](#device-checks) against that account's master key and the certificates its devices hold, and matches the device's registration if there is one: on that account, with its keys and `name`. A device without a registration is registered by it. With the devices limit, a certificate the account has no place for is refused and gives no session.

- The session is of the certificate's account, has no sign-in method and is listed with its device. The server takes it as any device session. It ends when its device is revoked, when it signs out, when a web session ends it, and when the device gets another session this way: a device has one such session at a time.
- A refused request uses its challenge up. The same request again, a retry after a lost answer, works until the challenge expires, and its session replaces the first one.

### Anchors
Each `client` device keeps on the server an anchors object: the roster revisions a device linked with the recovery key starts from.

```cddl
anchors = signed<anchors-body>   ; label "skald anchors", by `device`

anchors-body = {
  1 => id,                 ; account
  2 => id,                 ; device: a `client` device of the account
  3 => [* [space: id, revision: bstr .size 32]],  ; as in a link object
  ? 4 => id,               ; personal: as in a link object
  * int => any,            ; fields of later versions
}
```

- A `client` device signs a new anchors object when it accepts a revision; the server keeps the latest per device and gives them only to the account's sessions and, with their device's proof, its service tokens.
- A device linked with the recovery key takes the anchors objects after it revokes the stolen devices. It MUST reject one unless `account` is its own and the signature verifies with the certificate of `device`, a `client` device whose certificate passes checks 1 and 2 of [Device checks](#device-checks) and that no master-key revocation marks as stolen. Other revocations do not matter here, so anchors of lost or wiped devices count.
- For each space, it starts from the anchor with the lowest revision number and accepts later revisions as [Roster](#roster) says. Every other anchor of the space MUST be a revision it so accepts; otherwise it warns its user and does not join the space. It does not join a space no valid anchors object names.
- It pins as its personal space the `personal` of the valid anchors objects that have one. When they name different spaces, it pins none and warns its user.

A linked `client` device also starts each space it does not follow that the server lists among its account's spaces, such as one another device of the account created or joined after it was linked. It takes the anchors objects when it connects and on a `space-notice` of such a space ([Frames](delivery.md#frames)), and starts the space as a device linked with the recovery key does, with one difference: an anchors object counts only if its `device` is another `client` device of the account with a valid certificate (checks 1–4 of [Device checks](#device-checks)). A device linked with the recovery key keeps the rule above while it links.

A `service` device's run starts its space's roster from the latest revision named by an anchors object of its account whose `device` is a `client` device with a valid certificate ([Device checks](#device-checks), against the pinned master key) and whose signature verifies with that certificate. It does so when the revisions from its [key secret](#service-device)'s `anchor` up to it pass through every other revision such objects name. Otherwise it starts from `anchor` and warns.
- Named revisions at or below `anchor` do not count.
- It accepts the revisions up to its start without checks 3 and 4 of [Device checks](#device-checks) on their `signer`; later ones with them ([Roster](#roster)).

## Service device
A space admin's `client` device creates the `service` device of a service agent of the space:
1. refuses an `owner/agent` that the space's latest accepted revision has ([Roster](#roster), check 3);
2. generates the device's keys in memory;
3. registers the device on its account, with `name` the address `owner/agent` cut to 64 bytes, and issues its certificate;
4. signs the space's next revision with the device's `service-agent` entry; refused for a revision signed meanwhile, it signs the entry into that one ([Roster](#roster));
5. signs the device's [service grant](README.md#service-grant), which a service token carries.

The device's private keys leave the admin's device only in its key secret, shown once and kept nowhere:

```cddl
service-key = {
  1 => bstr .size 32,      ; signing: the Ed25519 private key, its seed
  2 => bstr .size 32,      ; hpke: the X25519 private key
  3 => id,                 ; account: the admin's
  4 => bstr .size 32,      ; master: the account's master public key
  5 => id,                 ; space
  6 => bstr .size 32,      ; anchor: hash of the revision with the device's entry
  7 => tstr,               ; server: its public address, e.g. "https://api.skald.so"
  * int => any,            ; fields of later versions
}
```

- A `service` device pins `master` as its account's master key ([Device checks](#device-checks)) and starts its roster as [Anchors](#anchors) says.
- One that keeps state is a member of the MLS groups of the channels whose `services` list it ([MLS profile](#mls-profile)). Its token then needs the `receive` permission: it reads its Commits and Welcomes from its mailbox.
- Once such a device has been in a group, a run without its earlier state MUST refuse, so its counters do not restart.
- One that keeps no state is in no group: it sends and receives under `hpke` only ([From a stateless service agent](delivery.md#from-a-stateless-service-agent), [Mailbox](delivery.md#mailbox)).
- In the CLI, the key secret is base64url without padding [[RFC 4648](https://www.rfc-editor.org/rfc/rfc4648), §5]: [Service agents](../service-agents.md).

An admin's `client` device of the account the device is on replaces its key: it creates a new `service` device as above, signs the next revision with the new device in place of the old one in the entry and in `services`, then revokes the old device ([Revocation](#revocation)). The new key secret's `anchor` is that revision.

Any admin's device removes a service agent: the next revision lists its device neither in an entry nor in `services`. The device's account then revokes it.

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
- A client MUST ignore a revocation unless its signature verifies with the key `signer` names: without `signer`, the master key; with `signer` and without `also`, the key of the certificate of `signer`, a `client` or `browser` device of `account` whose certificate passes checks 1 and 2 of [Device checks](#device-checks). Otherwise it counts, whether or not its signer is revoked itself, until a master-key revocation voids it.
- A master-key revocation marks as stolen `device` and every device whose chain passes through it other than through a device in `keep`, and voids every revocation they signed. The device that signs it shows its user the devices the voided revocations named; it lists in `also` those the user keeps revoked, so one object voids and re-revokes.
- Only voiding undoes a revocation; a master-key revocation is never undone.
- The server stores revocations and gives them with the certificates.

## Device checks
A server signature alone never admits a device: a client trusts a device only through its certificate chain to the account's master key.

A certificate is valid when:
1. it passes the [proof check](#device-certificate), and `signature` verifies: with the master key when `issuer` is absent; otherwise with the signing key of the issuer's certificate;
2. its chain, the certificate and those of its issuers in turn, ends at the master key without repeating a device; every certificate of the chain passes check 1, every issuer is a `client` device of the same account, and a certificate without `issuer` is of a `client` device;
3. no counting revocation names the device in `device` or `also`;
4. for each revoked issuer in the chain, every counting revocation of it lists in `keep` the next device down the chain.

The master key is the one the client pinned: its own account's on linking or, for a `service` device, from its [key secret](#service-device); or the `master` of the member in the [roster](#roster). A device is trusted only with a valid certificate; a client MUST NOT send to, accept content from or admit to a channel any other device.

A client MUST warn its user when:
- an account's device set changes: a device is added, revoked or restored by a voided revocation;
- an account record's `recovery` changes, or a record the server gives fails the check of [Account](#account), once per record; one that names the master key of the record the client holds and verifies with it does not count, as an account's records stay so after its `master` changes;
- a KeyPackage the server hands out fails its checks ([MLS profile](#mls-profile));
- a member's `master` changes in the roster; for an account it [verified](#account-fingerprint), it also says the account is no longer verified;
- a member handle names different accounts in two consecutive accepted roster revisions.
- an `ssh` signature it checks is a key change ([Device certificate](#device-certificate));
- its space's [domain](#domain) lists a member only with other fingerprints, or does not list an admin;
- the server gives a device it trusted with another certificate in place of the one it kept ([Kept certificates](#device-certificate)), or a device with certificates of two bodies ([Device certificate](#device-certificate));
- the latest revisions it accepted in its spaces give one account different master keys, or its own account one other than the one it pinned; each space's devices are still checked against that space's revisions;
- two valid devices of an account have the same `name`.

All but the first and the last are **key changes**. A client keeps each in its local state until its person sees it:
- Until then, each command, as the person or as an agent, and each tool call of its MCP server says on one line how many wait and which command shows them; with JSON output, as a field. The line names no space, account or device. A [service device](#service-device)'s runs do not say it: no person sees them at a terminal.
- One an `ssh` signature or a domain shows counts only in runs with the GitHub API or resolver that showed it, as their results do.
- The command shows them only to the person at a terminal, never as an agent or through a pipe, and counts them seen once the person confirms.

## Account fingerprint
Two people check that their devices see the same master key for each other's accounts by comparing fingerprints.

- **Fingerprint:** SHA-256 over the encoded `["skald fingerprint", account, master]`, the account ID and the master public key as byte strings. Each of its eight 4-byte words, as a big-endian integer modulo 100000, gives five decimal digits with leading zeros; the groups are joined by one space, 40 digits in all, about 132 bits. Vector: [test-vectors.md](test-vectors.md#account-fingerprint).
- A device shows its own account's fingerprint from the master key it pinned, and a member's from the `master` the roster gives.
- Two people compare the fingerprints their devices show for their own accounts over a channel they trust, such as in person or on a call; a fingerprint the server or a message gives proves nothing.
- A device marks an account verified, for that master key, only on its person's yes to whether the fingerprint matches; an agent never marks one. A verification stays on the device that made it.
- A verified account whose `master` changes in an accepted revision is no longer verified, and the device warns ([Device checks](#device-checks)).

### Domain
An organization lists its accounts' fingerprints in its domain's DNS, and member devices of a space whose policy names the domain check its members against them.

- **Records:** TXT records at `_skald.<domain>`, one per account, its strings joined: the account ID as text, one space and the account's fingerprint as above, e.g. `01a1c4c4-94cb-7a3e-9c41-5d2e8f6b7a10 01234 56789 …`, with no other whitespace. A TXT record of another form lists nothing.
- **Domain name:** two labels or more joined by `.`, each of 1 to 63 lowercase ASCII letters, digits and `-`, none starting or ending with `-`; at most 246 characters, so `_skald.<domain>` fits in 253; no trailing dot. A device checks nothing against a `domain` of another form.
- **Lookup:** with DNS over HTTPS [[RFC 8484](https://www.rfc-editor.org/rfc/rfc8484)] at the resolver the device's setting names. The device keeps the records for their TTL, or for an answer without records the negative TTL [[RFC 2308](https://www.rfc-editor.org/rfc/rfc2308)], and looks them up again after it. A resolver that fails leaves the members not checked.
- A member the records list with the fingerprint the device shows for it is listed by the domain. One they list only with other fingerprints, and an admin they do not list, are key changes, and the device warns ([Device checks](#device-checks)). Other members they do not list show nothing.
- The check comes on top of the roster, never instead of it: it changes no master key, role or device check.
- DNSSEC is not checked: [threat model](../threat-model.md#trade-offs).

## Trust levels
A recipient device sets the level of each message it accepts from the sending device, the one its scheme authenticates ([Envelope](README.md#envelope)), and of each task it claims from its `sender` ([Claim checks](queues.md#claim-checks)). The level sets how a message's text reaches the agent ([Device to agent](delivery.md#device-to-agent)) and a task's rights ([Running a task](queues.md#running-a-task)).

| Level | The sending device |
|---|---|
| `own` | A `client` or `browser` device of the recipient's account |
| `peer` | A `client` or `browser` device of another account the recipient's space roster lists as a member |
| `service` | A `service` device |

- The level comes from the sending device's valid certificate, its `account` and `kind`, and the roster of the recipient's space; never from the content, and no setting changes it.
- `own` is above `peer`, which is above `service`.

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
  * int => any,
}

channel-member = [
  account: id,
  history: bool,           ; whether it reads the history from before it joined
]
```

- A space's creator signs revision 1 and is its first admin.
- An admin's device in a revision is a `client` or `browser` device with a valid certificate ([Device checks](#device-checks)) of an account that is an admin in it, unless that member's `excluded` in it lists the device.
- A revision's hash is SHA-256 over its encoded `roster`.
- The server keeps every revision. It stores revision n+1 only if it has revision n, so of two revisions signed at once it keeps the first; it refuses the other, and its signer applies its change to the new revision and signs again.
- It sends a [`space-notice`](delivery.md#frames) to the space's devices when it stores a revision.
- A `service` device sends only as `space/owner/agent` from its entry ([Envelope](README.md#envelope)).
- A revision that lists a `service` device revoked later stays acceptable to a device that walks past it, a stateless run included (check 4). A revoked device is trusted for nothing else ([Device checks](#device-checks)).
- A revision an admin's device signs drops the entries and `services` of revoked `service` devices and of the accounts it removes. The server refuses a new revision that lists, in an entry or in `services`, a `service` device that is revoked or not on a member's account.

A client starts from an anchor: revision 1 of a space it created, the revision its [invite](#invite) link names, or one from its [link object](#linking-a-device) or the [anchors](#anchors) objects of its account's devices. A `service` device starts as [Anchors](#anchors) says. A client MUST NOT accept revision n+1 before it has accepted revision n, nor unless:
1. its signature verifies with the certificate of `signer`, an admin's device in revision n;
2. `space` and `name` are those of revision n, `revision` is n+1, and `prev` is the hash of revision n;
3. it has at least one admin; no list repeats an account, device, channel or invite; `name`, handles, owners and agents are valid names ([Agent address](README.md#agent-address)); no two members share a handle, and no owner is a handle; service agents may share an owner, but not an owner and agent;
4. each `service-agent` device has a certificate of kind `service` on a member's account that passes checks 1 and 2 of [Device checks](#device-checks), revoked or not; each `channel-member` is a member; `policy` passes [its checks](#policy).

A revision that fails, and every revision built on it, has no effect: the client keeps the last revision it accepted, warns its user and keeps working with that one. Two valid revisions with the same number mean the server forked the roster: the client keeps the one it accepted and warns its user.

## Policy
A space's policy travels in its roster, so an admin's device signs every change.

```cddl
policy = {
  1 => [peer: mode, service: mode],  ; ceilings: per trust level
  2 => uint .ge 86400,     ; history: the history key's longest age, in seconds
  ? 3 => [own: wake, peer: wake, service: wake],  ; wakes: per trust level
  ? 4 => tstr,             ; domain: the organization's domain name
  * int => any,            ; fields of later versions
}

mode = &(read-only: 1, full: 2) / uint   ; other values: modes of later versions

wake = wake-now / wake-batch / wake-pull / uint / [uint, * any]   ; other values: wake modes of later versions
wake-now = 1
wake-batch = [2, window: uint .ge 1]   ; window: seconds, at least, between two wakes
wake-pull = 3
```

- `ceilings`: the most rights a task from a `peer` or `service` sender gets without the approval of the claiming device's user, who can only lower them ([Running a task](queues.md#running-a-task)). An `own` task has no ceiling.
- `mode`: `read-only` reads only the agent's folder and changes nothing, with no network; `full` has no limits. This version defines only these two; a device takes a mode it does not know as `read-only`.
- `history`: [History key](#history-key).
- `wakes`: the most waking delivery mode for messages of each sender trust level, in every channel of the space ([Device to agent](delivery.md#device-to-agent)). Absent, there is no ceiling. A device takes a wake mode it does not know as `pull`.
- `domain`: [Domain](#domain).
- A new space starts with `[read-only, read-only]`, 30 days, no `wakes` and no `domain`.

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
- **Link:** `<web>/invite/<invite>#<fragment>`; `<web>` is the server's web address, an origin such as `https://skald.example`, which the server names in its answers to `GET /session` and `GET /devices`; the fragment is base64url without padding, 86 characters, of the secret followed by the hash of the roster revision the admin's device last accepted. A link with a query, another path or another fragment is refused. Browsers do not send a fragment to the server.
- The server stores the invite and the acceptance. An invite admits one account: the server keeps its first acceptance and refuses others. `expires` is at most 30 days after the server stores the invite.
- An invite ends when it expires, an admin revokes it from the web app or a stored revision admits a member with it. The server deletes it with its acceptance. A revision that newly lists a member with an invite is refused unless the invite is active and has that member's acceptance. Revocation leaves earlier revisions and their members in place.

While sign-up is closed, the invite also lets one unlisted person create an account. Before opening the device flow's verification page, the device sends the invite ID and an Ed25519 signature by the invite key over the CBOR array `["skald invite sign-up", invite, device-code]`. The signed `invite` is its 16 UUID bytes. The signed `device-code` is the UTF-8 bytes of the code from `POST /auth/device`. Both are CBOR byte strings.

The server creates the account only if the attached proof verifies against the key of an invite the server still holds that is live and unused. In the same transaction it reserves the invite for that account. Only that account's sessions get the invite and can accept it. A sign-in already allowed by the server's sign-up policy leaves the invite available.

The newcomer's device takes a link only from its person. To accept it, the device MUST, from the link:
1. take the roster revision with the hash in the fragment, check its hash and accept it as its anchor, then accept later revisions as [Roster](#roster) says;
2. check the invite's signature with `signer`, an [admin's device](#roster) per those revisions, and that `key` is the one derived from the secret;
3. only then prove it holds the secret: sign an acceptance with the invite key, for its own account, master key and handle.

It then follows the space from the anchor. When the invite ends before a revision lists its account, it stops following the space.

An admin's device, `client` or `browser`, MUST NOT admit a newcomer unless it:
1. checks the invite: its signature with the certificate of `signer`, an [admin's device](#roster) in the latest revision, `expires` not passed, no member of a revision it accepted with its `invite`;
2. checks the acceptance's signature with the invite's `key`;
3. checks that the account is not a member and the handle is free;
4. checks each of the newcomer's devices: a valid certificate chained to `master` from the acceptance, not to one the server gives.

It then signs the next revision with the member, its `invite` and the invite's role, and the newcomer in the default channel with `history` true and in the invite's other channels that the revision has, with their `history`. The default channel is the latest revision's first one: it may have changed since the invite. An acceptance a revision already admitted is done. The newcomer's devices join the channels: [Group changes](#group-changes).

## Admin actions from a browser
A linked browser is not an MLS member, but an admin's browser signs revisions like a `client` device: invites, accepting newcomers, roles, removing members, policy.

- The browser stores the revision on the server; the server sends a `space-notice`.
- Member devices bring the channels' MLS groups in line with the accepted revision, as for any revision: [Group changes](#group-changes). Nothing waits for the admin's `client` device.

## Channels
A channel is an MLS group [[RFC 9420](https://www.rfc-editor.org/rfc/rfc9420)] whose `group_id` is the channel ID. The server is its Delivery Service: it stores KeyPackages and GroupInfo, and orders Commits.

### MLS profile
- Ciphersuite `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519` (`0x0001`).
- Credential: `basic`, its `identity` the 16-byte device ID. A leaf's `signature_key` MUST equal the `signing` key of the device's certificate, so `identity` is the ID derived from `signature_key`.
- Members: `client` devices and `service` devices that keep state. A `browser` and a stateless `service` device are never members.
- Application messages are PrivateMessages. Proposals and Commits are PublicMessages, so the server knows each epoch's member devices.
- Forward secrecy: a device deletes a message's key once it decrypts the message. It keeps the secrets of the 3 epochs before its current one, for messages stored after a Commit their sender had not processed yet ([Order](delivery.md#order)), and deletes older ones. Within an epoch it keeps the keys of skipped messages up to 1024 generations ahead, until it deletes that epoch's secrets.
- Each Commit's sender uploads the GroupInfo of the new epoch, with the `ratchet_tree` and `external_pub` extensions, over HTTPS. Any member device of the current epoch uploads one if the server has none.
- The server keeps an upload of the current epoch from a session of an account with a member device that the GroupInfo lists, or from a service token that lists the channel, whose `service` device the GroupInfo lists, with both extensions, whose signature verifies and whose GroupContext is that of the group it follows ([Commit ordering](#commit-ordering)). It gives the latest only while it is of the current epoch, so after a Commit it has none until one of the new epoch is uploaded, and only to sessions of accounts the latest revision lists as the channel's members and to service tokens that list the channel; without one, it says whether the channel has a group yet.
- A `service` device sends its Commits on a connection of its service token, which needs the token's `send` permission and the channel among its channels ([Send](delivery.md#send)); its token alone uploads the GroupInfo.

### Admitted devices
A device is admitted to a channel when its certificate is valid, its account is not one the server reports deleted, and it is one of:
- a `client` device of an account in the channel's `channel-member` entries in the latest revision the client accepted, unless that member's `excluded` lists it;
- a device in the channel's `services`.

The server reports an account deleted in the space's device list ([api/openapi.json](../../api/openapi.json)). A member device fetches it with the revisions and on a `space-notice` ([Space notice](delivery.md#frames)); once its account is reported deleted, a device is no longer admitted, so member devices commit its Remove, which starts an epoch with a new history key, before they send in the channel, and add it nowhere.

A member device:
- MUST NOT commit the Add of a device that is not admitted;
- before it treats a device a Commit adds as not admitted, MUST fetch the space's latest revisions and accept them as [Roster](#roster) says;
- after it processes a Commit or accepts a revision, MUST commit the Remove of each member that is not admitted, and MUST NOT send application messages in the channel until no such member is left;
- MUST warn its user when a Commit by another device added a device that is not admitted.

### Creating a channel
1. An admin's device gets a channel ID from the server and signs a revision with the channel.
2. The first admitted device to accept the revision creates the channel's group at epoch 0, adds every admitted device it has a KeyPackage for and commits: epoch 1. A `service` device creates none, as its token gets no other device's KeyPackages: it waits for a member's group ([KeyPackages](#keypackages)).
3. The server keeps the first Commit of epoch 0 it receives for the channel and refuses later ones with `epoch-mismatch` ([Send](delivery.md#send)). A device whose group is refused drops it and joins the kept one.
4. Once the server stores its Commit, the creator uploads the GroupInfo of epoch 1, then sends the Welcome ([Commit ordering](#commit-ordering)).

### KeyPackages
- Every device that can be a member keeps unused KeyPackages on the server: when fewer than 10 remain, it uploads enough to have 20. Their `lifetime` is at most 60 days; a device replaces a KeyPackage 7 days before it expires.
- Each device also keeps one last-resort KeyPackage, which the server hands out only when no other remains and does not delete. The device replaces it once it was used, and as the others.
- The server hands out each other KeyPackage once, then deletes it, and deletes expired ones. It hands them only to devices of accounts that share a space with the KeyPackage's device.
- A `service` device uploads its own KeyPackages with its service token alone. A service token gets no other device's KeyPackages, and uploads or counts none of them.
- A device MUST NOT add a KeyPackage unless its MLS signature verifies, its `lifetime` holds and spans at most 60 days, its ciphersuite is `0x0001`, and its `identity` and `signature_key` match an admitted device's valid certificate.

### Commit ordering
- The server follows each channel's group from outside it, without group secrets. It keeps the channel's first Commit of epoch 0 without checking it, as it has no state of epoch 0. It takes the GroupInfo of epoch 1 only from a session of the account of that Commit's sending device, listing that device, and follows the group from it. Until then it refuses the channel's other `mls` envelopes with `not-a-member`.
- It keeps a channel's current epoch from the Commits it accepts. It accepts a later Commit only if:
  1. its `epoch` is the channel's current epoch; otherwise it refuses it with `epoch-mismatch`;
  2. its sender is a member of that epoch whose `identity` is the device sending it, or, for an external Commit, the joining device is the one sending it; otherwise `not-a-member`.
- A Commit it cannot process, such as one whose signature does not verify, and a proposal get `not-a-member`.
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
- A device joins by itself only while no device has added it: once its mailbox catch-up is done, with no Welcome for the channel in it, and when the latest GroupInfo does not list it. Without a GroupInfo of the current epoch it waits. This is the table's "Otherwise": a device that should add it but has not yet, or got no KeyPackage for it, does not block it.
- A device's Adds skip devices already in the group, such as one that joined by itself meanwhile.
- A device that gets a Welcome checks the signer and every member it lists the same way.
- A device refuses a Welcome whose signer is not admitted, and a GroupInfo whose signer or a member it lists is not; it stays outside the group and warns its user. It joins by itself once the latest GroupInfo passes, also if that lists it. A device that joins from a Welcome listing a member that is not admitted warns its user and removes that member.
- A device also joins by itself from a GroupInfo whose signer and members that are not admitted are all revoked devices or devices of accounts the server reports deleted, of accounts in the channel's `channel-member` entries, their certificates passing checks 1 and 2 of [Device checks](#device-checks). It then commits their Remove before it sends, as [Admitted devices](#admitted-devices) says. Any other device that is not admitted still makes it refuse the GroupInfo. So a device linked with the recovery key gets into a channel whose other devices it revoked, and a member's device gets into one whose other members were deleted.

### Rejoining
- **After missing Commits:** a device that cannot process a channel's next handshake message, or that a Commit removed while it is admitted, rejoins with an external Commit that also removes its old leaf. It MUST warn its user if a Commit removed it while it was admitted. A Commit of an epoch the device has passed is a replay: it is dropped.
- **After a wipe:** the device lost its keys and counters, so it is a new device: it is linked again ([Linking a device](#linking-a-device)) and joins as a member's new device. The user revokes the old one.
- **Offline past the mailbox retention:** handshake messages do not expire by time, so the device processes the Commits it missed in order. The application messages that expired are reported by [gap detection](delivery.md#gap-detection); the device reads them from the [history](#history).

## History
The server keeps a copy of each channel message under the channel's history key, so new devices, linked browsers and the recovery key read past messages.

### History key
- The history key of epoch e of a channel's group is `MLS-Exporter("skald history", "", 16)` in that epoch [[RFC 9420](https://www.rfc-editor.org/rfc/rfc9420), §8.5]: every member device of the epoch derives it, so it needs no distribution of its own.
- The key rotates with every epoch:
  - **revocation:** a Remove of a revoked device starts an epoch; when a browser of a channel member's account is revoked, a member device commits an empty Commit before it next sends in the channel;
  - **schedule:** when the current epoch is older than the policy's `history`, the first member device to notice commits an empty Commit.
- A device counts an epoch's age by its own clock from when it entered the epoch: by creating the group, a Welcome, a Commit, its own or another's, or a join or rejoin. Never from the server's `stored` time.
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
- Each clean epoch's key is wrapped to the recovery key of each account among the channel's members and to each browser of those accounts with a valid certificate, unless that member's `excluded` in the latest revision the wrapping device accepted lists it. None goes to the recovery key or browsers of an account the server reports deleted. `client` devices take keys from MLS and the key chain and need no wrap.
- A browser linked later gets wraps of the current epochs from member devices that find them missing.

A reader MUST drop a wrap unless `ct` opens with its HPKE key and the signature verifies with the certificate of `signer`, revoked or not. That certificate MUST pass checks 1 and 2 of [Device checks](#device-checks) with the master key of an account that an accepted revision lists as a member of the space.

A browser writes entries only in the epoch of the channel's latest GroupInfo, under the key of a wrap of that epoch. It MUST NOT write any unless that GroupInfo is of the channel and the wrap's epoch, its signature verifies with the `signature_key` of a member leaf, it lists `signer` as a member, and every member it lists is admitted.

### Uploads
- The sender of each Commit uploads the new epoch's link and wraps. Any member device of an epoch uploads those it finds missing and can make.
- Devices can judge an epoch differently, for example when only one has seen a revocation, so each uploads the link its own judgment gives.
- A device never uploads again a link the server has had, having taken it from the device or listed it to it, nor one the server refused: a link that retention deleted stays deleted.
- A device uploads each link, wrap, entry and [queue key object](queues.md#queue-key-object) in a [`history`](delivery.md#frames) frame on its connection, which proves its key ([Connecting](delivery.md#connecting)). A session or a service token alone uploads none. A member `service` device uploads on a connection of its service token, as any member device does, in the token's channels only.
- The server keeps the first link per channel, epoch and `prev` and the first wrap per channel, epoch and recipient, each only from a member device of its epoch. A later one changes nothing.
- It answers `history-stored` when it keeps the item or keeps one in its place already, and otherwise `history-refused`, keeping nothing, with one of these problems. A client treats a problem it does not know as a refusal too.

| Problem | The item |
|---|---|
| `malformed` | Does not decode as its kind, names another channel than its upload, or is a link whose `prev` is not below its epoch |
| `not-a-member` | Comes from a device that is no member of the epoch, or a revoked one, or names no channel of the space whose group the server follows, or, on a service token's connection, a channel the token does not list; a wrap, to the recovery key or a browser of a deleted account |
| `epoch-mismatch` | Is of an epoch the channel has not had; a link or wrap, of an epoch before the channel's cutoff; an entry, of an epoch other than the channel's current or previous one |

- With a history retention, the server deletes each entry stored longer ago than it. A channel's cutoff is the latest epoch of an entry the server has deleted, and never moves back. The server deletes the links and wraps of epochs before the cutoff; those of the cutoff and later epochs stay, whatever their age. Without a retention, it deletes none.
- The server gives a channel's links, wraps and entries, as the bytes it got, to sessions of accounts the latest revision lists as the channel's members, and to service tokens that list the channel: [api/openapi.json](../../api/openapi.json).

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
- The server keeps one entry per sending device and `msg_id`, which the upload names, as the server cannot read it. It takes an entry only for the channel's current or previous epoch, and only from a member device of the current epoch or a browser with a wrap of the current epoch.

A reader MUST drop an entry unless:
1. it decrypts with the epoch's key;
2. `channel` matches the entry's, and `signature` verifies with the certificate of `sender`, revoked or not: a device of an account that an accepted revision lists among the channel's members, or one of its `services`, whose certificate passes checks 1 and 2 of [Device checks](#device-checks) with the `master` that revision gives for its account. A device that starts from an anchor after revision 1 also counts each earlier revision whose hash is the `prev` of one it counts, back to revision 1, as accepted, without a warning: it checks the entries of members removed before it was linked against those;
3. its content passes checks 2 and 4 of [Envelope](README.md#envelope), with `sender` as the sending device.

### Access
- A member's device reads the history through its keys: from its first epoch back along the key chain.
- A channel member whose `history` is false in the latest revision reads only epochs from its join: the first epoch with a device of its account after the revision that last added the account to the channel. The server gives its sessions only the links whose `prev` is that epoch or a later one, and no wrap or entry of an earlier epoch; before the join, none. A Commit that removes the account's devices does not move its join: its devices that join later read from that epoch too.
- The recovery key reads every epoch wrapped to it: a device linked with it reads the account's history before it rejoins the channels.

## Security considerations
- A device ID keeps 122 bits of a hash of the signing key. Taking an existing device's ID means finding a key for it, about 2^122 hashes. Two keys with one ID take about 2^61; both are then their finder's, and a client that sees both certificates treats neither as valid.
- The server gives the certificates, revocations and revisions. It cannot forge them, but it can withhold them: a hidden revocation keeps a device trusted for those who do not see it, and a hidden revision keeps clients on an older roster.
- A stolen device can revoke the account's other devices and certify new ones, but not under an existing device's ID, and its revocations count even after another device revokes it. The recovery key restores the account: a device linked with it revokes the stolen one with the master key. That one object voids every revocation the stolen device and the devices it certified signed, and lists in `also` the devices that stay revoked, so the server cannot restore them by withholding a second object.
- The master key cannot be rotated. A leaked recovery key, current or old, with any copy of a master backup it opens, gives away the master key for good: its holder can certify and revoke devices and sign an account record with its own `recovery`, so member devices wrap history keys to it. A new recovery key does not lock it out: the holder signs a record with a higher `revision` again. Clients warn when `recovery` changes.
- Only a holder of the recovery key seals a master backup that opens with it, so the server cannot give a device linked with the recovery key another master key. It can withhold the backup: the device then cannot link with the recovery key.
- A recovery key holds 60 bits. Whoever holds the server's database tests a guess at an account's key offline, with its record or backup, at the cost of one [derivation](#account): about a second of a laptop core, with 256 MiB of memory. The salt makes each guess good for one account. Finding a key takes 2^59 derivations on average, about 18 billion laptop-years; hardware a million times faster takes about 18,000 years.
- A link code holds 30 bits, and a CPace run gives a server or network attacker one online guess at it and no offline test. Each code takes an action of the new device's user, and the new device runs it at most 3 times, whoever answers: an attacker that answers its hellos gives it a certificate, a master key and a link object of its choice with a chance of at most 3 in 2^30 per user action, about 3 in a billion.
- The new device does not count the issuing device's runs, which the issuing device counts itself: an attacker that answers its entries with hellos of its own gets at most 3 more guesses at each issuing device before it stops. Each issuing device counts only its own runs, so a code allows at most 3 at the new device plus 3 at each issuing device its user enters it on, each guess with a chance of 1 in 2^30. A success there still needs the issuing device's user to confirm the `name` and kind the attacker chose.
- A run without a session is no account's until a session enters its pairing. A session of any account that knows a pairing can pair the run first and answer the new device's hellos, as the server can: the new device counts its runs whoever answers, so the chance stays at most 3 in 2^30 per action of its user. It spends one of those runs, and the person's issuing device then finds no run. A pairing holds 30 bits, which the server picks unique among the open runs and gives only to the run's device, and an account enters at most 10 pairings the server does not hold per 15 minutes.
- In a run started without a session, CI binds the pairing: an issuing device completes the run only with a new device that ran it with the pairing its user entered.
- A device's signing key with its certificate gives a session of its account with no sign-in. A stolen device thus gets a session again after its sessions end, until it is revoked; its revocation ends that session.
- The issuing device certifies the device whose code its user enters, with the `name` and kind that device chose. A user persuaded to enter someone else's code, such as one a sign-in thief sends, links the thief's device; the issuing device's reminder to enter only a code shown on their own device is the only defense.
- A device learns whether its account has a master key only from the server. Whoever signs in before the user's first device, such as the IdP, a thief of the user's sign-in or the server, creates the master key and holds the account's root of trust for good: the master key cannot be rotated. The user's first device then waits for linking, and the holder links none of the user's devices: that takes a code the user enters on the holder's device. A server that holds such a key can instead report none: the user's first device then creates a second master key, as below, with no warning.
- Members pin an account's master key from the roster, which takes it from the account's device that created the space or accepted the invite. So members of a space the holder created or joined trust the holder's devices as the account's, and get no key change warning: no other master key came first. The `ssh` signature is optional, so its absence shows nothing.
- A server that reports no master key for an existing account makes a new device create a second master key, and the account splits into two sets of devices that do not trust each other: devices and members that pinned the first master key reject certificates chained to the second, and the device rejects those chained to the first. The server gains no signing power: the device deletes the second master private key, and only its user sees the second recovery key. No device set change warns of the device: the account's other devices do not count it as theirs. Only key verification, planned ([roadmap.md, "MVP"](../dev/roadmap.md#mvp)), shows the second master key.
- A device linked with the recovery key trusts the revisions anchored by its account's `client` devices other than those its user names as stolen. A stolen device the user does not name counts as honest: in a space no other device anchored, it can make the device start from a revision the server forged. Where other devices anchored the space, its anchor off their chain only stops the device from joining. The server can withhold anchors objects: the device then starts from older revisions, or joins no space.
- A linked `client` device trusts the revisions anchored by its account's other `client` devices with a valid certificate. Until a stolen one is revoked, in a space no other device anchored, it can make the device start from a revision the server forged. The server can withhold anchors objects: the device then stays out of a space another device of its account created or joined.
- A newcomer trusts the roster revision named in the invite link; the link's integrity rests on the channel it was sent over.
- Whoever names an invite's ID reads its space's roster until the invite has an acceptance.
- A channel member whose `history` is false holds the key of every epoch from its join on, and each opens that epoch's link: the join epoch's link, or a later one whose uploader judged the epochs between unclean, gives a key from before the join. Only the server withholding those links keeps it from earlier history.
- The server holds GroupInfo, so it can join a channel by an external Commit from its own device. Members remove the device, and its epoch is not clean: no link opens an earlier key with that epoch's key, and no member writes under it.
- A browser writes only under the key of a wrap whose signer the epoch's GroupInfo lists, with every member admitted, so the server cannot choose the key a browser writes under.
- A reader takes a wrap signed by any device of a member account, revoked or not. A member device can upload a wrong link or wrap first. That breaks reading the history, not its authenticity: every entry is signed.
- The server can report any account deleted. Member devices then remove its devices from the channels, and they rejoin by themselves once the report stops: a denial of service, as withholding is, that gives no one access.
- A deleted account's devices read a channel until a member device that learned of the deletion commits their Remove; until then the server delivers them nothing.
- A revoked device reads a channel until a member device that holds its revocation commits its Remove, or the empty Commit for a browser; the server delays this by withholding the revocation from every member device. A member device that did not process that Commit, such as one that joined later, learns the revocation only from the server: if the server withholds it and lets the device rejoin, that member's links open to it keys of epochs after the Remove.
- A device that joins a group holding revoked devices or devices of deleted accounts ([Group changes](#group-changes)) shares its join epoch with them. That epoch is not clean, and the joiner sends nothing in it.
- An entry's `timestamp` is its sender's claim. The server, not the reader, keeps a removed member from adding entries, and a member from adding them to epochs before the previous one.
- A member can make a channel unusable with a Commit the others cannot process; the channel is then replaced by a new one.
- A `service` device's run is as current as its account's devices: it starts from the revisions they anchored. A stolen `client` device of the account with the server can make it start from a revision that a revoked admin device signed after its revocation, but only while the stolen device is not revoked: a revoked device's anchors objects do not count. A revision that none of the account's devices accepted before its signer's revocation fails for the run as it does for them.
- A stateless `service` device's run keeps nothing, so the server can withhold later revisions and anchors objects and give an older GroupInfo, back to the key secret's `anchor`. The run then seals to devices of a member removed since: with the server's help, a removed member keeps reading the agent's messages.
- Everything the server sees: [threat model, "The server sees"](../threat-model.md#the-server-sees).
