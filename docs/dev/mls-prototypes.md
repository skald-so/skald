# MLS prototypes

Prototypes on OpenMLS and mls-rs for choosing the crypto core's MLS library (Story 3 in [task.md](../../task.md)).

- Code: [`prototypes/mls/`](../../prototypes/mls), a cargo workspace apart from the product crates. `mls-scenario` holds the library-neutral suite; `mls-proto-openmls` and `mls-proto-mls-rs` implement its `Device` on each library.
- Versions: OpenMLS 0.9.2 with `openmls_rust_crypto` 0.6.0; mls-rs 0.57.0 with the `last_resort_key_package_ext` feature and `mls-rs-crypto-rustcrypto` 0.23.0.
- Profile: [membership.md](../protocol/membership.md#mls-profile), "MLS profile". The device's Ed25519 signing key is generated outside the library.
- Every step of a device loads its saved state and saves it after the step, as one-shot mode does ([architecture.md](architecture.md#components)). A failed step leaves the saved state as it was.
- Run: `cargo test --manifest-path prototypes/mls/Cargo.toml`; `script/test` runs it too.

## Scenarios
| Scenario | Checks |
|---|---|
| `creating_a_channel` | [Creating a channel](../protocol/membership.md#creating-a-channel) on 3 devices: KeyPackages with ciphersuite `0x0001`, a `basic` credential with the device ID and the device's key as `signature_key`; the creator's group at epoch 0; its Add Commit a PublicMessage of epoch 0, pending until merged; the Welcome's ciphersuite; all at epoch 1 with the same members in leaf order |
| `member_adds_a_device` | A member other than the creator adds a device; the other members process the Commit, a PublicMessage, and get its sender and, before the merge, the added device's leaf; all at epoch 2 |
| `members_exchange_messages` | On 4 devices, each sends two messages: PrivateMessages without the plaintext; every other member decrypts each and gets the sender's device ID from its credential, derived from its `signature_key` |
| `replayed_message_is_rejected` | A member rejects an application message it already decrypted and still decrypts the next one |
| `removed_device_reads_nothing_later` | A member commits the Remove of a revoked device: a PublicMessage Commit with a Remove, an UpdatePath and the device's revocation in its `authenticated_data`; the others at epoch 2 without it. The removed device processes its Remove and sees itself removed. Its saved state from before the Remove and from after it processes the Remove fails on every later message: application messages of epoch 2, an empty Commit, a message of epoch 3. No member reads what it sends from its copy |
| `revocations_are_read_before_merging` | A member reads a Remove Commit before it merges it: its sender, the removed leaf and the revocations in its `authenticated_data`, which count. A failed check leaves the device in its epoch with its members; the same Commit merges once the check passes |
| `empty_commit_starts_a_new_epoch` | An empty Commit, after a browser's revocation with the revocation in its `authenticated_data`, then on schedule without one: a PublicMessage Commit with an UpdatePath and no proposals, no Welcome; every member reads it and enters the next epoch with the same members |
| `history_key_rotates_with_each_epoch` | Every member of an epoch derives the same history key, `MLS-Exporter("skald history", "", 16)`; messages and reloads keep it. A Remove and an empty Commit each change it. The removed device has epoch 1's key from its copy, processes its Remove, fails on the empty Commit and derives neither later key |
| `device_keeps_key_packages` | [KeyPackages](../protocol/membership.md#keypackages): with none unused, a device makes 20 and a last-resort one, each with the profile's ciphersuite and credential, its own keys and a lifetime of 60 days from an hour before it was made; only the last-resort one has the `last_resort` extension. With 9 left, it makes 11. A KeyPackage of each batch adds it to a group |
| `key_packages_that_fail_a_check_are_refused` | An Add is refused, alone and next to another device's valid KeyPackage, for a KeyPackage with a bad signature, signatures by another key than its `signature_key`, an expired or not yet valid lifetime, ciphersuite `0x0003`, an admitted device's `identity` with another `signature_key`, or an `identity` without a certificate. The same device then adds valid ones |
| `devices_that_are_not_admitted_are_refused` | An Add is refused for a valid KeyPackage of a device without a certificate, a `client` of an account that is not a member, a `browser`, an excluded device and a revoked one; a `service` device in `services` is added |
| `last_resort_key_package_adds_to_two_groups` | One last-resort KeyPackage adds its device to two groups, which it reads and sends in; a regular one joins it to one group, and its second Welcome fails |
| `device_added_without_admission_is_removed` | A member with another view of the roster adds a device without a certificate. Another member sees the device's leaf before it merges the Commit and finds it not admitted ([Admitted devices](../protocol/membership.md#admitted-devices)), then commits its Remove; every device, the removed one too, processes it. No member has anyone left to remove; the removed device reads no later message |

The failing KeyPackages are the device's own, changed one field at a time and signed again with its key by `mls-scenario`; signed again unchanged, a KeyPackage is byte for byte the library's.

Certificates and revocations are encoded as [Device certificate](../protocol/membership.md#device-certificate) and [Revocation](../protocol/membership.md#revocation) say, in deterministic CBOR. A test fixture, `Roster`, stands in for the certificates, revocations and pinned master keys a device gets and the roster revision it accepted:
- a certificate counts only if it passes the proof check, and one device's certificates with two bodies make it invalid;
- a certificate is valid by checks 1 to 4 of [Device checks](../protocol/membership.md#device-checks); a revocation counts as "Revocation" says, but no scenario has a stolen device, so none is voided;
- the admitted devices are those of [Admitted devices](../protocol/membership.md#admitted-devices) for a channel that is not shared, without `custodian` or `auditor` devices;
- a fleet's devices are `client` devices of the channel's one member account, certified by its master key.

## Findings

### OpenMLS
- **Saving state:** `MemoryStorage` saves and loads only as a JSON file, with the `persistence` feature; its `load_from_file` panics on a malformed entry. `OpenMlsRustCrypto` cannot take a loaded storage. Workaround: a provider of `RustCrypto` and `MemoryStorage` that saves the storage's public `values` map as bytes, the form a device's state takes.
- **Signing key:** the `Signer` trait, two methods, signs with the device's key; the key never enters the storage.
- **Wire formats:** `PURE_PLAINTEXT_WIRE_FORMAT_POLICY` makes handshake messages PublicMessages; application messages are always PrivateMessages. The policy goes into both the create and the join config.
- **Decoding:** `MlsMessageIn::into_keypackage` and `into_welcome` exist only with `test-utils`; the prototype matches `extract()` and validates the KeyPackage with `KeyPackageIn::validate`.
- **Sender:** a processed message gives the sender's credential, not its `signature_key`; the prototype reads the sender's leaf with `member_at` before merging a Commit.
- **Persistence:** each operation writes to the storage by itself; a pending Commit survives a reload.
- **Replay:** rejected with `SecretReuseError`.
- **Own messages:** processing an application message the device sent gives `OwnPrivateMessage`, not an error.
- **Authenticated data:** `set_aad` before the Commit, for the next message only. A processed message gives it with `aad()`.
- **Reading before merging:** `process_message` stages another member's Commit; the device reads its `aad()` and the removed leaves, `remove_proposals` with `member_at`, and calls `merge_staged_commit` only if they pass.
- **Empty Commit:** `self_update` with default `LeafNodeParameters` and no stored proposals.
- **History key:** `MlsGroup::export_secret`.
- **Removed device:** processing its own Remove merges into an inactive group at the new epoch; `export_secret` and `create_message` then fail with `UseAfterEviction`.
- **Late messages:** a member in a new epoch rejects every message of an earlier one, the removed device's and other members' alike, with `Generation is too old to be processed`: the group config keeps no past epochs unless `max_past_epochs` is set.
- **KeyPackage lifetime:** the builder's default is 84 days from an hour back; `Lifetime::init` sets both bounds. Validation does not bound a lifetime's length, which RFC 9420, §7.2, asks applications to do: `Lifetime::has_acceptable_range` exists, but nothing calls it.
- **Last resort:** `mark_as_last_resort` adds the `last_resort` extension, and joining from a Welcome keeps such a KeyPackage's private keys; a regular one's are deleted, and its second Welcome fails with `NoMatchingKeyPackage`. The builder's default capabilities list no extensions, and `KeyPackageIn::validate` refuses a KeyPackage with an extension its capabilities lack: a last-resort KeyPackage from the default builder fails validation. Workaround: every KeyPackage lists `LastResort` in its capabilities.
- **KeyPackage checks:** `KeyPackageIn::validate` verifies both signatures, the lifetime at the current time, the protocol version and distinct init and encryption keys. `add_members` refuses another ciphersuite when it builds the Commit (`InvalidAddProposalCiphersuiteOrVersion`). Nothing checks the credential: the device checks the validated KeyPackage's credential and `signature_key` against the admitted devices before `add_members`.
- **Added devices:** a staged Commit's `add_proposals` give each added KeyPackage, with its credential and `signature_key`, before the merge.

### mls-rs
- **Saving state:** the in-memory storages have no save or load. `InMemoryKeyPackageStorage` exports through `key_packages()` and imports through `insert()`. `InMemoryGroupStateStorage` exports through `stored_groups()` and its `GroupStateStorage` methods, but lists no prior epoch IDs: an export walks back from `max_epoch_id`. Workaround: one store implementing `GroupStateStorage` and `KeyPackageStorage` that saves its maps directly, about 100 lines, instead of export and import code over two storages; its `GroupState` and `EpochRecord` need a direct dependency on `mls-rs-core`.
- **Signing key in the state:** a group's saved state holds the signer's secret key, so the device's Ed25519 key leaves its key store for every group's state.
- **Signing key format:** `SignatureSecretKey` for Ed25519 is the 64 bytes of the seed and the public key; the docs do not say so.
- **Client:** the client's config type cannot be named without spelling out the builder's generics; the prototype builds a client per step in a function returning `Client<impl MlsConfig>`.
- **Defaults:** handshake messages are PublicMessages and the Welcome carries the ratchet tree without configuration.
- **Persistence:** every operation needs an explicit `write_to_storage()`; a pending Commit is in the group's state and survives a reload.
- **Sender:** a processed message or Commit gives the sender's leaf index, and a Commit the removed members' indexes; the prototype takes their credentials from the roster it reads before processing.
- **Replay:** rejected with `KeyMissing`.
- **Own messages:** processing an application message the device sent fails.
- **Authenticated data:** `CommitBuilder::authenticated_data` and `Group::commit(authenticated_data)`. A processed Commit gives it in `CommitMessageDescription`.
- **Reading before merging:** `process_incoming_message` merges another member's Commit at once; there is no staged Commit to check. Workaround: the device checks the processed Commit and only then writes the group with `write_to_storage()`; a failed check drops the merged group, unsaved.
- **Empty Commit:** `Group::commit` with no proposals always has an UpdatePath.
- **History key:** `Group::export_secret`.
- **Removed device:** processing its own Remove succeeds with `CommitEffect::Removed` but leaves the group in its old epoch with its keys: the device still derives the old history key and encrypts. The device has to act on the effect itself.
- **Late messages:** a member rejects a message of a prior epoch with `MemberNotFound` when the sender's leaf is now blank or holds another signature key, as after the sender's Remove: what the removed device sends from its copy, and also what it sent before its Remove. An UpdatePath keeps the leaf's signature key, so earlier messages of a committer stay readable.
- **KeyPackage lifetime:** the default is 365 days from the time `generate_key_package_message` is given, or from now, without a margin for clocks that are behind. Workaround: `key_package_lifetime` of 60 days from a time an hour back. No check bounds a lifetime's length (RFC 9420, §7.2).
- **Last resort:** `LastResortKeyPackageExt` exists only with the `last_resort_key_package_ext` feature, off by default. Joining from a Welcome keeps such a KeyPackage's private keys; a regular one's are deleted, and its second Welcome fails with `WelcomeKeyPackageNotFound`. mls-rs does not require a KeyPackage's extensions in its capabilities, as OpenMLS does; `extension_type` lists `last_resort` there.
- **KeyPackage checks:** committing an Add checks its KeyPackage: both signatures, the lifetime at the current time, the protocol version and ciphersuite (`CipherSuiteMismatch`) and the init key. A received Commit's Adds get no lifetime check unless the message is processed with `process_incoming_message_with_time`.
- **Credential check:** the client's `IdentityProvider::validate_member` runs on every leaf mls-rs validates: an Add's KeyPackage, before its signatures, a Commit's UpdatePath leaf and every leaf of a tree it joins. A member has to process a Commit that adds a device that is not admitted, then remove it, so the prototype's provider refuses devices only in the client that commits an Add. The provider's error type has to implement `IntoAnyError`; the prototype wraps `BasicIdentityProvider` for everything else.
- **Added devices:** a processed Commit's `applied_proposals` give each Add's `signing_identity`.
