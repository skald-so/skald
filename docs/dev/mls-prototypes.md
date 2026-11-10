# MLS prototypes

Prototypes on OpenMLS and mls-rs for choosing the crypto core's MLS library (Story 3 in [task.md](../../task.md)).

- Code: [`prototypes/mls/`](../../prototypes/mls), a cargo workspace apart from the product crates. `mls-scenario` holds the library-neutral suite; `mls-proto-openmls` and `mls-proto-mls-rs` implement its `Device` on each library.
- Versions: OpenMLS 0.9.2 with `openmls_rust_crypto` 0.6.0; mls-rs 0.57.0 with `mls-rs-crypto-rustcrypto` 0.23.0.
- Profile: [membership.md](../protocol/membership.md#mls-profile), "MLS profile". The device's Ed25519 signing key is generated outside the library.
- Every step of a device loads its saved state and saves it after the step, as one-shot mode does ([architecture.md](architecture.md#components)). A failed step leaves the saved state as it was.
- Run: `cargo test --manifest-path prototypes/mls/Cargo.toml`; `script/test` runs it too.

## Scenarios
| Scenario | Checks |
|---|---|
| `creating_a_channel` | [Creating a channel](../protocol/membership.md#creating-a-channel) on 3 devices: KeyPackages with ciphersuite `0x0001`, a `basic` credential with the device ID and the device's key as `signature_key`; the creator's group at epoch 0; its Add Commit a PublicMessage of epoch 0, pending until merged; the Welcome's ciphersuite; all at epoch 1 with the same members in leaf order |
| `member_adds_a_device` | A member other than the creator adds a device; the other members process the Commit, a PublicMessage, and get its sender; all at epoch 2 |
| `members_exchange_messages` | On 4 devices, each sends two messages: PrivateMessages without the plaintext; every other member decrypts each and gets the sender's device ID from its credential, derived from its `signature_key` |
| `replayed_message_is_rejected` | A member rejects an application message it already decrypted and still decrypts the next one |
| `removed_device_reads_nothing_later` | A member commits the Remove of a revoked device: a PublicMessage Commit with a Remove, an UpdatePath and the device's revocation in its `authenticated_data`; the others at epoch 2 without it. The removed device processes its Remove and sees itself removed. Its saved state from before the Remove and from after it processes the Remove fails on every later message: application messages of epoch 2, an empty Commit, a message of epoch 3. No member reads what it sends from its copy |
| `revocations_are_read_before_merging` | A member reads a Remove Commit before it merges it: its sender, the removed leaf and the revocations in its `authenticated_data`, which count. A failed check leaves the device in its epoch with its members; the same Commit merges once the check passes |
| `empty_commit_starts_a_new_epoch` | An empty Commit, after a browser's revocation with the revocation in its `authenticated_data`, then on schedule without one: a PublicMessage Commit with an UpdatePath and no proposals, no Welcome; every member reads it and enters the next epoch with the same members |
| `history_key_rotates_with_each_epoch` | Every member of an epoch derives the same history key, `MLS-Exporter("skald history", "", 16)`; messages and reloads keep it. A Remove and an empty Commit each change it. The removed device has epoch 1's key from its copy, processes its Remove, fails on the empty Commit and derives neither later key |

Revocations are encoded as [Revocation](../protocol/membership.md#revocation) says, in deterministic CBOR. Until certificates exist, a revocation counts if its signer is a member leaf of the epoch whose key verifies it, its `account` is the fleet's and it has no `also`.

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
- **Late messages:** a member rejects a message of a prior epoch whose sender's leaf has changed since, with `MemberNotFound`: what the removed device sends from its copy, and also what it sent before its Remove.
