# MLS prototypes

Prototypes on OpenMLS and mls-rs for choosing the crypto core's MLS library ([roadmap.md, "Preparation"](roadmap.md#preparation-before-mvp)).

- Code: [`prototypes/mls/`](../../prototypes/mls), a cargo workspace apart from the product crates. `mls-scenario` holds the library-neutral suite; `mls-proto-openmls` and `mls-proto-mls-rs` implement its `Device` and `Observer` on each library; their examples are the [WASM](#wasm) modules whose size the report gives. `mls-interop` runs the [interoperability](#interoperability) scenarios on both. `mls-bench` holds the [benchmarks](#benchmarks).
- Versions: OpenMLS 0.9.2 with `openmls_rust_crypto` 0.6.0, and the `js` feature in WASM; mls-rs 0.57.1 with the `external_client` and `last_resort_key_package_ext` features and `mls-rs-crypto-rustcrypto` 0.23.0, with the `browser` feature in WASM.
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
- the admitted devices are those of [Admitted devices](../protocol/membership.md#admitted-devices);
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
The creating Commit has no UpdatePath ([OpenMLS](#openmls) and [mls-rs](#mls-rs), "Commits of Adds") and no member has committed one, so the group's tree has no parent nodes. Once members have committed, most parent nodes are set. Against such a group of n devices:
- **Commits:** an UpdatePath encrypts a path secret to each node in the resolution of the committer's copath: here about n − 1 leaves, 82 bytes each; once members have committed, about log2(n) nodes. Every Commit with an UpdatePath, all but those of Adds, is the most a group of n has. Its time grows with the recipients: on one core, an empty Commit takes about 50 µs more per device from 100 to 1000 devices. The times of committing a Remove or an empty Commit and of joining by external Commit are the most for the size.
- **Processing:** a member decrypts one path secret either way, but here it decodes and verifies a Commit of about n ciphertexts instead of about log2(n). The rows that process a Commit with an UpdatePath, "Catch up on missed Commits" and "Delivery Service's Commit check" among them, are high by that work.
- **Joining:** a joining device and the browser verify the tree: here without parent nodes' parent hashes. "Join from a Welcome", the tree check of "Join by external Commit" and "Browser's GroupInfo check" are low by that work.
- **Sizes:** a set parent node adds 68 bytes to the encoded tree, its HPKE public key and parent hash: about 68 KB at 1000 devices. The GroupInfo, the Welcome of an Add and the saved state hold the tree: they are low by that. The Commit and the Welcome creating the group are those of any channel.

### Results
- Machine: Apple M3 Max, 14 cores, 36 GiB, macOS on arm64, shared with other workloads.
- Versions: OpenMLS 0.9.2, openmls_rust_crypto 0.6.0, mls-rs 0.57.0, mls-rs-crypto-rustcrypto 0.23.0; Rust 1.99.1, Node.js 26.5.0.
- Three runs at load averages from 2 to 13; below is the one at the lowest, 2.36, 6.25, 10.55 (1, 5, 15 min) before it and 2.23, 5.17, 9.67 after.
- Across the runs, the median cell's median changed by 13% on every core, 2% on one core and 1% in WASM. Changes over 30% are in cells under 4 ms, but OpenMLS's "Commit a Remove" at 1000 devices on every core: 15.0 to 20.3 ms.

Native, every core:

| Operation, ms | Library | 2 | 10 | 100 | 1000 |
|---|---|---|---|---|---|
| Create the group with its devices | OpenMLS | 0.290 (0.284–0.325) | 1.31 (1.29–1.41) | 14.5 (14.3–15.0) | 349 (304–409) |
| Create the group with its devices | mls-rs | 0.288 (0.277–0.336) | 0.767 (0.661–0.902) | 3.66 (3.20–4.42) | 56.9 (44.6–63.1) |
| Commit an Add | OpenMLS | 0.252 (0.250–0.303) | 0.331 (0.325–0.357) | 1.21 (1.20–1.39) | 10.2 (9.96–10.6) |
| Commit an Add | mls-rs | 0.239 (0.219–0.441) | 0.324 (0.318–0.456) | 1.27 (1.16–1.61) | 6.89 (6.43–13.3) |
| Process an Add | OpenMLS | 0.137 (0.137–0.217) | 0.176 (0.173–0.227) | 0.605 (0.592–0.700) | 5.02 (4.86–5.17) |
| Process an Add | mls-rs | 0.120 (0.104–0.166) | 0.138 (0.136–0.168) | 0.430 (0.418–0.537) | 2.18 (2.15–2.54) |
| Commit a Remove | OpenMLS | 0.149 (0.147–0.189) | 0.573 (0.490–0.821) | 2.05 (1.91–2.49) | 19.4 (14.5–25.5) |
| Commit a Remove | mls-rs | 0.141 (0.140–0.198) | 0.550 (0.487–0.646) | 2.11 (1.94–2.45) | 17.9 (12.5–21.6) |
| Process a Remove | OpenMLS | 0.190 (0.187–0.228) | 0.422 (0.385–0.518) | 1.04 (0.952–6.24) | 7.57 (6.77–9.86) |
| Process a Remove | mls-rs | 0.157 (0.156–0.178) | 0.346 (0.324–0.413) | 0.719 (0.667–0.791) | 3.30 (2.73–4.12) |
| Commit an empty Commit | OpenMLS | 0.201 (0.198–0.249) | 0.535 (0.475–0.720) | 2.00 (1.82–2.36) | 19.0 (15.2–19.9) |
| Commit an empty Commit | mls-rs | 0.195 (0.193–0.223) | 0.548 (0.483–0.679) | 2.07 (1.86–2.29) | 16.3 (12.1–19.0) |
| Process an empty Commit | OpenMLS | 0.158 (0.157–0.217) | 0.313 (0.279–0.442) | 0.937 (0.852–1.05) | 7.00 (6.41–8.83) |
| Process an empty Commit | mls-rs | 0.112 (0.112–0.126) | 0.223 (0.222–0.254) | 0.584 (0.542–0.643) | 3.28 (2.96–4.02) |
| Join from a Welcome | OpenMLS | 0.204 (0.200–0.237) | 0.475 (0.468–0.533) | 3.61 (3.56–3.99) | 37.9 (37.6–41.3) |
| Join from a Welcome | mls-rs | 0.209 (0.130–0.283) | 0.344 (0.274–0.426) | 1.04 (0.944–1.20) | 8.72 (6.79–10.1) |
| Join by external Commit | OpenMLS | 0.446 (0.428–0.574) | 1.19 (1.12–9.36) | 8.37 (8.03–8.99) | 84.3 (81.2–87.3) |
| Join by external Commit | mls-rs | 0.701 (0.613–0.824) | 1.24 (1.10–1.47) | 4.23 (3.83–4.83) | 32.2 (28.7–35.3) |
| Process an external Commit | OpenMLS | 0.213 (0.212–0.262) | 0.263 (0.258–0.311) | 0.794 (0.776–0.865) | 6.05 (5.82–6.69) |
| Process an external Commit | mls-rs | 0.176 (0.175–0.204) | 0.261 (0.247–0.295) | 0.589 (0.582–0.703) | 3.44 (2.86–4.90) |
| Make a KeyPackage | OpenMLS | 0.0469 (0.0466–0.0547) | 0.0468 (0.0465–0.0542) | 0.0495 (0.0491–0.0659) | 0.0468 (0.0465–0.0843) |
| Make a KeyPackage | mls-rs | 0.0762 (0.0760–0.0967) | 0.0958 (0.0954–0.117) | 0.104 (0.103–0.127) | 0.0799 (0.0797–0.0967) |
| Encrypt a message | OpenMLS | 0.0426 (0.0417–0.0563) | 0.0665 (0.0655–0.0925) | 0.327 (0.322–0.359) | 3.44 (2.73–3.74) |
| Encrypt a message | mls-rs | 0.0392 (0.0390–0.0559) | 0.0671 (0.0666–0.107) | 0.224 (0.222–0.265) | 1.33 (1.26–1.56) |
| Decrypt a message | OpenMLS | 0.0607 (0.0604–0.0701) | 0.0843 (0.0840–0.101) | 0.346 (0.341–0.354) | 3.10 (2.80–3.28) |
| Decrypt a message | mls-rs | 0.0442 (0.0440–0.0458) | 0.0752 (0.0747–0.0798) | 0.251 (0.232–0.292) | 1.47 (1.43–1.55) |
| Catch up on missed Commits | OpenMLS | 1.63 (1.61–1.70) | 3.36 (3.18–3.75) | 10.9 (9.59–11.9) | 62.5 (61.8–65.9) |
| Catch up on missed Commits | mls-rs | 1.29 (1.20–1.34) | 2.11 (2.00–2.48) | 5.20 (4.90–5.64) | 26.6 (25.7–27.7) |
| Delivery Service's Commit check | OpenMLS | 0.0717 (0.0714–0.0795) | 0.0963 (0.0956–0.110) | 0.351 (0.324–0.453) | 2.72 (2.65–3.45) |
| Delivery Service's Commit check | mls-rs | 0.0758 (0.0686–0.0975) | 0.0984 (0.0905–0.113) | 0.297 (0.256–0.393) | 2.15 (1.41–3.14) |
| Browser's GroupInfo check | OpenMLS | 0.101 (0.0997–0.112) | 0.367 (0.361–0.412) | 3.45 (3.35–3.88) | 36.5 (36.1–37.2) |
| Browser's GroupInfo check | mls-rs | 0.147 (0.0824–0.267) | 0.239 (0.164–0.356) | 0.998 (0.892–1.12) | 8.28 (6.92–9.76) |

Native, one core:

| Operation, ms | Library | 2 | 10 | 100 | 1000 |
|---|---|---|---|---|---|
| Create the group with its devices | OpenMLS | 0.409 (0.406–0.452) | 1.31 (1.30–1.46) | 14.5 (14.3–15.0) | 301 (300–320) |
| Create the group with its devices | mls-rs | 0.298 (0.289–0.351) | 1.18 (1.15–1.27) | 12.0 (11.8–12.4) | 241 (241–242) |
| Commit an Add | OpenMLS | 0.336 (0.333–0.395) | 0.332 (0.326–0.360) | 1.20 (1.18–1.29) | 10.6 (9.94–12.9) |
| Commit an Add | mls-rs | 0.232 (0.225–0.254) | 0.299 (0.281–0.326) | 0.840 (0.825–0.916) | 6.76 (6.44–7.27) |
| Process an Add | OpenMLS | 0.183 (0.183–0.246) | 0.177 (0.173–0.240) | 0.596 (0.585–0.681) | 5.14 (4.84–5.41) |
| Process an Add | mls-rs | 0.123 (0.110–0.175) | 0.140 (0.139–0.169) | 0.324 (0.317–0.352) | 2.39 (2.31–2.59) |
| Commit a Remove | OpenMLS | 0.201 (0.188–0.311) | 0.684 (0.673–0.751) | 5.50 (5.45–5.91) | 53.5 (53.0–55.2) |
| Commit a Remove | mls-rs | 0.145 (0.141–0.226) | 0.656 (0.650–0.687) | 4.89 (4.85–5.10) | 48.4 (48.0–50.6) |
| Process a Remove | OpenMLS | 0.235 (0.234–0.268) | 0.366 (0.354–0.434) | 0.921 (0.899–1.02) | 6.17 (6.07–6.52) |
| Process a Remove | mls-rs | 0.158 (0.157–0.183) | 0.296 (0.295–0.322) | 0.507 (0.497–0.566) | 2.62 (2.60–2.91) |
| Commit an empty Commit | OpenMLS | 0.252 (0.251–0.274) | 0.719 (0.703–0.817) | 5.49 (5.44–5.77) | 53.4 (53.1–56.1) |
| Commit an empty Commit | mls-rs | 0.199 (0.194–0.218) | 0.726 (0.693–0.821) | 4.97 (4.87–7.68) | 48.3 (47.9–48.9) |
| Process an empty Commit | OpenMLS | 0.199 (0.189–0.247) | 0.286 (0.262–5.48) | 0.824 (0.807–0.893) | 6.03 (5.96–6.36) |
| Process an empty Commit | mls-rs | 0.115 (0.111–0.157) | 0.199 (0.198–0.228) | 0.515 (0.483–0.792) | 2.53 (2.50–2.73) |
| Join from a Welcome | OpenMLS | 0.241 (0.240–0.287) | 0.551 (0.542–0.826) | 3.60 (3.56–3.79) | 37.9 (37.6–38.4) |
| Join from a Welcome | mls-rs | 0.175 (0.170–0.235) | 0.477 (0.475–0.516) | 3.92 (3.56–4.46) | 31.1 (30.9–31.5) |
| Join by external Commit | OpenMLS | 0.462 (0.451–0.631) | 1.64 (1.55–1.88) | 11.7 (11.6–12.0) | 120 (120–121) |
| Join by external Commit | mls-rs | 0.570 (0.558–0.646) | 1.63 (1.61–1.80) | 11.3 (10.7–13.5) | 106 (105–162) |
| Process an external Commit | OpenMLS | 0.213 (0.212–0.247) | 0.298 (0.297–0.350) | 0.769 (0.751–0.858) | 5.84 (5.78–6.21) |
| Process an external Commit | mls-rs | 0.153 (0.152–0.181) | 0.206 (0.205–0.220) | 0.379 (0.372–0.443) | 3.48 (3.19–3.88) |
| Make a KeyPackage | OpenMLS | 0.0467 (0.0465–0.0572) | 0.0537 (0.0535–0.0622) | 0.0468 (0.0465–0.0608) | 0.0467 (0.0465–0.0622) |
| Make a KeyPackage | mls-rs | 0.0674 (0.0661–0.0879) | 0.0798 (0.0795–0.0962) | 0.0664 (0.0661–0.0813) | 0.0967 (0.0960–0.131) |
| Encrypt a message | OpenMLS | 0.0428 (0.0418–0.0532) | 0.0765 (0.0761–0.127) | 0.310 (0.303–0.333) | 3.40 (2.69–3.72) |
| Encrypt a message | mls-rs | 0.0341 (0.0339–0.0547) | 0.0555 (0.0550–0.0850) | 0.155 (0.153–0.176) | 1.59 (1.50–2.00) |
| Decrypt a message | OpenMLS | 0.0619 (0.0616–0.0697) | 0.0975 (0.0972–0.107) | 0.328 (0.322–0.358) | 3.00 (2.72–3.24) |
| Decrypt a message | mls-rs | 0.0383 (0.0381–0.0479) | 0.0624 (0.0619–0.0697) | 0.174 (0.173–0.198) | 1.71 (1.62–1.79) |
| Catch up on missed Commits | OpenMLS | 1.63 (1.61–1.74) | 2.89 (2.75–3.18) | 8.44 (8.34–8.74) | 70.1 (61.0–77.0) |
| Catch up on missed Commits | mls-rs | 1.14 (1.12–1.18) | 1.94 (1.84–2.07) | 4.14 (4.10–4.35) | 27.5 (26.7–29.0) |
| Delivery Service's Commit check | OpenMLS | 0.0711 (0.0707–0.0821) | 0.0967 (0.0962–0.170) | 0.333 (0.325–0.396) | 2.70 (2.66–3.91) |
| Delivery Service's Commit check | mls-rs | 0.0623 (0.0619–0.0807) | 0.0819 (0.0796–0.0934) | 0.188 (0.178–0.242) | 1.21 (1.16–1.43) |
| Browser's GroupInfo check | OpenMLS | 0.100 (0.0998–0.117) | 0.367 (0.361–0.408) | 3.37 (3.34–3.49) | 36.4 (36.1–36.7) |
| Browser's GroupInfo check | mls-rs | 0.104 (0.0990–0.132) | 0.348 (0.333–0.379) | 3.01 (2.98–3.14) | 30.3 (29.9–31.5) |

WASM, in Node.js:

| Operation, ms | Library | 2 | 10 | 100 | 1000 |
|---|---|---|---|---|---|
| Browser's GroupInfo check | OpenMLS | 0.256 (0.242–0.571) | 0.878 (0.843–1.09) | 7.72 (7.59–8.13) | 77.9 (77.5–95.3) |
| Browser's GroupInfo check | mls-rs | 0.206 (0.202–0.291) | 0.776 (0.765–0.903) | 7.17 (7.10–7.37) | 71.9 (71.6–72.9) |

Object sizes, the same on every run but OpenMLS's saved state ([OpenMLS](#openmls), "Saved state size"):

| Object, bytes | Library | 2 | 10 | 100 | 1000 |
|---|---|---|---|---|---|
| KeyPackage | OpenMLS | 294 | 294 | 294 | 294 |
| KeyPackage | mls-rs | 296 | 296 | 296 | 296 |
| Commit creating the group | OpenMLS | 464 | 2808 | 29180 | 292880 |
| Commit creating the group | mls-rs | 466 | 2826 | 29378 | 294878 |
| Commit of an Add | OpenMLS | 464 | 464 | 464 | 464 |
| Commit of an Add | mls-rs | 466 | 466 | 466 | 466 |
| Commit of a Remove | OpenMLS | 479 | 1308 | 8793 | 82704 |
| Commit of a Remove | mls-rs | 483 | 1312 | 8797 | 82708 |
| Empty Commit | OpenMLS | 490 | 1251 | 8736 | 82647 |
| Empty Commit | mls-rs | 494 | 1255 | 8740 | 82651 |
| External Commit | OpenMLS | 571 | 1262 | 8677 | 82588 |
| External Commit | mls-rs | 575 | 1266 | 8681 | 82592 |
| Welcome creating the group | OpenMLS | 726 | 3182 | 30820 | 307122 |
| Welcome creating the group | mls-rs | 732 | 3204 | 31022 | 309124 |
| Welcome of an Add | OpenMLS | 915 | 2427 | 19445 | 189545 |
| Welcome of an Add | mls-rs | 923 | 2451 | 19649 | 191549 |
| GroupInfo | OpenMLS | 622 | 2134 | 19150 | 189250 |
| GroupInfo | mls-rs | 628 | 2156 | 19352 | 191252 |
| Member's saved state | OpenMLS | 6790 | 15031 | 105492 | 1000440 |
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
- **KeyPackage checks:** `KeyPackageIn::validate` verifies both signatures, the lifetime at the current time, the protocol version and distinct init and encryption keys. `add_members_without_update` refuses another ciphersuite when it builds the Commit (`InvalidAddProposalCiphersuiteOrVersion`). Nothing checks the credential: the device checks the validated KeyPackage's credential and `signature_key` against the admitted devices before `add_members_without_update`.
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
- **Commits of Adds:** `add_members` forces an UpdatePath; `add_members_without_update` and `commit_builder`, whose `force_self_update` defaults to `false`, commit only Adds without one, as RFC 9420, §12.4, allows. The prototype uses `add_members_without_update`.
- **Saved state size:** the storage keeps its values as JSON, so a member's state varies in length by up to 150 bytes with the values' bytes.

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
- **Commits of Adds:** a Commit of only Adds has no UpdatePath, as RFC 9420, §12.4, allows, unless the client's `MlsRules` give `CommitOptions::with_path_required`.

### Spec
- The server cannot check a channel's first Commit: it has no state of epoch 0. Nor can it tie the GroupInfo of epoch 1 to that Commit, so [Creating a channel](../protocol/membership.md#creating-a-channel) leaves open whose GroupInfo it takes.
- Before the first GroupInfo, the server cannot tell a member's Commit.
- [Send](../protocol/delivery.md#send) names no problem for a Commit the server cannot process, such as one whose signature does not verify.
- "MLS profile" says only that the server keeps the latest GroupInfo. Without a check, one member could give joiners and browsers a GroupInfo of another group.
- [Envelope](../protocol/README.md#envelope) has a browser take the members from the latest GroupInfo, but names its checks of that GroupInfo's channel, epoch and members only in [Wraps](../protocol/membership.md#wraps), before the browser writes entries.
- [Group changes](../protocol/membership.md#group-changes) says a joining device checks a GroupInfo's or a Welcome's members, but not what it does when one is not admitted. In the prototypes it refuses and joins by itself once a member has removed that device.

## Comparison
OpenMLS 0.9.2 against mls-rs 0.57.0. Facts from outside this repository were checked on 2026-11-11 at the linked sources.

### RFC 9420 compliance
**Test vectors:** the [official set](https://github.com/mlswg/mls-implementations/tree/2034fe72e0c805a683ba76165660c834f2a8142a/test-vectors) has 16 files. Each library's tests read its own copies, OpenMLS's [`test_vectors`](https://github.com/openmls/openmls/tree/v0.9.2/openmls/test_vectors) and mls-rs's [`test_data`](https://github.com/awslabs/mls-rs/tree/0.57.0/mls-rs/test_data); CI passed on each tagged commit ([OpenMLS](https://github.com/openmls/openmls/commit/8a8f83b4b61fc45f28facec25e914992b3465de6), [mls-rs](https://github.com/awslabs/mls-rs/commit/a1436f4a5433214ed62349e77835b57a0accdc42)). The prototypes do not run them.

| Copy of an official file | OpenMLS | mls-rs |
|---|---|---|
| Identical | 12: `deserialization`, `key-schedule`, `messages`, the three `passive-client-*`, `secret-tree`, `tree-math`, `tree-operations`, `tree-validation`, `treekem`, `welcome` | 6: `crypto-basics`, `key-schedule`, `secret-tree`, `transcript-hashes`, `tree-validation`, `treekem` |
| A subset of its cases | `psk_secret`: 70 of 77 | `tree-math`: 8 of 10 |
| Its format, other values | `crypto-basics`, `message-protection`, `transcript-hashes` | `message-protection`, `messages`, `psk_secret`, the three `passive-client-*`; `tree-operations` without the ciphersuite and tree hashes |
| None | — | `deserialization`, `welcome` |
| Crypto provider of the run | RustCrypto, the prototypes' ([`Cargo.toml`](https://github.com/openmls/openmls/blob/v0.9.2/openmls/Cargo.toml#L184), [`tests.yml`](https://github.com/openmls/openmls/blob/v0.9.2/.github/workflows/tests.yml#L48)) | OpenSSL ([`crypto.rs`](https://github.com/awslabs/mls-rs/blob/0.57.0/mls-rs/src/crypto.rs#L20-L24)); the RustCrypto provider passes `mls-rs-core`'s provider tests ([`mls_core_tests`](https://github.com/awslabs/mls-rs/blob/0.57.0/mls-rs-crypto-rustcrypto/src/lib.rs#L407-L419), run by [`native_build.yml`](https://github.com/awslabs/mls-rs/blob/0.57.0/.github/workflows/native_build.yml#L63-L68)) |

**Interoperability:**
- With each other: no mismatch ([Interoperability](#interoperability)).
- OpenMLS: one manual run of its interop workflow, on 2026-09-23 from a development branch, against mls-rs and MLS++ through the mlswg test runner, reports no interop failures ([run](https://github.com/openmls/openmls/actions/runs/35860183185)).
- mls-rs: its CI runs that test runner against mls-rs only ([`interop_tests.yml`](https://github.com/awslabs/mls-rs/blob/0.57.0/.github/workflows/interop_tests.yml)).

**Gaps the prototypes found:** OpenMLS does not check an external Commit's Removes, RFC 9420, §12.2 ([OpenMLS](#openmls), "Removes in an external Commit"); mls-rs does. Neither bounds a KeyPackage lifetime's length (§7.2).

### Audit
- **OpenMLS:** no published audit found. Its [README](https://github.com/openmls/openmls/blob/v0.9.2/README.md), [SECURITY.md](https://github.com/openmls/openmls/blob/main/SECURITY.md) on `main`, the [repository at v0.9.2](https://github.com/openmls/openmls/tree/v0.9.2) and [openmls.tech](https://openmls.tech) name none; an audit, its scope or date cannot be confirmed.
- **mls-rs:** none. Its [README](https://github.com/awslabs/mls-rs/blob/0.57.0/README.md#security-notice): "validated for conformance to the RFC 9420 specification but has not yet received a full security audit by a 3rd party".

### API ergonomics
From the [findings](#findings):
- **OpenMLS** matches the check-then-merge steps of the spec: a staged Commit and a `StagedWelcome` give everything before the merge. The device's signing key stays outside the library's state. Workarounds: "Saving state", "Decoding", "Sender", "Last resort", "Outside party", "Dropping a group", "GroupInfo signer", "External Commit", "Removes in an external Commit", "WASM build".
- **mls-rs** checks more itself: an external Commit's Removes, the GroupInfo's signer. It merges a processed Commit at once and keeps a removed device in its old epoch, so the device checks before `write_to_storage()` and acts on `CommitEffect::Removed`. It copies the signing key into each group's state. Two needed features are off by default. Workarounds: "Saving state", "Signing key format", "Client", "Sender", "Reading before merging", "Removed device", "KeyPackage lifetime", "Credential check", "GroupInfo", "Outside party", "Dropping a group", "Checking a Welcome", "External Commit", "Processing an external Commit".

### Performance
From the [results](#results):
- At 2 and 10 devices, the libraries differ by under 1.3 ms in each operation.
- At 1000 devices, mls-rs is faster in every operation but making a KeyPackage: 0.080–0.097 ms against OpenMLS's 0.047 ms. On one core, it processes Commits, catches up, encrypts and decrypts in 39–60% of OpenMLS's time, and commits and joins in 64–90%. On every core, it creates a group in 57 ms against 349 ms and joins from a Welcome in 8.7 ms against 37.9 ms.
- The browser's GroupInfo check in WASM at 1000 devices: 71.9 ms against 77.9 ms.
- A member's saved state at 1000 devices: 354 KB against 1.0 MB. Wire objects differ by under 2%.

### WASM size
mls-rs's modules are 31% smaller raw and 18–23% smaller with Brotli: the browser check 94.9 KiB against 115.4 KiB, the whole suite 336.2 KiB against 434.8 KiB ([WASM](#wasm)).

### Maintenance
| | OpenMLS | mls-rs |
|---|---|---|
| Releases, 2025-11-11 to 2026-11-11 ([OpenMLS](https://crates.io/crates/openmls/versions), [mls-rs](https://crates.io/crates/mls-rs/versions)) | 10, and 4 release candidates: 2 breaking (0.8.0, 0.9.0) and fixes to 0.7 and 0.8 next to 0.9 | 9: 6 breaking (0.52.0 to 0.57.0) |
| Maintainers | Phoenix R&D and CE Labs ([README](https://github.com/openmls/openmls/blob/v0.9.2/README.md#maintenance--support)); 3 crates.io owners and 2 teams ([crates.io](https://crates.io/crates/openmls)); 258 commits by 28 people in the 12 months, 151 by the top three ([commits](https://github.com/openmls/openmls/commits/main)) | AWS (`awslabs`); 3 crates.io owners ([crates.io](https://crates.io/crates/mls-rs)); 50 commits by 11 people in the 12 months, 30 by one ([commits](https://github.com/awslabs/mls-rs/commits/main)) |
| Security advisories | 7 in its repository from 2025-09 to 2026-10, 2 high, 4 medium, 1 low, each fixed in 0.9.2 ([advisories](https://github.com/openmls/openmls/security/advisories)); 2 of them in the GitHub Advisory Database ([query](https://api.github.com/advisories?ecosystem=rust&affects=openmls)) | None in its repository ([advisories](https://github.com/awslabs/mls-rs/security/advisories)) or the GitHub Advisory Database ([query](https://api.github.com/advisories?ecosystem=rust&affects=mls-rs)) |
| RustSec advisories ([database](https://github.com/rustsec/advisory-db/tree/main/crates)) | None | None |
| Minimum Rust version, `rust_version` in the crates.io API ([OpenMLS](https://crates.io/api/v1/crates/openmls/versions), [mls-rs](https://crates.io/api/v1/crates/mls-rs/versions)) | 1.91.0 | 1.82.0; the RustCrypto provider declares none ([API](https://crates.io/api/v1/crates/mls-rs-crypto-rustcrypto/versions)) |
| License | MIT ([`Cargo.toml`](https://github.com/openmls/openmls/blob/v0.9.2/openmls/Cargo.toml#L8)) | Apache-2.0 OR MIT ([`Cargo.toml`](https://github.com/awslabs/mls-rs/blob/0.57.0/mls-rs/Cargo.toml#L10), [RustCrypto provider](https://github.com/awslabs/mls-rs/blob/0.57.0/mls-rs-crypto-rustcrypto/Cargo.toml#L9)), as the product crates ([`Cargo.toml`](../../Cargo.toml)) |
| Dependencies, `cargo tree -e normal --prune mls-scenario` of each prototype | 125 crate versions natively, 135 in WASM. Two RustCrypto generations: the provider signs with ed25519-dalek 2 and sha2 0.10, and its `hpke-rs-rust-crypto` takes x25519-dalek 3 and sha2 0.11. rayon is required | 91 natively, 94 in WASM. One RustCrypto generation, that of [tech-stack.md](tech-stack.md): ed25519-dalek 3, sha2 0.11, getrandom 0.4 |
| `cargo deny check advisories licenses` with the product's [`deny.toml`](../../deny.toml) | Fails: `hpke-rs`, `hpke-rs-crypto` and `hpke-rs-rust-crypto` are MPL-2.0; RUSTSEC-2026-0387, constant-time defects of crypto-bigint 0.5.5, through p256 and p384 0.13, outside the profile's ciphersuite; RUSTSEC-2026-0173, proc-macro-error2 unmaintained, through `hpke-rs`'s libcrux-sha3 | Passes |

### Spec fit
How each library meets the requirements of [membership.md](../protocol/membership.md) that the prototypes exercised. "App": the device's or the Delivery Service's own code does it; a quoted name is a finding of [OpenMLS](#openmls) or [mls-rs](#mls-rs).

| Requirement | OpenMLS | mls-rs |
|---|---|---|
| [MLS profile](../protocol/membership.md#mls-profile): ciphersuite, `basic` credential, the device's signing key | Met; the key stays in the device's key store ("Signing key") | Met; the key is copied into each group's state ("Signing key in the state") |
| MLS profile: PrivateMessages and PublicMessages | Met by configuration ("Wire formats") | Met by default |
| MLS profile: the new epoch's GroupInfo with each Commit | Met by configuration, with external Commits too ("GroupInfo") | Met by configuration; the app makes it after an external Commit ("GroupInfo", "External Commit") |
| [Admitted devices](../protocol/membership.md#admitted-devices): no Add of a device that is not admitted | App, before the Add ("KeyPackage checks") | App, in the `IdentityProvider` of the committing client ("Credential check") |
| Admitted devices: see added devices before merging, remove those not admitted | App, on a staged Commit ("Added devices") | App, before saving the merged group ("Reading before merging") |
| [Creating a channel](../protocol/membership.md#creating-a-channel): a refused creator drops its group and joins the kept one | App deletes the group first ("Dropping a group") | Met: a Welcome replaces the group |
| [KeyPackages](../protocol/membership.md#keypackages): lifetime, batches, last resort | Met; last resort with a workaround ("Last resort") | Met with configuration and a feature ("KeyPackage lifetime", "Last resort") |
| KeyPackages: the checks before an Add | Signature, lifetime, ciphersuite; app: credential ("KeyPackage checks") | Signature, lifetime, ciphersuite; app: credential ("KeyPackage checks") |
| [Commit ordering](../protocol/membership.md#commit-ordering): the server checks epoch and sender without group secrets | Met: `PublicGroup`, given the tree ("Outside party") | Met with a feature: `ExternalGroup`, on a copy ("Outside party") |
| Commit ordering: refused senders drop their Commit and commit again | Met ("Dropping a Commit") | Met ("Dropping a Commit") |
| [Group changes](../protocol/membership.md#group-changes): revocations in `authenticated_data` read before merging | App, on a staged Commit ("Reading before merging") | App, before saving the merged group ("Reading before merging") |
| Group changes: a joiner checks the GroupInfo's signer and members; members check it as an Add | App; the signer only by a workaround ("GroupInfo signer") | App, with the signer from the library ("GroupInfo signer", "Processing an external Commit") |
| Group changes: a device checks a Welcome's signer and members | App, before `into_group` ("Checking a Welcome") | App, before saving the group ("Checking a Welcome") |
| [Rejoining](../protocol/membership.md#rejoining): an external Commit that removes the old leaf | Met; app refuses one that removes another leaf ("External Commit", "Removes in an external Commit") | Met with workarounds ("External Commit"); refuses one that removes another leaf |
| Rejoining: process missed Commits in order | Met ("Missed Commits") | Met ("Missed Commits") |
| [History key](../protocol/membership.md#history-key): the exporter, an empty Commit, the removed device | Met ("Removed device") | Met; app acts on the removal ("Removed device") |
| [Wraps](../protocol/membership.md#wraps): the browser's GroupInfo check | Met, with the signer by a workaround ("GroupInfo signer", "Browser check") | Met ("Browser check") |

## Choice
**mls-rs 0.57 with `mls-rs-crypto-rustcrypto` 0.23.**
- It refuses an external Commit that removes another device's leaf (RFC 9420, §12.2). OpenMLS leaves that check to the application, and a missed check lets any holder of the latest GroupInfo remove any member. mls-rs also gives a GroupInfo's signer, which OpenMLS does not.
- Its dependencies pass the product's `cargo deny` policy and use the RustCrypto generation of the product crates. OpenMLS's RustCrypto provider brings MPL-2.0 crates, a vulnerability, an unmaintained crate and a second RustCrypto generation.
- At 1000 devices it is faster in all but one operation; its saved state is a third of OpenMLS's; its browser module is 18% smaller compressed.
- Both libraries interoperate. On audit, mls-rs states it has none, and none was found for OpenMLS.
- Accepted against it: a smaller team, a breaking release every two months, fewer official test vectors in its CI, the signing key in each group's state, and Commits merged before the application's checks ([For the crypto core](#for-the-crypto-core)).

## For the crypto core
The crypto core takes these workarounds from the mls-rs prototype ([mls-rs](#mls-rs)):
- **Features:** `external_client` for the Delivery Service and the browser, `last_resort_key_package_ext`; `browser` of `mls-rs-crypto-rustcrypto` in WASM.
- **Storage:** one store implementing `GroupStateStorage` and `KeyPackageStorage` over the local state, with a direct `mls-rs-core` dependency; it also drops a group ("Saving state", "Dropping a group").
- **Signing key:** the device's Ed25519 key goes into `SignatureSecretKey` as the 64 bytes of the seed and the public key ("Signing key format").
- **Sender:** the device reads the roster before processing, to take the credentials of the sender and the removed members from their leaf indexes ("Sender"). An external Commit's joiner comes from the new epoch's roster ("Processing an external Commit").
- **Checks before saving:** a processed Commit's sender, Adds, Removes and `authenticated_data`, and a Welcome's signer and members, are checked before `write_to_storage()`; a failed check drops the group unsaved ("Reading before merging", "Checking a Welcome").
- **Admission:** only the device that commits an Add checks its KeyPackage against the admitted devices, before the Add; members remove such devices after processing ("Credential check").
- **KeyPackages:** `key_package_lifetime` of 60 days from an hour back ("KeyPackage lifetime").
- **GroupInfo:** `CommitOptions::with_allow_external_commit`; after an external Commit, `group_info_message_allowing_ext_commit(true)` ("GroupInfo", "External Commit").
- **Rejoining:** `with_removal` of the old leaf, found by its signature key and accepted by `valid_successor`; the device drops its old state of the group before the build ("External Commit").
- **Removed device:** acts on `CommitEffect::Removed` ("Removed device").
- **Delivery Service:** processes a copy of its `ExternalGroup` and keeps it only if the sender check passes ("Outside party").
- **Client:** built per step by a function returning `Client<impl MlsConfig>` ("Client").

The device's signing key, which mls-rs copies into each group's state ("Signing key in the state"), reaches the local state only sealed ([architecture.md](architecture.md#cryptography)).

Of the [membership.md](../protocol/membership.md) requirements the prototypes exercised, mls-rs meets each, with the workarounds above. The gaps in [Spec](#spec) are the spec's own.
