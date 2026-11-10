# MLS prototypes

Prototypes on OpenMLS and mls-rs for choosing the crypto core's MLS library (Story 3 in [task.md](../../task.md)).

- Code: [`prototypes/mls/`](../../prototypes/mls), a cargo workspace apart from the product crates. `mls-scenario` holds the library-neutral suite; `mls-proto-openmls` and `mls-proto-mls-rs` implement its `Device` and `Observer` on each library; their examples are the [WASM](#wasm) modules whose size the report gives. `mls-interop` runs the [interoperability](#interoperability) scenarios on both. `mls-bench` holds the [benchmarks](#benchmarks).
- Versions: OpenMLS 0.9.2 with `openmls_rust_crypto` 0.6.0, and the `js` feature in WASM; mls-rs 0.57.0 with the `external_client` and `last_resort_key_package_ext` features and `mls-rs-crypto-rustcrypto` 0.23.0, with the `browser` feature in WASM.
- Profile: [membership.md](../protocol/membership.md#mls-profile), "MLS profile". The device's Ed25519 signing key is generated outside the library.
- Every step of a device loads its saved state and saves it after the step, as one-shot mode does ([architecture.md](architecture.md#components)). A failed step leaves the saved state as it was.
- Run: `cargo test --manifest-path prototypes/mls/Cargo.toml`. In WASM, in Node.js, from `prototypes/mls`: `cargo test --target wasm32-unknown-unknown -p mls-proto-openmls -p mls-proto-mls-rs --test scenarios`; tools: [CONTRIBUTING.md](../../CONTRIBUTING.md#requirements). `script/test` runs both.

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
| `commits_carry_the_new_epochs_group_info` | The creator's Commit, a member's Add, the added device's empty Commit and a member's Remove each come with a GroupInfo of the new epoch, signed by the committer's leaf, with `ratchet_tree` and `external_pub`; the service keeps the latest and follows the members. It refuses a GroupInfo of an earlier epoch, one from the removed device, and one without either extension signed again by its signer; it takes the latest from another member |
| `two_commits_in_one_epoch_converge` | In epoch 1, device 0 commits an Add and device 1 the Remove of device 3. The service accepts the Add and refuses the Remove with `epoch-mismatch`; device 1 has no pending Commit, stays in epoch 1, processes the Add and, device 3 still a member, commits the Remove again. All, with the service, converge on epoch 3 with one history key |
| `three_commits_in_one_epoch_converge` | In epoch 1, device 0 commits an Add and devices 1 and 2 each the Remove of device 3. The service accepts device 1's and refuses the others with `epoch-mismatch`. After processing it, device 2 has nothing left to commit; device 0 commits the Add again. All converge on epoch 3 |
| `commit_for_an_old_epoch_is_refused` | A Commit made in epoch 1 after the service accepted another, that accepted Commit again, and the channel's first Commit again each get `epoch-mismatch` and leave the service's epoch, Commits and GroupInfo. The refused device processes the accepted Commit and commits again |
| `commit_from_a_non_member_is_refused` | A device outside the group creates its own group with the channel's ID and commits in its epoch 1, the channel's: its Commit gets `not-a-member` from it and from device 0, whose leaf it names; a member's Commit sent by another member or by the outsider gets `not-a-member`; the fork's GroupInfo is refused. The service changes nothing and then accepts the member's Commit from the member |
| `two_creators_end_in_the_kept_group` | [Creating a channel](../protocol/membership.md#creating-a-channel): devices 0 and 1 each create the channel's group and commit epoch 0. The service keeps device 0's Commit and refuses device 1's with `epoch-mismatch`; it refuses device 1's GroupInfo and takes device 0's. Device 1 drops its group and joins the kept one from device 0's Welcome, as device 2 does; all read each other |
| `device_joins_by_itself` | Joining by itself, [Group changes](../protocol/membership.md#group-changes): a new device reads the latest GroupInfo's signer and members, which pass, and commits an external Commit: a PublicMessage from a new member with an ExternalInit and an UpdatePath, no Welcome. The service refuses it from a member and from another device with `not-a-member`, accepts it from the joiner and keeps its GroupInfo. Every member reads the joiner as the sender and the one device added. A second device's external Commit from the same GroupInfo gets `epoch-mismatch`; it drops the group and joins from the latest GroupInfo. All converge and read each other |
| `joiner_that_is_not_admitted_is_removed` | A device without a certificate joins by itself. A member reads it as an added device that is not admitted and commits its Remove; all converge without it, no one has anyone left to remove, and it reads no later message |
| `group_info_that_fails_a_check_is_refused` | A new device refuses to join by itself from a GroupInfo that lists a member that is not admitted, one signed by such a member, and the latest GroupInfo signed again by another key or for a leaf that does not exist, which the library refuses before the check; it is left without the group. It joins from the latest GroupInfo once a member removed the device that is not admitted |
| `welcome_that_fails_a_check_is_refused` | A new device refuses a Welcome to a group with a member that is not admitted and one from a device that is not admitted, in a group of its own with the channel's ID; it reads each Welcome's signer and members and is left without the group. It joins from a Welcome that passes and reads the channel |
| `device_that_missed_a_commit_rejoins` | [Rejoining](../protocol/membership.md#rejoining): a device that missed a Commit fails on the next one and stays in its epoch. It rejoins from the latest GroupInfo with an external Commit that removes its old leaf; members read it as the Remove of its old leaf and its Add. All converge, the device with one leaf, and read each other |
| `offline_device_catches_up` | A device offline for 100 Commits, empty ones, an Add and a Remove among them, processes each in order with its sender and changes, then reads and sends new messages |
| `browser_reads_the_members_from_the_group_info` | A linked browser, outside the group, takes the member devices from the service's latest GroupInfo ([Envelope](../protocol/README.md#envelope)) once it passes the checks of [Wraps](../protocol/membership.md#wraps): of the channel and the wrap's epoch, signed by a member leaf, listing the wrap's signer, every member admitted. It reads epoch 1's members for a wrap by each member, and epoch 2's after a Remove |
| `group_info_that_fails_the_browser_check_is_rejected` | The browser rejects the latest GroupInfo signed again by a removed device at its old, blank leaf, by another key at the signer's leaf, or for a leaf that does not exist; with its tree altered to list the removed device in a member's leaf, signed again by its signer or not; with the tree of epoch 1, signed again by its signer; for another channel, or a wrap of an earlier or a later epoch; for a wrap by a device it does not list; and with a member the browser does not admit |

The failing KeyPackages are the device's own, changed one field at a time and signed again with its key by `mls-scenario`; signed again unchanged, a KeyPackage is byte for byte the library's.

Certificates and revocations are encoded as [Device certificate](../protocol/membership.md#device-certificate) and [Revocation](../protocol/membership.md#revocation) say, in deterministic CBOR. A test fixture, `Roster`, stands in for the certificates, revocations and pinned master keys a device gets and the roster revision it accepted:
- a certificate counts only if it passes the proof check, and one device's certificates with two bodies make it invalid;
- a certificate is valid by checks 1 to 4 of [Device checks](../protocol/membership.md#device-checks); a revocation counts as "Revocation" says, but no scenario has a stolen device, so none is voided;
- the admitted devices are those of [Admitted devices](../protocol/membership.md#admitted-devices) for a channel that is not shared, without `custodian` or `auditor` devices;
- a fleet's devices are `client` devices of the channel's one member account, certified by its master key.

A minimal in-process Delivery Service, `DeliveryService`, stands in for `skald-server`. It knows a channel only from its PublicMessage Commits and GroupInfo, through each library's party outside the group, its `Observer`:
- it keeps a channel's first Commit of epoch 0 without checking it, and refuses later ones with `epoch-mismatch` ([Creating a channel](../protocol/membership.md#creating-a-channel));
- it accepts a later Commit as [Commit ordering](../protocol/membership.md#commit-ordering) says: of the current epoch, else `epoch-mismatch`; verified by its `Observer`, with the sender leaf's `identity` the sending device's, else `not-a-member`, as is one sent before the first GroupInfo; the `Observer` then moves to the new epoch. An external Commit's sender leaf is the joiner's new leaf;
- it keeps an uploaded GroupInfo if it is of the current epoch, has `ratchet_tree` and `external_pub`, verifies, lists its uploader, and has the `Observer`'s GroupContext. The first one, of epoch 1, it takes only from the sender of the kept Commit and starts its `Observer` from it;
- members process its accepted Commits in its order; a device merges its Commit only after the service accepts it and drops it if refused. A device's external Commit is applied at once; the device drops the group if the service refuses it.

A device joining by external Commit or from a Welcome checks the GroupInfo's signer and every member it lists against the fixture's admitted devices; a member processing an external Commit checks the joiner as an Add.

## WASM
- Both prototypes build for `wasm32-unknown-unknown`. The whole suite, the browser check among it, passes in Node.js as natively, with the same scenarios.
- The browser check, `mls_scenario::browser`, is library-neutral: the library's `Observer` verifies the GroupInfo and its tree; the check makes the other checks of [Wraps](../protocol/membership.md#wraps).
- Sizes of each library's module: `<library>-browser-check` exports only the browser check; `<library>-suite` runs the whole suite. [`prototypes/mls/wasm-size`](../../prototypes/mls/wasm-size) builds them and prints the table; besides the WASM test tools, it needs `wasm-opt` (Binaryen) and `brotli`.
- Build: the `wasm` profile, `release` with `opt-level = "s"`, fat LTO and one codegen unit; `wasm-bindgen --target web` without the name and producers sections; `wasm-opt -Oz`. Measured with Rust 1.99.1, wasm-bindgen 0.2.130 and wasm-opt 133.

| Module | Raw, KiB | gzip -9, KiB | Brotli -q 11, KiB |
|---|---|---|---|
| `openmls-browser-check` | 391.1 | 145.9 | 115.4 |
| `openmls-suite` | 1472.1 | 590.8 | 434.8 |
| `mls-rs-browser-check` | 269.9 | 114.9 | 94.9 |
| `mls-rs-suite` | 1014.0 | 425.6 | 336.2 |

## Interoperability
One channel with devices on both prototypes: a device on one library takes each step, devices on the other process it and read it as in the [scenarios](#scenarios). Each scenario runs in both directions, OpenMLS to mls-rs and mls-rs to OpenMLS, natively. `mls_scenario::interop` holds them; its `Mixed` device runs each device on its own library, and its saved state names that library.

After each step, every member is at the same epoch with the same members in leaf order and derives the same history key, and each member's message is decrypted by every other one with its sender's leaf.

| Scenario | Checks |
|---|---|
| `creates_the_channel` | A device creates the group and commits the Add of devices on both libraries from their KeyPackages; they join from its Welcome |
| `adds_a_device` | A member adds a device on each library to a channel the other library created; members on the other library read the added leaves before they merge; the new devices join from its Welcome |
| `removes_a_device` | A member commits the Remove of a device on the other library with its revocation in `authenticated_data`, which members on the other library read before they merge. The removed device processes its Remove and sees itself removed; from its state before and after, it reads no later message |
| `makes_an_empty_commit` | A member commits an empty Commit with a browser's revocation in `authenticated_data`: an UpdatePath and no proposals; members on the other library read the revocation before they merge |
| `joins_by_external_commit` | A device joins by external Commit from the GroupInfo of a channel on the other library, whose members check it as an Add. It misses a Commit and rejoins from the next GroupInfo with an external Commit that removes its old leaf; members read the Remove and the Add |
| `messages_flow_both_ways` | Each of 4 devices, 2 on each library, sends 3 messages; every other device decrypts them, the first two out of order, with the sender's leaf |
| `devices_run_on_their_library` | The harness: each device keeps its library across steps; the steps no scenario takes run on it |

**Result:** no mismatch: every scenario passes in both directions on OpenMLS 0.9.2 and mls-rs 0.57.0. They exposed a defect of the mls-rs prototype's rejoin, not a mismatch: [mls-rs](#mls-rs), "External Commit".

## Benchmarks
`mls-bench` times each channel operation on both prototypes through the suite's `Device`, `Fleet` and `DeliveryService`, on groups of 2, 10, 100 and 1000 devices. [`prototypes/mls/benchmarks`](../../prototypes/mls/benchmarks) runs them and prints the results below; it needs the WASM test tools. `script/test` runs each benchmark once on a group of 2, natively and in WASM.

- Group: device 0 creates it with the other devices from their KeyPackages; only device 1 joins from the Welcome. Every benchmark starts from that epoch 1, whose tree biases the results: [Tree](#tree).
- A run loads its device from the saved state, untimed, and times only the operation. The operation reads the group from the library's storage and writes it back, as a one-shot step does.
- Each benchmark runs at least 5 times, then until its runs take 1 s, at most 100 times. A cell is the median and, in parentheses, the fastest and the slowest run, in ms.
- Natively, the benchmarks run on every core and, with `RAYON_NUM_THREADS=1`, on one: "Parallelism" in [OpenMLS](#openmls) and [mls-rs](#mls-rs). WASM has one thread.

| Operation | Timed |
|---|---|
| Create the group with its devices | Device 0 creates the group, commits the Add of the other devices, each KeyPackage checked, and merges it |
| Commit an Add | Device 0 commits the Add of one device, its KeyPackage checked, and merges it |
| Process an Add | Device 1 checks the added device and merges the Commit |
| Commit a Remove | Device 0 commits the Remove of the last device, its revocation in `authenticated_data`, and merges it |
| Process a Remove | Device 1 reads the revocation and merges the Commit; in a group of 2, as the removed device |
| Commit an empty Commit | Device 0 commits an empty Commit and merges it |
| Process an empty Commit | Device 1 processes and merges it |
| Join from a Welcome | The device of the Add checks the Welcome's signer and members and joins |
| Join by external Commit | A device outside the group checks the GroupInfo's signer and members and joins |
| Process an external Commit | Device 1 checks the joiner as an Add and merges the Commit |
| Make a KeyPackage | Device 1 makes one |
| Encrypt a message, Decrypt a message | Device 0 encrypts 1 KiB; device 1 decrypts it |
| Catch up on missed Commits | Device 1 processes 10 empty Commits of device 0 in order |
| Delivery Service's Commit check | `DeliveryService::send` accepts the empty Commit: its `Observer` verifies it and the sender's leaf |
| Browser's GroupInfo check | `mls_scenario::browser::members` on the GroupInfo, for a wrap by device 0 |

Objects: a KeyPackage; the Commits of the operations; the Welcome creating the group, for all devices but its creator, and that of an Add, for one; the GroupInfo with `ratchet_tree` and `external_pub`; the saved state of device 1, a member of only this group.

### Tree
No member but device 0 has committed, so the group's tree has no parent nodes but those the creating Commit's UpdatePath sets. Once members have committed, most parent nodes are set. Against such a group of n devices:
- **Commits:** an UpdatePath encrypts a path secret to each node in the resolution of the committer's copath: here about n − 1 leaves, 82 bytes each; once members have committed, about log2(n) nodes. Every Commit with an UpdatePath, all but mls-rs's of an Add, is the most a group of n has: 82647 bytes for OpenMLS's empty Commit at 1000 devices, against about 10 ciphertexts. Its time grows with the recipients: on one core, an empty Commit takes about 50 µs more per device from 100 to 1000 devices. The times of committing and of joining by external Commit are the most for the size.
- **Processing:** a member decrypts one path secret either way, but here it decodes and verifies a Commit of about n ciphertexts instead of about log2(n). The rows that process a Commit with an UpdatePath, "Catch up on missed Commits" and "Delivery Service's Commit check" among them, are high by that work.
- **Joining:** a joining device and the browser verify the tree: here without parent nodes' parent hashes. "Join from a Welcome", the tree check of "Join by external Commit" and "Browser's GroupInfo check" are low by that work.
- **Sizes:** a set parent node adds 68 bytes to the encoded tree, its HPKE public key and parent hash: about 68 KB at 1000 devices. The GroupInfo, the Welcome of an Add and the saved state hold the tree: they are low by that. The Commit and the Welcome creating the group are those of any channel.
- **Per library:** OpenMLS's creating Commit has an UpdatePath, which sets device 0's direct path; mls-rs's has none ([mls-rs](#mls-rs), "Commits of Adds"). A joiner by external Commit, at leaf n, encrypts to the node over leaves 0 to 511 at 1000 devices on OpenMLS and to those 512 leaves on mls-rs: about 490 against about 1000 recipients, 40684 against 82592 bytes. The rows "Join by external Commit", "Process an external Commit" and "External Commit" compare unlike work. Device 0's Commits encrypt to the same leaves on both.

### Results
- Machine: Apple M3 Max, 14 cores, 36 GiB, macOS on arm64, shared with other workloads.
- Versions: OpenMLS 0.9.2, openmls_rust_crypto 0.6.0, mls-rs 0.57.0, mls-rs-crypto-rustcrypto 0.23.0; Rust 1.99.1, Node.js 26.5.0.
- Three runs at load averages from 2 to 9; below is the one at the lowest, 2.79, 4.94, 7.01 (1, 5, 15 min) before it and 2.25, 4.21, 6.51 after.
- Across the runs, the median cell's median changed by 15% on every core, 6% on one core and 1% in WASM. Changes over 30% are in cells under 7 ms.
- On every core, the libraries' parallel work competes with the load: this run's times are 8% below those of the run before it, at load averages of 3 to 8. On one core and in WASM, they are within 1%.

Native, every core:

| Operation, ms | Library | 2 | 10 | 100 | 1000 |
|---|---|---|---|---|---|
| Create the group with its devices | OpenMLS | 0.384 (0.382–0.421) | 1.56 (1.49–1.78) | 15.3 (14.8–18.1) | 302 (302–350) |
| Create the group with its devices | mls-rs | 0.284 (0.273–0.339) | 0.692 (0.590–0.819) | 3.70 (3.10–4.82) | 61.6 (44.5–85.9) |
| Commit an Add | OpenMLS | 0.462 (0.442–0.655) | 0.747 (0.676–0.936) | 2.60 (2.29–2.95) | 18.8 (15.7–24.4) |
| Commit an Add | mls-rs | 0.227 (0.214–0.305) | 0.280 (0.264–0.385) | 1.38 (1.24–1.86) | 7.05 (6.42–15.3) |
| Process an Add | OpenMLS | 0.304 (0.290–0.455) | 0.428 (0.396–0.610) | 1.22 (1.14–1.36) | 7.76 (7.23–9.77) |
| Process an Add | mls-rs | 0.114 (0.103–0.188) | 0.129 (0.118–0.164) | 0.427 (0.412–0.570) | 2.19 (2.14–2.59) |
| Commit a Remove | OpenMLS | 0.172 (0.171–0.261) | 0.632 (0.540–0.765) | 2.63 (2.24–3.06) | 20.8 (16.7–23.3) |
| Commit a Remove | mls-rs | 0.143 (0.140–0.211) | 0.459 (0.398–0.604) | 2.11 (1.80–2.32) | 16.0 (11.2–29.0) |
| Process a Remove | OpenMLS | 0.217 (0.208–0.298) | 0.491 (0.435–0.727) | 1.41 (1.26–1.67) | 6.54 (6.18–9.62) |
| Process a Remove | mls-rs | 0.156 (0.156–0.189) | 0.262 (0.255–0.311) | 0.621 (0.618–0.693) | 3.69 (3.23–5.31) |
| Commit an empty Commit | OpenMLS | 0.217 (0.213–0.277) | 0.633 (0.547–0.821) | 2.66 (2.46–2.96) | 20.4 (14.1–22.7) |
| Commit an empty Commit | mls-rs | 0.194 (0.193–0.221) | 0.461 (0.399–0.606) | 1.93 (1.73–2.24) | 16.4 (13.3–19.7) |
| Process an empty Commit | OpenMLS | 0.169 (0.168–0.206) | 0.404 (0.351–4.86) | 1.29 (1.16–1.48) | 6.22 (6.02–9.50) |
| Process an empty Commit | mls-rs | 0.113 (0.111–0.130) | 0.165 (0.164–0.203) | 0.482 (0.478–0.638) | 2.87 (2.62–4.03) |
| Join from a Welcome | OpenMLS | 0.230 (0.225–0.300) | 0.725 (0.719–0.949) | 3.90 (3.66–5.07) | 39.3 (38.5–42.1) |
| Join from a Welcome | mls-rs | 0.206 (0.127–0.321) | 0.294 (0.213–0.425) | 0.970 (0.875–1.12) | 8.89 (5.93–11.4) |
| Join by external Commit | OpenMLS | 0.412 (0.394–0.543) | 1.56 (1.48–1.70) | 7.98 (7.79–8.29) | 81.2 (80.5–82.6) |
| Join by external Commit | mls-rs | 0.664 (0.564–0.824) | 1.05 (0.896–1.22) | 4.15 (3.48–4.56) | 33.6 (28.3–40.5) |
| Process an external Commit | OpenMLS | 0.223 (0.215–0.273) | 0.361 (0.360–0.492) | 0.760 (0.744–0.909) | 5.56 (5.49–5.86) |
| Process an external Commit | mls-rs | 0.152 (0.151–0.188) | 0.217 (0.206–0.368) | 0.644 (0.572–0.711) | 3.20 (2.39–4.72) |
| Make a KeyPackage | OpenMLS | 0.0477 (0.0466–0.0780) | 0.0630 (0.0625–0.0996) | 0.0469 (0.0466–0.0703) | 0.0467 (0.0465–0.0640) |
| Make a KeyPackage | mls-rs | 0.0660 (0.0659–0.0821) | 0.0794 (0.0791–0.105) | 0.103 (0.103–0.133) | 0.0676 (0.0660–0.0806) |
| Encrypt a message | OpenMLS | 0.0423 (0.0421–0.0856) | 0.0930 (0.0923–0.120) | 0.316 (0.310–0.359) | 2.73 (2.69–2.86) |
| Encrypt a message | mls-rs | 0.0338 (0.0335–0.0515) | 0.0553 (0.0550–0.107) | 0.240 (0.238–0.283) | 1.29 (1.26–1.44) |
| Decrypt a message | OpenMLS | 0.0618 (0.0615–0.0964) | 0.119 (0.119–0.154) | 0.335 (0.329–0.379) | 2.76 (2.70–3.03) |
| Decrypt a message | mls-rs | 0.0381 (0.0379–0.0607) | 0.0625 (0.0621–0.0921) | 0.270 (0.250–0.313) | 1.44 (1.42–1.59) |
| Catch up on missed Commits | OpenMLS | 1.63 (1.61–1.73) | 4.05 (3.70–4.52) | 9.95 (8.91–10.8) | 71.9 (62.3–78.0) |
| Catch up on missed Commits | mls-rs | 1.13 (1.12–1.22) | 1.92 (1.90–2.05) | 4.15 (4.04–5.88) | 25.3 (25.1–25.6) |
| Delivery Service's Commit check | OpenMLS | 0.0715 (0.0711–0.0844) | 0.122 (0.116–0.156) | 0.351 (0.327–0.419) | 2.72 (2.67–2.84) |
| Delivery Service's Commit check | mls-rs | 0.0692 (0.0645–0.0888) | 0.0909 (0.0850–0.123) | 0.208 (0.186–0.287) | 1.77 (1.22–2.57) |
| Browser's GroupInfo check | OpenMLS | 0.103 (0.102–0.121) | 0.453 (0.451–0.464) | 3.47 (3.43–3.77) | 37.1 (36.8–37.9) |
| Browser's GroupInfo check | mls-rs | 0.134 (0.0761–0.201) | 0.222 (0.151–0.358) | 0.725 (0.630–0.869) | 8.52 (6.78–9.87) |

Native, one core:

| Operation, ms | Library | 2 | 10 | 100 | 1000 |
|---|---|---|---|---|---|
| Create the group with its devices | OpenMLS | 0.451 (0.447–0.521) | 1.45 (1.43–1.53) | 14.7 (14.6–15.4) | 302 (301–302) |
| Create the group with its devices | mls-rs | 0.291 (0.285–0.340) | 1.16 (1.14–1.22) | 12.0 (11.8–12.4) | 253 (240–284) |
| Commit an Add | OpenMLS | 0.478 (0.474–0.577) | 0.887 (0.864–1.11) | 6.19 (5.80–10.3) | 55.4 (55.0–58.1) |
| Commit an Add | mls-rs | 0.233 (0.222–0.273) | 0.284 (0.278–0.309) | 0.847 (0.832–0.901) | 6.81 (6.45–7.76) |
| Process an Add | OpenMLS | 0.289 (0.265–0.405) | 0.370 (0.339–0.418) | 0.915 (0.893–1.07) | 6.16 (6.07–6.49) |
| Process an Add | mls-rs | 0.120 (0.109–0.150) | 0.136 (0.132–0.160) | 0.321 (0.313–0.358) | 2.38 (2.25–2.64) |
| Commit a Remove | OpenMLS | 0.161 (0.159–0.277) | 0.722 (0.688–0.850) | 5.52 (5.46–6.02) | 53.7 (53.2–55.8) |
| Commit a Remove | mls-rs | 0.141 (0.140–0.178) | 0.588 (0.578–0.629) | 4.89 (4.84–5.13) | 49.9 (48.2–54.6) |
| Process a Remove | OpenMLS | 0.202 (0.200–0.227) | 0.386 (0.365–0.587) | 0.939 (0.923–1.04) | 6.20 (6.11–6.80) |
| Process a Remove | mls-rs | 0.156 (0.155–0.174) | 0.261 (0.255–0.293) | 0.502 (0.492–0.542) | 2.60 (2.58–2.82) |
| Commit an empty Commit | OpenMLS | 0.212 (0.209–0.227) | 0.785 (0.712–6.04) | 5.51 (5.48–6.02) | 53.4 (53.0–54.4) |
| Commit an empty Commit | mls-rs | 0.193 (0.192–0.210) | 0.632 (0.615–0.679) | 4.90 (4.87–5.13) | 48.2 (47.8–48.7) |
| Process an empty Commit | OpenMLS | 0.172 (0.169–0.188) | 0.319 (0.299–0.477) | 0.848 (0.828–1.06) | 6.07 (6.00–6.37) |
| Process an empty Commit | mls-rs | 0.111 (0.111–0.131) | 0.165 (0.164–0.206) | 0.405 (0.397–0.440) | 2.50 (2.47–2.67) |
| Join from a Welcome | OpenMLS | 0.230 (0.228–0.277) | 0.599 (0.597–0.659) | 3.71 (3.67–4.10) | 38.7 (38.4–39.2) |
| Join from a Welcome | mls-rs | 0.173 (0.168–0.215) | 0.414 (0.404–0.448) | 3.15 (3.13–3.36) | 31.4 (30.9–32.0) |
| Join by external Commit | OpenMLS | 0.404 (0.394–0.497) | 1.24 (1.23–1.41) | 9.10 (9.01–9.78) | 99.1 (98.9–99.5) |
| Join by external Commit | mls-rs | 0.574 (0.555–0.632) | 1.41 (1.39–1.49) | 10.7 (10.7–11.1) | 105 (105–106) |
| Process an external Commit | OpenMLS | 0.219 (0.215–0.266) | 0.309 (0.297–0.346) | 0.754 (0.740–0.883) | 5.58 (5.51–5.85) |
| Process an external Commit | mls-rs | 0.155 (0.151–0.184) | 0.171 (0.171–0.192) | 0.374 (0.368–0.406) | 2.41 (2.38–2.63) |
| Make a KeyPackage | OpenMLS | 0.0468 (0.0465–0.0622) | 0.0518 (0.0515–0.0646) | 0.0481 (0.0467–0.0770) | 0.0469 (0.0465–0.0672) |
| Make a KeyPackage | mls-rs | 0.0661 (0.0658–0.0828) | 0.0673 (0.0659–0.0755) | 0.0662 (0.0658–0.0813) | 0.0675 (0.0672–0.0810) |
| Encrypt a message | OpenMLS | 0.0425 (0.0422–0.0569) | 0.0766 (0.0762–0.0931) | 0.320 (0.313–0.440) | 2.75 (2.71–2.86) |
| Encrypt a message | mls-rs | 0.0338 (0.0335–0.0495) | 0.0461 (0.0459–0.0789) | 0.158 (0.155–0.176) | 1.28 (1.26–1.38) |
| Decrypt a message | OpenMLS | 0.0617 (0.0614–0.0763) | 0.0979 (0.0976–0.110) | 0.337 (0.333–0.367) | 2.78 (2.75–2.95) |
| Decrypt a message | mls-rs | 0.0382 (0.0380–0.0490) | 0.0521 (0.0518–0.0655) | 0.175 (0.174–0.205) | 1.45 (1.43–1.54) |
| Catch up on missed Commits | OpenMLS | 1.63 (1.62–1.69) | 2.83 (2.73–3.05) | 8.43 (8.36–9.46) | 61.1 (60.4–61.7) |
| Catch up on missed Commits | mls-rs | 1.13 (1.12–1.17) | 1.68 (1.67–1.86) | 4.06 (4.03–4.40) | 25.4 (25.0–26.3) |
| Delivery Service's Commit check | OpenMLS | 0.0717 (0.0712–0.0827) | 0.0984 (0.0959–0.115) | 0.332 (0.325–0.393) | 2.70 (2.64–3.26) |
| Delivery Service's Commit check | mls-rs | 0.0635 (0.0621–0.0805) | 0.0741 (0.0737–0.110) | 0.173 (0.171–0.242) | 1.20 (1.14–1.54) |
| Browser's GroupInfo check | OpenMLS | 0.103 (0.102–0.117) | 0.376 (0.374–0.409) | 3.47 (3.44–3.60) | 37.2 (36.9–37.6) |
| Browser's GroupInfo check | mls-rs | 0.105 (0.0958–0.166) | 0.348 (0.333–0.412) | 2.99 (2.97–3.13) | 30.1 (29.9–30.7) |

WASM, in Node.js:

| Operation, ms | Library | 2 | 10 | 100 | 1000 |
|---|---|---|---|---|---|
| Browser's GroupInfo check | OpenMLS | 0.259 (0.242–0.584) | 0.905 (0.883–1.06) | 7.99 (7.88–8.26) | 80.3 (80.1–81.3) |
| Browser's GroupInfo check | mls-rs | 0.207 (0.203–0.293) | 0.785 (0.768–0.899) | 7.22 (7.16–7.43) | 72.8 (72.4–73.5) |

Object sizes, the same on every run but OpenMLS's saved state, whose length varies by tens of bytes ([OpenMLS](#openmls), "Saved state size"):

| Object, bytes | Library | 2 | 10 | 100 | 1000 |
|---|---|---|---|---|---|
| KeyPackage | OpenMLS | 294 | 294 | 294 | 294 |
| KeyPackage | mls-rs | 296 | 296 | 296 | 296 |
| Commit creating the group | OpenMLS | 700 | 3147 | 29621 | 293423 |
| Commit creating the group | mls-rs | 466 | 2826 | 29378 | 294878 |
| Commit of an Add | OpenMLS | 818 | 1545 | 9030 | 82941 |
| Commit of an Add | mls-rs | 466 | 466 | 466 | 466 |
| Commit of a Remove | OpenMLS | 479 | 1308 | 8793 | 82704 |
| Commit of a Remove | mls-rs | 483 | 1312 | 8797 | 82708 |
| Empty Commit | OpenMLS | 490 | 1251 | 8736 | 82647 |
| Empty Commit | mls-rs | 494 | 1255 | 8740 | 82651 |
| External Commit | OpenMLS | 489 | 688 | 3511 | 40684 |
| External Commit | mls-rs | 575 | 1266 | 8681 | 82592 |
| Welcome creating the group | OpenMLS | 813 | 3745 | 34647 | 341753 |
| Welcome creating the group | mls-rs | 732 | 3204 | 31022 | 309124 |
| Welcome of an Add | OpenMLS | 1070 | 2718 | 19940 | 190244 |
| Welcome of an Add | mls-rs | 923 | 2451 | 19649 | 191549 |
| GroupInfo | OpenMLS | 675 | 2391 | 19611 | 189915 |
| GroupInfo | mls-rs | 628 | 2156 | 19352 | 191252 |
| Member's saved state | OpenMLS | 7348 | 17368 | 109814 | 1006363 |
| Member's saved state | mls-rs | 1295 | 4508 | 37642 | 354186 |

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
- **GroupInfo:** with `use_ratchet_tree_extension` in the group config, every Commit comes with the new epoch's GroupInfo with `ratchet_tree` and `external_pub`, signed by the committer.
- **Outside party:** `PublicGroup::from_external` takes the ratchet tree, which it does not read from the GroupInfo itself, a `ProposalStore` and a storage of its own. `process_message` stages a Commit, so the Delivery Service checks the sender's leaf before `merge_commit`. A member's membership tag goes unchecked, as without the group's secrets it must.
- **Dropping a Commit:** `clear_pending_commit` drops it. Processing another member's Commit with one pending merges the other's and drops the pending one. Merging a refused Commit moves the device to an epoch no member is in: no one decrypts what it sends.
- **Dropping a group:** `MlsGroup::delete`. A Welcome for a group whose ID the device has fails with `GroupAlreadyExists`, so a creator whose epoch-0 Commit is refused deletes its group first.
- **Checking a Welcome:** a `StagedWelcome` gives the signer's leaf, `welcome_sender`, and the members before `into_group`; the device checks them there.
- **GroupInfo signer:** nothing public gives a GroupInfo's signer. Workaround: the prototype reads its leaf index from the encoding and the leaf from a `PublicGroup` of the GroupInfo, which verified the signature with it. A GroupInfo signed by another key fails with `InvalidGroupInfoSignature`, one for a leaf that does not exist with `UnknownSender`.
- **External Commit:** `MlsGroup::external_commit_builder` verifies the GroupInfo and its tree and itself commits the Remove of a leaf with the device's signature key, so a rejoin needs nothing more. With `use_ratchet_tree_extension` in its join config, the Commit comes with the new epoch's GroupInfo. The new group's pending Commit cannot be cleared, only merged or the group deleted: the prototype merges it at once. The new group replaces the device's old state of it without an error, unlike a Welcome.
- **Processing an external Commit:** its sender is `Sender::NewMemberCommit`; the joiner's new leaf is the staged Commit's `update_path_leaf_node`, and `remove_proposals` give the leaves it removes. `PublicGroup` processes it the same way.
- **Removes in an external Commit:** nothing checks that an external Commit removes at most one leaf, the joiner's old one (RFC 9420, §12.2): `validate_external_commit` leaves it to the application. Unchecked, any holder of the latest GroupInfo can remove any member. The prototype's members and `Observer` refuse an external Commit with more than one Remove or with a Remove of a leaf whose `identity` is not the joiner's. Its test, `external_commit_that_removes_another_device_is_refused`, has a joiner with a member's signing key and another device ID: the builder removes only a leaf with the joiner's signature key.
- **Missed Commits:** the Commit after a missed one fails with `ValidationError(WrongEpoch)`; the group stays in its epoch. Processing 100 missed Commits in order needs no configuration.
- **WASM build:** needs the `js` feature, for the browser's clock and random number generator; without it, a `compile_error!` names the feature. `openmls_rust_crypto` also takes getrandom 0.2 through `rand_core` 0.6, which `js` does not reach: the prototype turns on getrandom 0.2's `js` feature itself. The module holds getrandom 0.2 and 0.4.
- **Browser check:** `PublicGroup::from_external` checks the tree first: each leaf's signature (`InvalidNodeSignature`), then the tree hash against the GroupInfo's (`TreeHashMismatch`). Then the signer's leaf (`UnknownSender` for a blank or missing one) and the GroupInfo's signature (`InvalidGroupInfoSignature`).
- **Parallelism:** natively, tree operations run in parallel with rayon, a required dependency; `RAYON_NUM_THREADS` limits it.
- **Commits of Adds:** every Commit has an UpdatePath, one of only Adds too. In the benchmarks' [tree](#tree), where no member but the creator has committed, the Commit of one Add is 81 KiB at 1000 devices; once members have committed, its UpdatePath holds about log2(n) ciphertexts.
- **Saved state size:** the storage keeps its values as JSON: a member's state is 1 MB at 1000 devices, and its length varies by tens of bytes with the values' bytes.

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
- **GroupInfo:** a Commit comes with the new epoch's GroupInfo, `external_commit_group_info`, only if the client's `MlsRules` allow external Commits: `CommitOptions::with_allow_external_commit`. It then has `ratchet_tree` and `external_pub`.
- **Outside party:** `ExternalClient` and `ExternalGroup` exist only with the `external_client` feature, off by default. A struct that holds an `ExternalGroup` spells out its config type with `WithIdentityProvider` and `WithCryptoProvider`. `process_incoming_message` moves the group to the new epoch at once; the Delivery Service processes a copy and keeps it only if the sender check passes.
- **Dropping a Commit:** `clear_pending_commit`, then `write_to_storage()`. Processing another member's Commit with one pending merges the other's and drops the pending one. Merging a refused Commit moves the device to an epoch no member is in: no one decrypts what it sends.
- **Dropping a group:** `GroupStateStorage` has no delete; the prototype's store drops the group. A Welcome for a group whose ID the device has replaces that group without an error.
- **Checking a Welcome:** `join_group` returns the group and the signer's index, `NewMemberInfo::sender`; the device checks the signer and the roster before `write_to_storage`.
- **GroupInfo signer:** `GroupInfo::sender`; `ExternalClient::observe_group` verifies the GroupInfo and gives the members. A GroupInfo signed by another key fails with `InvalidSignature`, one for a leaf that does not exist with `InvalidNodeIndex`.
- **External Commit:** `ExternalCommitBuilder::build` verifies the GroupInfo and its tree and applies the Commit at once: there is no pending external Commit to drop. It gives no GroupInfo of the new epoch; the device makes one with `group_info_message_allowing_ext_commit(true)`. It removes the device's old leaf only with `with_removal(index)`, which the identity provider's `valid_successor` must accept: the prototype finds the index by its signature key among the observed members. The new group takes the prior epochs the storage keeps of its group ID and fails with `InvalidEpoch` when it adds its own after them, as on a rejoin by a device that has processed a Commit. Workaround: the device drops its old state of the group before the build.
- **Processing an external Commit:** `CommitMessageDescription::is_external`; its `committer` is the joiner's index in the new epoch, so the joiner's leaf comes from the new roster. Its Remove of the old leaf is in `applied_proposals`.
- **Removes in an external Commit:** members and `ExternalGroup` enforce RFC 9420, §12.2: an external Commit with more than one Remove fails with `ExternalCommitWithMoreThanOneRemove`, one with a Remove of a leaf that the identity provider's `valid_successor` does not accept as the joiner's predecessor with `ExternalCommitRemovesOtherIdentity`.
- **Missed Commits:** the Commit after a missed one fails with `InvalidEpoch`; the group stays in its epoch. Processing 100 missed Commits in order needs no configuration.
- **WASM build:** on `wasm32`, mls-rs itself takes getrandom's `wasm_js` and reads the clock with `Date.now` through wasm-bindgen; the RustCrypto provider's `browser` feature turns on `wasm_js` for its own getrandom. Nothing else changes.
- **Browser check:** `ExternalClient::observe_group` checks the tree hash first: an altered tree fails with `TreeHashMismatch`, signed again or not. Then the signer's leaf (`ExpectedNode` for a blank one, `InvalidNodeIndex` past the tree) and the GroupInfo's signature (`InvalidSignature`).
- **Parallelism:** natively, with the default `rayon` feature, tree operations run in parallel; `RAYON_NUM_THREADS` limits it.
- **Commits of Adds:** a Commit of only Adds has no UpdatePath, as RFC 9420, §12.4, allows, unless the client's `MlsRules` give `CommitOptions::with_path_required`: the Commit of one Add is 466 bytes at every size, and after a group is created its tree has no parent nodes.

### Spec
- The server cannot check a channel's first Commit: it has no state of epoch 0. Nor can it tie the GroupInfo of epoch 1 to that Commit, so [Creating a channel](../protocol/membership.md#creating-a-channel) leaves open whose GroupInfo it takes.
- Before the first GroupInfo, the server cannot tell a member's Commit.
- [Send](../protocol/delivery.md#send) names no problem for a Commit the server cannot process, such as one whose signature does not verify.
- "MLS profile" says only that the server keeps the latest GroupInfo. Without a check, one member could give joiners and browsers a GroupInfo of another group.
- [Envelope](../protocol/README.md#envelope) has a browser take the members from the latest GroupInfo, but names its checks of that GroupInfo's channel, epoch and members only in [Wraps](../protocol/membership.md#wraps), before the browser writes entries.
- [Group changes](../protocol/membership.md#group-changes) says a joining device checks a GroupInfo's or a Welcome's members, but not what it does when one is not admitted. In the prototypes it refuses and joins by itself once a member has removed that device.
